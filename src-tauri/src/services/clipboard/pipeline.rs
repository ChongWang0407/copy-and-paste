use crate::app_state::{AppDataDir, PasteQueue, SessionHistory, SettingsState};
use crate::database::{
    calc_image_hash_from_bytes, is_text_type, save_image_bytes_to_file_with_extension, DbState,
};
use crate::domain::models::ClipboardEntry;
#[cfg(target_os = "macos")]
use crate::infrastructure::macos_api::window::get_active_app_snapshot;
use crate::services::clipboard::utils::*;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone)]
pub enum ClipboardData {
    Text(String),
    RichText { text: String, html: String },
    Image { bytes: Vec<u8>, extension: String },
    Files(Vec<String>),
}

pub struct PipelineContext {
    pub data: ClipboardData,
    pub app_handle: AppHandle,
    pub source_app: String,
    pub source_app_path: Option<String>,
    pub timestamp: i64,
    pub entry: Option<ClipboardEntry>,
    pub should_stop: bool,
    pub pending_removals: Vec<i64>,
    pub reuse_session_id: Option<i64>,
    pub encoded_image: Option<(Vec<u8>, String)>,
    pub image_content_hash: Option<i64>,
}

impl PipelineContext {
    pub fn new(app_handle: AppHandle, data: ClipboardData) -> Self {
        #[cfg(target_os = "macos")]
        let active_app = get_active_app_snapshot();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let active_app = ("Unknown".to_string(), None);

        #[cfg(target_os = "macos")]
        let (source_app, source_app_path) = (active_app.app_name, active_app.process_path);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let (source_app, source_app_path) = active_app;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        Self {
            data,
            app_handle,
            source_app,
            source_app_path,
            timestamp,
            entry: None,
            should_stop: false,
            pending_removals: Vec::new(),
            reuse_session_id: None,
            encoded_image: None,
            image_content_hash: None,
        }
    }
}

pub trait PipelineStage {
    fn process(&self, context: &mut PipelineContext);
}

pub struct ClipboardPipeline {
    stages: Vec<Box<dyn PipelineStage + Send + Sync>>,
}

impl ClipboardPipeline {
    pub fn new() -> Self {
        Self {
            stages: vec![
                Box::new(DiscoveryStage),
                Box::new(TransformationStage),
                Box::new(SequentialEchoStage),
                Box::new(DeduplicationStage),
                Box::new(PersistenceStage),
                Box::new(DistributionStage),
            ],
        }
    }

    pub fn execute(&self, context: &mut PipelineContext) {
        for stage in &self.stages {
            stage.process(context);
            if context.should_stop {
                break;
            }
        }
    }
}

