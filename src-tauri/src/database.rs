use rusqlite::{Connection, Result};

use base64::Engine;

pub use crate::infrastructure::encryption::{self, ENCRYPT_PREFIX};

pub use crate::domain::models::ClipboardEntry;
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
use crate::infrastructure::repository::settings_repo::SqliteSettingsRepository;
use crate::infrastructure::repository::tag_repo::SqliteTagRepository;
use std::sync::{Arc, Mutex};

// Acquired before session/database locks. Writers hold a read guard until
// their file reference is committed. Cleanup only runs with a write guard.
pub(crate) static ATTACHMENT_WRITERS: std::sync::RwLock<()> = std::sync::RwLock::new(());

pub struct DbState {
    /// Lock order: take SessionHistory or AppDataDir first, then this.
    /// Never wait for another lock while holding the connection (the repo
    /// methods lock it too).
    pub conn: Arc<Mutex<Connection>>,
    pub repo: SqliteClipboardRepository,
    pub settings_repo: SqliteSettingsRepository,
    pub tag_repo: SqliteTagRepository,
}

pub const TRANSLATE_PROVIDER: &str = "app.translate_provider";
pub const TRANSLATE_DEEPL_KEY: &str = "app.translate_deepl_key";
pub const TRANSLATE_OPENAI_BASE_URL: &str = "app.translate_openai_base_url";
pub const TRANSLATE_OPENAI_MODEL: &str = "app.translate_openai_model";
pub const TRANSLATE_OPENAI_KEY: &str = "app.translate_openai_key";

const SENSITIVE_KEYS: &[&str] = &[
    "mqtt_password",
    "mqtt_username",
    "ai_profiles",
    "cloud_sync_api_key",
    "cloud_sync_webdav_password",
    "e2e_key",
    TRANSLATE_DEEPL_KEY,
    TRANSLATE_OPENAI_KEY,
];

pub const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];

pub fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key))
}

pub fn is_translation_secret(key: &str) -> bool {
    [TRANSLATE_DEEPL_KEY, TRANSLATE_OPENAI_KEY]
        .iter()
        .any(|setting| setting.eq_ignore_ascii_case(key))
}

pub fn is_translation_setting(key: &str) -> bool {
    is_translation_secret(key)
        || [TRANSLATE_PROVIDER, TRANSLATE_OPENAI_BASE_URL, TRANSLATE_OPENAI_MODEL]
            .iter()
            .any(|setting| setting.eq_ignore_ascii_case(key))
}

pub fn has_sensitive_tag(tags: &[String]) -> bool {
    tags.iter()
        .any(|t| SENSITIVE_TAGS.iter().any(|s| s.eq_ignore_ascii_case(t)))
}

pub fn is_text_type(content_type: &str) -> bool {
    matches!(content_type, "text" | "code" | "url" | "rich_text")
}