// Stage 1: Discovery
pub struct DiscoveryStage;
impl PipelineStage for DiscoveryStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let (content_type, content, html_content) = match &ctx.data {
            ClipboardData::Text(t) => (detect_content_type(t), t.clone(), None),
            ClipboardData::RichText { text, html } => {
                let line_plain = normalize_clipboard_line_endings(text);
                let content = if looks_like_cf_html_header_text(&line_plain)
                    || (line_plain.is_empty() && !html.trim().is_empty())
                    || looks_like_html_fragment(&line_plain)
                {
                    derive_rich_text_content(text, Some(html))
                } else {
                    line_plain
                };
                ("rich_text".to_string(), content, Some(html.clone()))
            }
            ClipboardData::Image { bytes, extension } => {
                ctx.encoded_image = Some((bytes.clone(), extension.clone()));
                ("image".to_string(), String::new(), None)
            }
            ClipboardData::Files(f) => {
                let content = f.join("\n");
                if f.len() == 1 {
                    let path = &f[0];
                    let lower = path.to_lowercase();
                    if lower.ends_with(".gif") {
                        ("image".to_string(), path.clone(), None)
                    } else if lower.ends_with(".png")
                        || lower.ends_with(".jpg")
                        || lower.ends_with(".jpeg")
                        || lower.ends_with(".bmp")
                        || lower.ends_with(".webp")
                    {
                        // Preserve the original path. Decoding and re-encoding the
                        // entire image on the clipboard watcher thread made large
                        // copied files freeze capture and multiply memory usage.
                        ("image".to_string(), path.clone(), None)
                    } else if lower.ends_with(".mp4")
                        || lower.ends_with(".mkv")
                        || lower.ends_with(".avi")
                        || lower.ends_with(".mov")
                        || lower.ends_with(".wmv")
                        || lower.ends_with(".flv")
                        || lower.ends_with(".webm")
                    {
                        ("video".to_string(), path.clone(), None)
                    } else {
                        ("file".to_string(), content, None)
                    }
                } else {
                    ("file".to_string(), content, None)
                }
            }
        };

        let preview = build_entry_preview(&content_type, &content, html_content.as_deref());

        let is_external =
            (content_type == "file" || content_type == "video" || content_type == "image")
                && !content.starts_with("data:");

        ctx.entry = Some(ClipboardEntry {
            id: 0,
            content_type,
            content,
            html_content,
            source_app: ctx.source_app.clone(),
            source_app_path: ctx.source_app_path.clone(),
            timestamp: ctx.timestamp,
            created_at: ctx.timestamp,
            last_used_at: 0,
            sort_at: ctx.timestamp,
            preview,
            is_pinned: false,
            tags: Vec::new(),
            note: String::new(),
            ocr_text: None,
            ocr_match: false,
            use_count: 0,
            is_external,
            pinned_order: 0,
            file_preview_exists: true,
        });
    }
}

// Stage 2: Transformation
pub struct TransformationStage;
impl PipelineStage for TransformationStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_mut().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();

        // Normalize line endings only; preserve leading/trailing whitespace (#97).
        entry.content = entry.content.replace("\r\n", "\n");

        // App policies first: an image from an app set to "ignore" must not be
        // written to disk at all (it used to be saved, then dropped, and left).
        let app_cleanup_policies_raw = settings.app_cleanup_policies.lock().unwrap().clone();
        if !app_cleanup_policies_raw.trim().is_empty() {
            let app_cleanup_policies = parse_app_cleanup_policies(&app_cleanup_policies_raw);
            for policy in app_cleanup_policies {
                if !app_cleanup_policy_matches(
                    &policy,
                    &ctx.source_app,
                    ctx.source_app_path.as_deref(),
                    &entry.content_type,
                ) {
                    continue;
                }

                if policy.action.eq_ignore_ascii_case("ignore") {
                    ctx.should_stop = true;
                    return;
                }

                if is_text_type(&entry.content_type) && !policy.cleanup_rules.trim().is_empty() {
                    let rules = parse_cleanup_rules(&policy.cleanup_rules);
                    if !rules.is_empty() {
                        let Some(cleaned) = apply_cleanup_rules(&entry.content, &rules) else {
                            ctx.should_stop = true;
                            return;
                        };
                        if cleaned.trim().is_empty() || cleaned == "__IGNORE_CAPTURE__" {
                            ctx.should_stop = true;
                            return;
                        }
                        entry.content = cleaned.replace("\r\n", "\n");
                        entry.preview =
                            build_entry_preview(&entry.content_type, &entry.content, None);
                    }
                }
            }
        }

        // Session-only mode keeps images out of the permanent attachments folder:
        // they live under session/ for this run only (see session_storage_root).
        let persistent = settings.persistent.load(Ordering::Relaxed);
        let storage_root = {
            let data_dir = ctx.app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
            if persistent { data_dir } else { session_storage_root(&data_dir) }
        };

        if let Some((image_bytes, extension)) = ctx.encoded_image.take() {
            ctx.image_content_hash = calc_image_hash_from_bytes(&image_bytes);
            let data_dir = storage_root.clone();
            if let Some(path) =
                save_image_bytes_to_file_with_extension(&image_bytes, &data_dir, &extension)
            {
                entry.content = path;
                entry.is_external = true;
            }
        } else if entry.content_type == "image" && entry.is_external && !entry.content.starts_with("data:") {
            // If the image was captured as an external file (e.g. from WeChat, QQ, Snipping Tool, or browser temp folder),
            // copy its bytes into the app's permanent attachments folder so it doesn't turn empty when the external temp file is cleaned up!
            let src_path = std::path::Path::new(&entry.content);
            let data_dir = storage_root.clone();
            let attachments_dir = data_dir.join("attachments");

            if !src_path.starts_with(&attachments_dir) && src_path.exists() {
                if let Ok(metadata) = std::fs::metadata(src_path) {
                    if metadata.len() > 0 && metadata.len() <= 64 * 1024 * 1024 {
                        if let Ok(image_bytes) = std::fs::read(src_path) {
                            let extension = src_path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("png");
                            if let Some(path) = save_image_bytes_to_file_with_extension(
                                &image_bytes,
                                &data_dir,
                                extension,
                            ) {
                                entry.content = path;
                                entry.is_external = true;
                                ctx.image_content_hash = calc_image_hash_from_bytes(&image_bytes);
                            }
                        }
                    }
                }
            }
        }

        if is_text_type(&entry.content_type) {
            let cleanup_rules_raw = settings.cleanup_rules.lock().unwrap().clone();
            if !cleanup_rules_raw.trim().is_empty() {
                let cleanup_rules = parse_cleanup_rules(&cleanup_rules_raw);
                if !cleanup_rules.is_empty() {
                    let Some(cleaned) = apply_cleanup_rules(&entry.content, &cleanup_rules) else {
                        ctx.should_stop = true;
                        return;
                    };
                    if cleaned.trim().is_empty() || cleaned == "__IGNORE_CAPTURE__" {
                        ctx.should_stop = true;
                        return;
                    }
                    entry.content = cleaned.replace("\r\n", "\n");
                    entry.preview = build_entry_preview(&entry.content_type, &entry.content, None);
                }
            }
        }

        // Rich Text Image Processing
        if let Some(html) = &entry.html_content {
            let app_data_dir = ctx.app_handle.state::<AppDataDir>();
            let data_dir = app_data_dir.0.lock().unwrap().clone();

            entry.html_content = if settings.persistent.load(Ordering::Relaxed) {
                let html_with_local_assets = process_local_images_in_html(html, &data_dir);
                Some(externalize_rich_image_fallback(
                    &html_with_local_assets,
                    &data_dir,
                ))
            } else {
                Some(embed_local_images(html))
            };
        }
    }
}

// Stage 3: Sequential paste echo (fast — must run before copy sound)
pub struct SequentialEchoStage;
impl PipelineStage for SequentialEchoStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let settings = ctx.app_handle.state::<SettingsState>();
        if !settings.sequential_mode.load(Ordering::Relaxed) {
            return;
        }

        let entry = ctx.entry.as_ref().unwrap();
        let queue_state = ctx.app_handle.state::<PasteQueue>();
        let queue = queue_state.0.lock().unwrap();
        if queue.last_action_was_paste
            && queue.last_pasted_content.as_deref() == Some(&entry.content)
        {
            println!("Ignoring echo paste from queue");
            ctx.should_stop = true;
        }
    }
}

fn duplicate_content_matches(content: &str, kind: &str, html: Option<&str>, old: &str, old_kind: &str, old_html: Option<&str>) -> bool {
    let compatible = kind == old_kind || (is_text_type(kind) && is_text_type(old_kind));
    if !compatible || normalize_clipboard_line_endings(content) != normalize_clipboard_line_endings(old) {
        return false;
    }
    if kind == "rich_text" && old_kind == "rich_text" {
        return match (html, old_html) {
            (None, None) => true,
            (Some(a), Some(b)) => normalize_clipboard_line_endings(a) == normalize_clipboard_line_endings(b),
            _ => false,
        };
    }
    true
}

fn preserve_rich_format(entry: &mut ClipboardEntry, existing: &ClipboardEntry) {
    if entry.content_type != "rich_text" && existing.content_type == "rich_text" {
        entry.content_type = existing.content_type.clone();
        entry.html_content = existing.html_content.clone();
    }
}