fn normalize_text(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn calc_text_hash(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let normalized = normalize_text(content);
    let mut hasher = crate::infrastructure::stable_hash::StableHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// Hash of raw bytes that stays the same across Rust releases (stored hashes
/// and attachment names depend on it; see infrastructure::stable_hash).
pub fn stable_bytes_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = crate::infrastructure::stable_hash::StableHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// File extension from an image's magic bytes.
pub fn image_extension_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

/// Content-addressed attachment (`img_<hash>.<ext>` in `dir`). Written to a
/// temp file and renamed into place; an existing file of the wrong size (a
/// write cut short by a crash or a full disk) is rewritten instead of being
/// reused for good.
pub fn write_attachment(dir: &std::path::Path, bytes: &[u8], ext: &str) -> Option<std::path::PathBuf> {
    write_attachment_file(dir, bytes, ext, stable_bytes_hash(bytes), None)
}

static PENDING_ATTACHMENTS: std::sync::LazyLock<
    Mutex<std::collections::HashMap<std::path::PathBuf, usize>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

#[derive(Default)]
pub(crate) struct PendingAttachments(Vec<std::path::PathBuf>);

impl PendingAttachments {
    pub fn protect(&mut self, path: &std::path::Path) {
        *PENDING_ATTACHMENTS
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_default() += 1;
        self.0.push(path.to_path_buf());
    }

    pub fn write(
        &mut self,
        dir: &std::path::Path,
        bytes: &[u8],
        ext: &str,
        hash: u64,
    ) -> Option<std::path::PathBuf> {
        write_attachment_file(dir, bytes, ext, hash, Some(self))
    }
}

impl Drop for PendingAttachments {
    fn drop(&mut self) {
        let mut pending = PENDING_ATTACHMENTS.lock().unwrap();
        for path in &self.0 {
            let count = pending.get_mut(path).expect("registered attachment");
            *count -= 1;
            if *count == 0 {
                pending.remove(path);
            }
        }
    }
}

/// A capture is still writing or committing this file: never delete it yet.
pub(crate) fn is_attachment_pending(path: &std::path::Path) -> bool {
    PENDING_ATTACHMENTS.lock().unwrap().contains_key(path)
}

pub(crate) fn remove_idle_attachment(path: &std::path::Path) {
    let pending = PENDING_ATTACHMENTS.lock().unwrap();
    if !pending.contains_key(path) {
        crate::app::commands::thumbnail_cmd::remove_cached_thumbnails(path);
        let _ = std::fs::remove_file(path);
    }
}

fn write_attachment_file(
    dir: &std::path::Path,
    bytes: &[u8],
    ext: &str,
    hash: u64,
    pending: Option<&mut PendingAttachments>,
) -> Option<std::path::PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let name = format!("img_{hash:x}.{ext}");
    let path = dir.join(&name);
    if let Some(pending) = pending {
        pending.protect(&path);
    }
    let intact = std::fs::metadata(&path)
        .map(|meta| meta.len() == bytes.len() as u64)
        .unwrap_or(false);
    if !intact {
        let temp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        if std::fs::write(&temp, bytes)
            .and_then(|()| std::fs::rename(&temp, &path))
            .is_err()
        {
            let _ = std::fs::remove_file(&temp);
            return None;
        }
    }
    Some(path)
}

pub fn calc_image_hash_from_bytes(bytes: &[u8]) -> Option<i64> {
    Some(stable_bytes_hash(bytes) as i64)
}

pub fn calc_image_hash(base64_data: &str) -> Option<i64> {
    let trimmed = base64_data.trim();
    if !trimmed.starts_with("data:") && (trimmed.starts_with('/') || trimmed.contains(":\\")) {
        return hash_image_file(std::path::Path::new(trimmed));
    }
    let parts: Vec<&str> = trimmed.splitn(2, ',').collect();
    let payload = if parts.len() == 2 { parts[1] } else { trimmed };
    let payload_clean = payload.replace("\r", "").replace("\n", "");
    if payload_clean.trim().is_empty() {
        return None;
    }

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload_clean.trim())
        .ok()?;

    calc_image_hash_from_bytes(&bytes)
}

fn hash_image_file(path: &std::path::Path) -> Option<i64> {
    use std::hash::Hasher;
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let size = usize::try_from(metadata.len()).ok()?;
    let mut hasher = crate::infrastructure::stable_hash::StableHasher::new();
    hasher.write_usize(size); // Same length prefix as hashing a byte slice.
    let mut buf = [0; 64 * 1024];
    let mut read = 0;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        read += n;
        hasher.write(&buf[..n]);
    }
    (read == size).then(|| hasher.finish() as i64)
}

pub fn init_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // Performance and space pragmas
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA auto_vacuum = FULL;
        PRAGMA busy_timeout = 5000;
        PRAGMA mmap_size = 268435456;
        PRAGMA temp_store = MEMORY;
        PRAGMA cache_size = -64000;
    ",
    )?;

    // Run migrations
    crate::infrastructure::repository::migrations::run_migrations(&conn)?;

    // Initialize default settings
    seed_defaults(&conn)?;

    Ok(conn)
}

// save_entry removed (migrated to repository)