// Stage 5: Deduplication (DB-heavy — intentionally after copy sound)
pub struct DeduplicationStage;
impl PipelineStage for DeduplicationStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let settings = ctx.app_handle.state::<SettingsState>();

        // Deduplication
        if settings.deduplicate.load(Ordering::Relaxed) {
            let persistent_enabled = settings.persistent.load(Ordering::Relaxed);
            let db_state = ctx.app_handle.state::<DbState>();
            let conn = db_state.conn.lock().unwrap();

            let mut existing_id = None;
            let (content, content_type, html_content) = {
                let e = ctx.entry.as_ref().unwrap();
                (
                    e.content.clone(),
                    e.content_type.clone(),
                    e.html_content.clone(),
                )
            };

            // Try precise match and normalized match
            if is_text_type(&content_type) {
                if let Ok(ids) = db_state.repo.find_text_candidates_with_conn(&conn, &content) {
                    for id in ids {
                        if let Ok(Some((old_content, old_type, old_html))) = db_state.repo.get_entry_content_with_html_with_conn(&conn, id) {
                            if duplicate_content_matches(&content, &content_type, html_content.as_deref(), &old_content, &old_type, old_html.as_deref()) {
                                existing_id = Some(id);
                                break;
                            }
                        }
                    }
                }
            } else {
                existing_id = db_state.repo.find_by_content_with_conn(&conn, &content, Some(&content_type)).ok().flatten();
            }

            if persistent_enabled {
                if let Some(id) = existing_id {
                    // Instead of deleting, we set the entry ID so PersistenceStage performs an UPDATE
                    // This ensures the item is "moved to top" without risking data loss
                    let entry_mut = ctx.entry.as_mut().unwrap();
                    entry_mut.id = id;
                    if let Ok(Some(existing)) = db_state.repo.get_entry_by_id_with_conn(&conn, id) {
                        preserve_rich_format(entry_mut, &existing);
                        entry_mut.created_at = existing.created_at;
                        entry_mut.last_used_at = existing.last_used_at;
                        // The UPDATE leaves these alone, but the entry is also
                        // sent to the list as is; without them a pinned item
                        // copied again drops out of the pinned section (#152).
                        entry_mut.is_pinned = existing.is_pinned;
                        entry_mut.pinned_order = existing.pinned_order;
                        entry_mut.note = existing.note;
                        entry_mut.use_count = existing.use_count + 1;
                        if entry_mut.tags.is_empty() {
                            entry_mut.tags = existing.tags;
                        } else {
                            let mut merged = existing.tags.clone();
                            for tag in &entry_mut.tags {
                                if !merged.iter().any(|t| t == tag) {
                                    merged.push(tag.clone());
                                }
                            }
                            entry_mut.tags = merged;
                        }
                    }
                }
            }
            // Never wait for another lock while holding the connection:
            // get_clipboard_history holds the session lock while it queries.
            drop(conn);

            // Session history cleanup
            let session_history = ctx.app_handle.state::<SessionHistory>();
            let mut removed_ids = Vec::new();
            let mut reuse_session_id: Option<i64> = None;
            {
                let session = session_history.0.lock().unwrap();
                let entry = ctx.entry.as_ref().expect("entry exists");
                for item in session.iter() {
                    let match_found = duplicate_content_matches(&entry.content, &entry.content_type, entry.html_content.as_deref(), &item.content, &item.content_type, item.html_content.as_deref());
                    if match_found {
                        removed_ids.push(item.id);
                        if !persistent_enabled {
                            reuse_session_id = Some(item.id);
                        }
                    }
                }
            }
            if !persistent_enabled {
                if let Some(reuse_id) = reuse_session_id {
                    ctx.reuse_session_id = Some(reuse_id);
                    if let Some(entry_mut) = ctx.entry.as_mut() {
                        entry_mut.id = reuse_id;
                    }
                    removed_ids.retain(|id| *id != reuse_id);
                }
            }
            ctx.pending_removals.extend(removed_ids);
        }
    }
}

// Stage 6: Persistence
pub struct PersistenceStage;
impl PipelineStage for PersistenceStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_mut().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();
        let db_state = ctx.app_handle.state::<DbState>();

        if settings.persistent.load(Ordering::Relaxed) {
            let app_data_dir = ctx.app_handle.state::<AppDataDir>();
            let data_dir = app_data_dir.0.lock().unwrap().clone();
            let conn = db_state.conn.lock().unwrap();

            if let Ok(id) =
                db_state
                    .repo
                    .save_with_conn(&conn, entry, Some(&data_dir), ctx.image_content_hash)
            {
                entry.id = id;
                if let Ok(deleted_ids) = db_state
                    .repo
                    .enforce_limit_with_conn(&conn, Some(&data_dir))
                {
                    for rid in deleted_ids {
                        let _ = ctx.app_handle.emit("clipboard-removed", rid);
                    }
                }
            }
        } else {
            // Session-only items
            if let Some(reuse_id) = ctx.reuse_session_id {
                let session_history = ctx.app_handle.state::<SessionHistory>();
                let mut updated_entry: Option<ClipboardEntry> = None;
                {
                    let mut session = session_history.0.lock().unwrap();
                    if let Some(existing) = session.iter_mut().find(|i| i.id == reuse_id) {
                        let preserved_tags = existing.tags.clone();
                        let preserved_pinned = existing.is_pinned;
                        let preserved_pinned_order = existing.pinned_order;
                        let preserved_use_count = existing.use_count;
                        let preserved_created_at = existing.created_at;
                        let preserved_last_used_at = existing.last_used_at;

                        preserve_rich_format(entry, existing);
                        existing.content_type = entry.content_type.clone();
                        existing.content = entry.content.clone();
                        existing.html_content = entry.html_content.clone();
                        existing.source_app = entry.source_app.clone();
                        existing.source_app_path = entry.source_app_path.clone();
                        existing.timestamp = entry.timestamp;
                        existing.created_at = preserved_created_at;
                        existing.last_used_at = preserved_last_used_at;
                        existing.sort_at = entry.sort_at;
                        existing.preview = entry.preview.clone();
                        existing.is_external = entry.is_external;
                        existing.file_preview_exists = entry.file_preview_exists;
                        existing.is_pinned = preserved_pinned;
                        existing.pinned_order = preserved_pinned_order;
                        existing.tags = if entry.tags.is_empty() {
                            preserved_tags
                        } else {
                            entry.tags.clone()
                        };
                        existing.use_count = preserved_use_count + 1;

                        updated_entry = Some(existing.clone());
                    }
                }

                if let Some(updated) = updated_entry {
                    *entry = updated;
                    return;
                }
            }

            // Use a unique negative ID for new session-only items
            let id = -(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64
                / 1000);
            entry.id = id;
            let session_history = ctx.app_handle.state::<SessionHistory>();
            let mut session = session_history.0.lock().unwrap();
            session.push_back(entry.clone());
            if session.len() > 500 {
                if let Some(removed) = session.pop_front() {
                    let _ = ctx.app_handle.emit("clipboard-removed", removed.id);
                }
            }
        }
    }
}

// Stage 7: Distribution
pub struct DistributionStage;
impl PipelineStage for DistributionStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_ref().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();

        if entry.id == 0 && settings.persistent.load(Ordering::Relaxed) {
            return; // Failed to save
        }

        if !ctx.pending_removals.is_empty() {
            let mut pending = std::mem::take(&mut ctx.pending_removals);
            pending.retain(|id| *id != entry.id);
            if !pending.is_empty() {
                let unique: std::collections::HashSet<i64> = pending.into_iter().collect();
                {
                    let session_history = ctx.app_handle.state::<SessionHistory>();
                    let mut session = session_history.0.lock().unwrap();
                    session.retain(|item| !unique.contains(&item.id));
                }
                for rid in unique {
                    let _ = ctx.app_handle.emit("clipboard-removed", rid);
                }
            }
        }

        // Sequential Queue updates
        if settings.sequential_mode.load(Ordering::Relaxed) {
            let queue_state = ctx.app_handle.state::<PasteQueue>();
            let mut queue = queue_state.0.lock().unwrap();
            if queue.last_action_was_paste {
                queue.items.clear();
                queue.last_action_was_paste = false;
                queue.last_pasted_content = None;
            }
            queue.items.push_back(entry.id);
        }

        // Notify
        let _ = ctx
            .app_handle
            .emit("clipboard-updated", truncate_entry_for_ui(entry.clone()));
        crate::app::capsule::notify_copied(&ctx.app_handle, entry);

        if entry.content_type == "image" {
            crate::services::ocr::enqueue(&ctx.app_handle, entry.id, &entry.content);
        }

        if entry.is_external && entry.id != 0 {
            if let Some(path) =
                crate::services::content_handler::existing_file_path_from_content(&entry.content)
            {
                crate::services::content_handler::register_file_preview_watch(
                    ctx.app_handle.clone(),
                    entry.id,
                    path,
                    entry.content_type.clone(),
                    true,
                );
            }
        }
    }
}