pub(crate) fn attachment_extension(image_bytes: &[u8], extension: &str) -> &'static str {
    // The bytes decide the extension: a GIF from a data URL labelled PNG, or a
    // BMP, used to be saved as .png.
    image_extension_from_magic(image_bytes).unwrap_or(
        match extension.trim().to_ascii_lowercase().as_str() {
            "gif" => "gif",
            "jpg" | "jpeg" => "jpg",
            "webp" => "webp",
            "bmp" => "bmp",
            _ => "png",
        },
    )
}

pub fn save_image_bytes_to_file_with_extension(
    image_bytes: &[u8],
    data_dir: &std::path::Path,
    extension: &str,
) -> Option<String> {
    let extension = attachment_extension(image_bytes, extension);
    write_attachment(&data_dir.join("attachments"), image_bytes, extension)
        .map(|path| path.to_string_lossy().to_string())
}

pub fn save_image_bytes_to_file(png_bytes: &[u8], data_dir: &std::path::Path) -> Option<String> {
    save_image_bytes_to_file_with_extension(png_bytes, data_dir, "png")
}

pub fn save_image_to_file(data_url: &str, data_dir: &std::path::Path) -> Option<String> {
    let parts: Vec<&str> = data_url.splitn(2, ',').collect();
    if parts.len() < 2 {
        return None;
    }

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(parts[1])
        .ok()?;

    save_image_bytes_to_file(&decoded, data_dir)
}

/// Where a stored file path of an entry points now. Stored paths are
/// absolute, so attachments outlive a move of the data folder (an older app
/// folder renamed on upgrade, a restored backup) only under their name in
/// this data folder's attachments. `None` when the file is in neither place.
pub fn resolve_entry_file_path(
    raw_path: &str,
    data_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = std::path::Path::new(raw_path);
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let candidate = data_dir.join("attachments").join(path.file_name()?);
    candidate.is_file().then_some(candidate)
}

// get_history removed (migrated to repository)

// search_history removed (migrated to repository)

// get_important_items removed (migrated to repository)

// delete_entry_db, delete_entry, enforce_storage_limit, clear_history removed (migrated to repository)

/// Set on a fresh database; startup turns on launch at login once, then
/// removes it (app/setup.rs). Existing installs keep their login item as is.
pub const AUTOSTART_PENDING_KEY: &str = "app.autostart_pending";

/// Defaults for every Copy & Paste setting. `INSERT OR IGNORE`: saved values win.
pub const DEFAULT_SETTINGS: &[(&str, &str)] = &[
    ("app.hotkey", "Shift+Z"),
    ("app.silent_start", "false"),
    ("app.window_position_mode", "follow_mouse"),
    ("app.hide_tray_icon", "false"),
    ("app.hide_dock_icon", "false"),
    ("app.show_search_box", "true"),
    ("app.show_type_tabs", "true"),
    ("app.show_source_app_icon", "true"),
    ("app.display_name", "Copy & Paste"),
    ("app.arrow_key_selection", "true"),
    ("app.glass_level", "35"),
    ("app.persistent", "true"),
    ("app.persistent_limit_enabled", "false"),
    ("app.persistent_limit", "1000"),
    ("app.deduplicate", "true"),
    ("app.delete_after_paste", "false"),
    ("app.move_to_top_after_paste", "true"),
    ("app.sequential_mode", "false"),
    ("app.sequential_hotkey", "Alt+V"),
    ("app.cleanup_rules", ""),
    ("app.app_cleanup_policies", "[]"),
    ("app.window_pinned", "false"),
];

pub fn seed_defaults(conn: &Connection) -> Result<()> {
    // Only a new (or reset) database lacks the hotkey.
    let fresh: bool = conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM settings WHERE key = 'app.hotkey')",
        [],
        |r| r.get(0),
    )?;
    if fresh {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, 'true')",
            [AUTOSTART_PENDING_KEY],
        )?;
    }
    for (key, value) in DEFAULT_SETTINGS {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            [*key, *value],
        )?;
    }
    Ok(())
}