/// Where session-only mode (persistence off) keeps images: `<data>/session`,
/// emptied at startup and as entries leave the session history.
pub fn session_storage_root(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("session")
}

/// Delete session-only image files no session entry shows any more. Entries
/// leave the session history in many ways (delete, clear, tag delete, the
/// 500-item cap, paste-and-delete); sweeping by reference catches all of them.
/// Run right after user deletes, and every 30 s by start_session_file_sweeper.
pub fn sweep_session_files(app: &tauri::AppHandle) {
    let data_dir = app.state::<AppDataDir>().0.lock().unwrap().clone();
    let attachments = session_storage_root(&data_dir).join("attachments");
    let Ok(files) = std::fs::read_dir(&attachments) else {
        return;
    };
    let referenced: Vec<String> = app
        .state::<crate::app_state::SessionHistory>()
        .0
        .lock()
        .unwrap()
        .iter()
        .flat_map(|entry| [entry.content.clone(), entry.html_content.clone().unwrap_or_default()])
        .collect();
    for file in files.flatten() {
        let path = file.path();
        let name = file.file_name().to_string_lossy().to_string();
        if !referenced.iter().any(|text| text.contains(&name)) {
            crate::app::commands::thumbnail_cmd::remove_cached_thumbnails(&path);
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Startup: the previous run's session images are unreachable now; delete
/// them, then keep sweeping while the app runs.
pub fn start_session_file_sweeper(app: tauri::AppHandle, data_dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(session_storage_root(data_dir));
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        sweep_session_files(&app);
    });
}

#[cfg(test)]
mod dedup_tests {
    use super::*;
    #[test]
    fn identical_text_merges_across_types_in_both_directions() {
        for a in ["text", "code", "url", "rich_text"] {
            for b in ["text", "code", "url", "rich_text"] {
                assert!(duplicate_content_matches("a\r\nb", a, Some("<b>a</b>"), "a\nb", b, Some("<b>a</b>")));
            }
        }
    }
    #[test]
    fn different_formats_and_meaningful_whitespace_stay_separate() {
        assert!(!duplicate_content_matches("a", "rich_text", Some("<b>a</b>"), "a", "rich_text", Some("<i>a</i>")));
        assert!(!duplicate_content_matches(" a", "text", None, "a", "text", None));
        assert!(!duplicate_content_matches("a", "file", None, "a", "text", None));
    }
    #[test]
    fn plain_recapture_preserves_existing_html() {
        let mut incoming = serde_json::from_value::<ClipboardEntry>(serde_json::json!({
            "id": 0, "content_type": "text", "content": "a", "source_app": "test", "timestamp": 1, "preview": "a", "is_pinned": false
        })).unwrap();
        incoming.content_type = "text".into();
        let mut existing = serde_json::from_value::<ClipboardEntry>(serde_json::json!({
            "id": 0, "content_type": "text", "content": "a", "source_app": "test", "timestamp": 1, "preview": "a", "is_pinned": false
        })).unwrap();
        existing.content_type = "rich_text".into();
        existing.html_content = Some("<b>a</b>".into());
        preserve_rich_format(&mut incoming, &existing);
        assert_eq!(incoming.content_type, "rich_text");
        assert_eq!(incoming.html_content, existing.html_content);
    }
}