// Migrated to repositories: toggle_pin, update_pinned_order, get_entry_by_content,
// update_entry_content, insert_entry, get_entry_content, get_entry_content_full,
// get_entry_content_with_html, get_entry_by_id, update_entry_tags, get_all_tags,
// create_tag, rename_tag, delete_tag_globally, get_entries_by_tag, set_tag_color, get_tag_colors

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::repository::clipboard_repo::{
        ClipboardRepository, ClipboardSortMode, SqliteClipboardRepository,
    };
    use crate::infrastructure::repository::settings_repo::{
        SettingsRepository, SqliteSettingsRepository,
    };

    // 辅助函数：创建一个内存中的临时测试数据库
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // 调用你的 init_db 逻辑（手动执行部分关键建表语句）
        conn.execute(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                html_content TEXT,
                source_app TEXT NOT NULL,
                source_app_path TEXT,
                timestamp INTEGER NOT NULL,
                preview TEXT NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                content_hash INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL DEFAULT '[]',
                note TEXT NOT NULL DEFAULT '',
                use_count INTEGER NOT NULL DEFAULT 0,
                item_hotkey TEXT NOT NULL DEFAULT '',
                item_hotkey_global INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0,
                sort_at INTEGER NOT NULL DEFAULT 0,
                move_to_group_hotkey TEXT NOT NULL DEFAULT '',
                move_to_group_hotkey_global INTEGER NOT NULL DEFAULT 0,
                is_external INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
            )",
            [],
        )
        .unwrap();
        conn
    }

        #[test]
    fn test_save_and_get_history() {
        let conn = setup_test_db();

        let entry = ClipboardEntry {
            id: 0,
            content_type: "text".to_string(),
            content: "Hello Integration Test".to_string(),
            html_content: None,
            source_app: "TestApp".to_string(),
            source_app_path: Some("/Applications/TestApp.app".to_string()),
            timestamp: 123456789,
            created_at: 123456789,
            last_used_at: 0,
            sort_at: 123456789,
            preview: "Hello...".to_string(),
            is_pinned: false,
            tags: vec![],
            note: String::new(),
            ocr_text: None,
            ocr_match: false,
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            file_preview_exists: true,
        };

        let conn_arc = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(conn_arc);

        // 1. 测试保存
        let id = repo.save(&entry, None).expect("保存失败");
        assert!(id > 0);

        // 2. 测试获取
        let history = repo.get_history(10, 0, None).expect("获取历史失败");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Hello Integration Test");
        assert_eq!(history[0].source_app, "TestApp");
        assert_eq!(history[0].created_at, 123456789);
        assert_eq!(history[0].sort_at, 123456789);
        assert_eq!(history[0].last_used_at, 0);

        repo.increment_use_count(id).expect("更新使用时间失败");
        let used = repo.get_history(10, 0, None).expect("重新获取历史失败");
        assert!(used[0].last_used_at > 0);
        assert_eq!(used[0].created_at, 123456789);
    }

    #[test]
    fn test_history_sort_modes_keep_pinned_items_first() {
        let conn = setup_test_db();
        for (content, is_pinned, created_at, last_used_at, sort_at, use_count, pinned_order) in [
            ("pinned-activity", 1, 100, 20, 50, 1, 1),
            ("pinned-manual", 1, 200, 10, 40, 5, 2),
            ("normal-used", 0, 300, 30, 60, 10, 99),
            ("normal-created", 0, 400, 0, 70, 0, 0),
        ] {
            conn.execute(
                "INSERT INTO clipboard_history (
                    content_type, content, source_app, timestamp, preview, is_pinned,
                    created_at, last_used_at, sort_at, use_count, pinned_order
                 ) VALUES ('text', ?1, 'TestApp', ?2, ?1, ?3, ?4, ?5, ?2, ?6, ?7)",
                rusqlite::params![
                    content,
                    sort_at,
                    is_pinned,
                    created_at,
                    last_used_at,
                    use_count,
                    pinned_order
                ],
            )
            .unwrap();
        }

        let repo = SqliteClipboardRepository::new(Arc::new(Mutex::new(conn)));
        let contents_for = |mode| {
            repo.get_history_sorted(10, 0, None, mode)
                .unwrap()
                .into_iter()
                .map(|entry| entry.content)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            contents_for(ClipboardSortMode::Activity),
            [
                "pinned-manual",
                "pinned-activity",
                "normal-created",
                "normal-used"
            ]
        );
        assert_eq!(
            contents_for(ClipboardSortMode::Created),
            [
                "pinned-manual",
                "pinned-activity",
                "normal-created",
                "normal-used"
            ]
        );
        assert_eq!(
            contents_for(ClipboardSortMode::LastUsed),
            [
                "pinned-manual",
                "pinned-activity",
                "normal-used",
                "normal-created"
            ]
        );
        assert_eq!(
            contents_for(ClipboardSortMode::Usage),
            [
                "pinned-manual",
                "pinned-activity",
                "normal-used",
                "normal-created"
            ]
        );
        assert_eq!(
            contents_for(ClipboardSortMode::Manual),
            [
                "pinned-manual",
                "pinned-activity",
                "normal-created",
                "normal-used"
            ]
        );
        // The plain text paste hotkey: newest copy first, pins have no say.
        assert_eq!(
            contents_for(ClipboardSortMode::Recent),
            [
                "normal-created",
                "normal-used",
                "pinned-activity",
                "pinned-manual"
            ]
        );
    }

    #[test]
    fn test_settings_persistence() {
        let conn = setup_test_db();
        let conn_arc = Arc::new(Mutex::new(conn));
        let repo = SqliteSettingsRepository::new(conn_arc);

        // 测试设置保存
        repo.set("test_key", "test_value").unwrap();

        // 测试设置读取
        let val = repo.get("test_key").unwrap();
        assert_eq!(val, Some("test_value".to_string()));
    }

    #[test]
    fn entry_files_are_found_after_the_data_folder_moved() {
        let root = std::env::temp_dir().join(format!("cp-entry-file-{}", uuid::Uuid::new_v4()));
        let data_dir = root.join("com.copypaste");
        std::fs::create_dir_all(data_dir.join("attachments")).unwrap();
        let moved = data_dir.join("attachments").join("img_1.png");
        std::fs::write(&moved, b"png").unwrap();
        let outside = root.join("photo.png");
        std::fs::write(&outside, b"png").unwrap();

        // Still where it was saved.
        let stored = outside.to_string_lossy().to_string();
        assert_eq!(resolve_entry_file_path(&format!(" {stored} "), &data_dir), Some(outside));
        // Saved under the old data folder, now in this one.
        let old_path = root.join("com.old.copypaste").join("attachments").join("img_1.png");
        assert_eq!(resolve_entry_file_path(&old_path.to_string_lossy(), &data_dir), Some(moved));
        // Gone from both.
        let gone = root.join("com.old.copypaste").join("attachments").join("img_2.png");
        assert_eq!(resolve_entry_file_path(&gone.to_string_lossy(), &data_dir), None);
        assert_eq!(resolve_entry_file_path("  ", &data_dir), None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleanup_keeps_an_attachment_until_all_pending_writes_finish() {
        let root = std::env::temp_dir().join(format!("cp-pending-{}", uuid::Uuid::new_v4()));
        let mut image_bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut image_bytes, image::ImageFormat::Png)
            .unwrap();
        let image_bytes = image_bytes.into_inner();
        let hash = super::stable_bytes_hash(&image_bytes);
        let mut first = super::PendingAttachments::default();
        let mut second = super::PendingAttachments::default();
        let path = first.write(&root, &image_bytes, "png", hash).unwrap();
        assert_eq!(
            second.write(&root, &image_bytes, "png", hash).unwrap(),
            path
        );
        assert_eq!(
            super::calc_image_hash(path.to_str().unwrap()),
            Some(hash as i64)
        );
        drop(first);
        super::remove_idle_attachment(&path);
        assert!(path.is_file());
        drop(second);
        super::remove_idle_attachment(&path);
        assert!(!path.exists());
        std::fs::remove_dir(root).unwrap();
    }
}
