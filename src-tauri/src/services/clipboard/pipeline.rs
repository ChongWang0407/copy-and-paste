use crate::app_state::{AppDataDir, PasteQueue, SessionHistory, SettingsState};
use crate::database::{
    attachment_extension, calc_image_hash, calc_text_hash, is_text_type, stable_bytes_hash,
    DbState, PendingAttachments,
};
use crate::domain::models::{ClipboardEntry, EntryIdentity, EntryVersion};
use crate::services::clipboard::utils::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
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
    pub encoded_image: Option<(Vec<u8>, String)>,
    content_hash: Option<i64>,
    persistent: bool,
    data_dir: PathBuf,
    pending_attachments: PendingAttachments,
    formats_cleared: bool,
}

impl PipelineContext {
    pub fn new(
        app_handle: AppHandle,
        data: ClipboardData,
        source_app: String,
        source_app_path: Option<String>,
        timestamp: i64,
        persistent: bool,
    ) -> Self {
        let data_dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
        Self {
            data,
            app_handle,
            source_app,
            source_app_path,
            timestamp,
            entry: None,
            should_stop: false,
            pending_removals: Vec::new(),
            encoded_image: None,
            content_hash: None,
            persistent,
            data_dir,
            pending_attachments: PendingAttachments::default(),
            formats_cleared: false,
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
                Box::new(PersistenceStage),
                Box::new(DistributionStage),
            ],
        }
    }

    pub fn execute(&self, context: &mut PipelineContext) {
        let _attachments = crate::database::ATTACHMENT_WRITERS.read().unwrap();
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
        let (content_type, content, html_content) = match &mut ctx.data {
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
                ctx.encoded_image = Some((std::mem::take(bytes), std::mem::take(extension)));
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
                        ctx.formats_cleared |= apply_cleaned_text(entry, cleaned);
                    }
                }
            }
        }

        // Session-only mode keeps images out of the permanent attachments folder:
        // they live under session/ for this run only (see session_storage_root).
        let storage_root = if ctx.persistent {
            ctx.data_dir.clone()
        } else {
            session_storage_root(&ctx.data_dir)
        };
        if entry.content_type == "image" {
            match capture_image(
                entry,
                ctx.encoded_image.take(),
                &storage_root,
                &mut ctx.pending_attachments,
            ) {
                Some(hash) => ctx.content_hash = Some(hash),
                None => {
                    eprintln!(">>> [CLIPBOARD] Could not save captured image");
                    ctx.should_stop = true;
                    return;
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
                    ctx.formats_cleared |= apply_cleaned_text(entry, cleaned);
                }
            }
        }

        if entry.content_type != "image" {
            ctx.content_hash = Some(calc_text_hash(&entry.content) as i64);
        }
        if let Some(html) = &entry.html_content {
            entry.html_content = if ctx.persistent {
                let html =
                    process_local_images_in_html(html, &storage_root, &mut ctx.pending_attachments);
                Some(externalize_rich_image_fallback(
                    &html,
                    &storage_root,
                    &mut ctx.pending_attachments,
                ))
            } else {
                Some(embed_local_images(html))
            };
        }
    }
}

fn apply_cleaned_text(entry: &mut ClipboardEntry, cleaned: String) -> bool {
    let cleaned = cleaned.replace("\r\n", "\n");
    if cleaned == entry.content {
        return false;
    }
    entry.content = cleaned;
    if entry.content_type == "rich_text" {
        entry.content_type = detect_content_type(&entry.content);
    }
    entry.html_content = None;
    entry.preview = build_entry_preview(&entry.content_type, &entry.content, None);
    true
}

fn capture_image(
    entry: &mut ClipboardEntry,
    encoded: Option<(Vec<u8>, String)>,
    root: &std::path::Path,
    pending: &mut PendingAttachments,
) -> Option<i64> {
    let attachments = root.join("attachments");
    let (bytes, extension) = match encoded {
        Some(image) => image,
        None => {
            let path = std::path::Path::new(&entry.content);
            let metadata = std::fs::metadata(path).ok()?;
            if !metadata.is_file() || metadata.len() == 0 {
                return None;
            }
            if path.starts_with(&attachments) {
                pending.protect(path);
                return calc_image_hash(&entry.content);
            }
            if metadata.len() > super::MAX_CAPTURE_IMAGE_BYTES as u64 {
                return calc_image_hash(&entry.content);
            }
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("png");
            (std::fs::read(path).ok()?, extension.to_string())
        }
    };
    let hash = stable_bytes_hash(&bytes);
    let extension = attachment_extension(&bytes, &extension);
    entry.content = pending
        .write(&attachments, &bytes, extension, hash)?
        .to_str()?
        .to_string();
    entry.is_external = true;
    Some(hash as i64)
}
fn duplicate_content_matches(content: &str, kind: &str, html: Option<&str>, old: &str, old_kind: &str, old_html: Option<&str>) -> bool {
    let compatible = kind == old_kind || (is_text_type(kind) && is_text_type(old_kind));
    if !compatible || normalize_clipboard_line_endings(content) != normalize_clipboard_line_endings(old) {
        return false;
    }
    if kind == "rich_text" && old_kind == "rich_text" {
        return match (html, old_html) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                normalize_clipboard_line_endings(&html_with_original_image_sources(a))
                    == normalize_clipboard_line_endings(&html_with_original_image_sources(b))
            }
            _ => false,
        };
    }
    true
}

fn preserve_rich_format(entry: &mut ClipboardEntry, existing: &ClipboardEntry) {
    if entry.content_type != "rich_text" && existing.content_type == "rich_text" {
        entry.content_type = existing.content_type.clone();
        entry.html_content = existing.html_content.clone();
    } else if existing
        .html_content
        .as_deref()
        .is_some_and(|html| html.contains("<!--CP_REMOTE_SRC:"))
        && entry.html_content.as_deref().is_some_and(has_remote_images)
    {
        entry.html_content = existing.html_content.clone();
    }
}

fn matches_capture(
    incoming: &ClipboardEntry,
    existing: &ClipboardEntry,
    formats_cleared: bool,
) -> bool {
    !(formats_cleared && existing.content_type == "rich_text")
        && duplicate_content_matches(
            &incoming.content,
            &incoming.content_type,
            incoming.html_content.as_deref(),
            &existing.content,
            &existing.content_type,
            existing.html_content.as_deref(),
        )
}

fn deduplicate_persistent(
    db: &DbState,
    conn: &rusqlite::Connection,
    entry: &mut ClipboardEntry,
    hash: Option<i64>,
    formats_cleared: bool,
) {
    let existing = if is_text_type(&entry.content_type) {
        db.repo
            .find_text_capture_candidates(conn, &entry.content, hash.unwrap())
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|id| db.repo.get_entry_by_id_with_conn(conn, id).ok().flatten())
            .find(|existing| matches_capture(entry, existing, formats_cleared))
    } else {
        let id = if entry.content_type == "image" {
            db.repo
                .find_image_capture_candidate(conn, &entry.content, hash)
        } else {
            db.repo
                .find_by_content_with_conn(conn, &entry.content, Some(&entry.content_type))
        };
        id.ok()
            .flatten()
            .and_then(|id| db.repo.get_entry_by_id_with_conn(conn, id).ok().flatten())
    };
    if let Some(existing) = existing {
        preserve_entry_metadata(entry, existing);
    }
}

fn preserve_entry_metadata(entry: &mut ClipboardEntry, existing: ClipboardEntry) {
    preserve_rich_format(entry, &existing);
    entry.id = existing.id;
    entry.created_at = existing.created_at;
    entry.last_used_at = existing.last_used_at;
    entry.is_pinned = existing.is_pinned;
    entry.pinned_order = existing.pinned_order;
    entry.note = existing.note;
    entry.ocr_text = existing.ocr_text;
    entry.use_count = existing.use_count + 1;
    for tag in existing.tags {
        if !entry.tags.contains(&tag) {
            entry.tags.push(tag);
        }
    }
}

// Stage 4: Persistence. Matching and updating a duplicate share one lock.
pub struct PersistenceStage;
impl PipelineStage for PersistenceStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_mut().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();
        let deduplicate = settings.deduplicate.load(Ordering::Relaxed);
        if ctx.persistent {
            let db = ctx.app_handle.state::<DbState>();
            let conn = db.conn.lock().unwrap();
            if deduplicate {
                deduplicate_persistent(&db, &conn, entry, ctx.content_hash, ctx.formats_cleared);
            }
            match db
                .repo
                .save_with_conn(&conn, entry, Some(&ctx.data_dir), ctx.content_hash)
            {
                Ok(id) => entry.id = id,
                Err(error) => {
                    eprintln!(">>> [CLIPBOARD] Could not save capture: {error}");
                    ctx.should_stop = true;
                    return;
                }
            }
            ctx.pending_attachments = PendingAttachments::default();
            if let Ok(deleted_ids) = db.repo.enforce_limit_with_conn(&conn, Some(&ctx.data_dir)) {
                ctx.pending_removals.extend(deleted_ids);
            }
            drop(conn);
            if deduplicate {
                let session = ctx.app_handle.state::<SessionHistory>();
                session.0.lock().unwrap().retain(|item| {
                    let duplicate = matches_capture(entry, item, ctx.formats_cleared);
                    if duplicate {
                        ctx.pending_removals.push(item.id);
                    }
                    !duplicate
                });
            }
        } else {
            let session = ctx.app_handle.state::<SessionHistory>();
            let mut session = session.0.lock().unwrap();
            if deduplicate {
                if let Some(index) = session
                    .iter()
                    .rposition(|item| matches_capture(entry, item, ctx.formats_cleared))
                {
                    let existing = session[index].clone();
                    let id = existing.id;
                    preserve_entry_metadata(entry, existing);
                    session[index] = entry.clone();
                    session.retain(|item| {
                        let duplicate =
                            item.id != id && matches_capture(entry, item, ctx.formats_cleared);
                        if duplicate {
                            ctx.pending_removals.push(item.id);
                        }
                        !duplicate
                    });
                    return;
                }
            }
            static NEXT_SESSION_ID: std::sync::atomic::AtomicI64 =
                std::sync::atomic::AtomicI64::new(-1);
            entry.id = NEXT_SESSION_ID.fetch_sub(1, Ordering::Relaxed);
            session.push_back(entry.clone());
            if session.len() > 500 {
                if let Some(removed) = session.pop_front() {
                    ctx.pending_removals.push(removed.id);
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

        for id in ctx.pending_removals.drain(..) {
            let _ = ctx.app_handle.emit("clipboard-removed", id);
        }

        // Sequential Queue updates
        if settings.sequential_mode.load(Ordering::Relaxed) {
            let queue_state = ctx.app_handle.state::<PasteQueue>();
            let mut queue = queue_state.0.lock().unwrap();
            if queue.last_action_was_paste {
                queue.items.clear();
                queue.last_action_was_paste = false;
            }
            queue.items.push_back(EntryIdentity::from_entry(entry));
        }

        // Notify
        let _ = ctx
            .app_handle
            .emit("clipboard-updated", truncate_entry_for_ui(entry.clone()));
        crate::app::capsule::notify_copied(&ctx.app_handle, entry);

        if entry.content_type == "image" {
            crate::services::ocr::enqueue(&ctx.app_handle, entry);
        }

        if entry.is_external
            && !std::path::Path::new(&entry.content).starts_with(&ctx.data_dir.join("attachments"))
            && !std::path::Path::new(&entry.content)
                .starts_with(session_storage_root(&ctx.data_dir))
        {
            if let Some(path) =
                crate::services::content_handler::existing_file_path_from_content(&entry.content)
            {
                crate::services::content_handler::register_file_preview_watch(
                    ctx.app_handle.clone(),
                    entry,
                    path,
                    true,
                );
            }
        }
        queue_remote_images(
            &ctx.app_handle,
            entry,
            ctx.persistent.then(|| ctx.data_dir.clone()),
        );
    }
}

struct RemoteImagesJob {
    app: AppHandle,
    version: EntryVersion,
    html: String,
    root: Option<PathBuf>,
}

fn queue_remote_images(app: &AppHandle, entry: &ClipboardEntry, root: Option<PathBuf>) {
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    static QUEUE: OnceLock<mpsc::SyncSender<RemoteImagesJob>> = OnceLock::new();

    let Some(html) = entry
        .html_content
        .as_ref()
        .filter(|html| has_remote_images(html))
    else {
        return;
    };
    let sender = QUEUE.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<RemoteImagesJob>(16);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..2 {
            let receiver = receiver.clone();
            std::thread::spawn(move || loop {
                let job = receiver.lock().unwrap().recv();
                match job {
                    Ok(job) => complete_remote_images(job),
                    Err(_) => break,
                }
            });
        }
        sender
    });
    // Saturation keeps the original HTML and must never delay clipboard capture.
    let _ = sender.try_send(RemoteImagesJob {
        app: app.clone(),
        version: EntryVersion::from_entry(entry),
        html: html.clone(),
        root,
    });
}

fn complete_remote_images(job: RemoteImagesJob) {
    let mut pending = PendingAttachments::default();
    let html = localize_remote_images(&job.html, job.root.as_deref(), &mut pending);
    if html == job.html || html.len() > super::MAX_PIPELINE_HTML_BYTES {
        return;
    }
    if job.version.id > 0 {
        let db = job.app.state::<DbState>();
        let conn = db.conn.lock().unwrap();
        match db
            .repo
            .store_capture_html(&conn, &job.version, &job.html, html)
        {
            Ok(Some(entry)) => {
                let _ = job
                    .app
                    .emit("clipboard-updated", truncate_entry_for_ui(entry));
            }
            Ok(None) => {}
            Err(error) => eprintln!(">>> [CLIPBOARD] Could not update copied HTML: {error}"),
        }
    } else {
        let session = job.app.state::<SessionHistory>();
        let mut session = session.0.lock().unwrap();
        if let Some(entry) = session.iter_mut().find(|entry| {
            job.version.matches(entry) && entry.html_content.as_deref() == Some(&job.html)
        }) {
            entry.html_content = Some(html);
            let _ = job
                .app
                .emit("clipboard-updated", truncate_entry_for_ui(entry.clone()));
        }
    }
}

use crate::app::data_directory::session_storage_root;

/// Delete session-only image files no session entry shows any more. Entries
/// leave the session history in many ways (delete, clear, tag delete, the
/// 500-item cap, paste-and-delete); sweeping by reference catches all of them.
/// Run right after user deletes, and every 30 s by start_session_file_sweeper.
pub fn sweep_session_files(app: &tauri::AppHandle) {
    let Ok(_attachments) = crate::database::ATTACHMENT_WRITERS.try_write() else {
        return;
    };
    let data_dir = app.state::<AppDataDir>().0.lock().unwrap().clone();
    let session = app.state::<SessionHistory>();
    let session = session.0.lock().unwrap();
    let db = app.state::<DbState>();
    let conn = db.conn.lock().unwrap();
    let root = session_storage_root(&data_dir);
    let attachments = root.join("attachments");
    let mut candidates = Vec::new();
    if crate::app::data_directory::owns_session(&root)
        && attachments.symlink_metadata().is_ok_and(|meta| meta.is_dir())
    {
        if let Ok(files) = std::fs::read_dir(attachments) {
            candidates.extend(files.flatten().filter(|file| {
                file.file_type().is_ok_and(|kind| kind.is_file())
            }).map(|file| file.path()));
        }
    }
    if let Err(error) = db.repo.sweep_attachment_deletions(&conn, &data_dir, candidates, session.iter()) {
        crate::error!(">>> [ATTACHMENTS] {error}");
    }
}

/// Startup: the previous run's session images are unreachable now; delete
/// them, then keep sweeping while the app runs.
pub fn start_session_file_sweeper(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        sweep_session_files(&app);
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
}

#[cfg(test)]
mod dedup_tests {
    use super::*;

    #[test]
    fn cleaned_text_does_not_reuse_html_or_native_formats() {
        let mut entry: ClipboardEntry = serde_json::from_value(serde_json::json!({
            "id": 4, "content_type": "rich_text", "content": "public secret", "source_app": "test",
            "timestamp": 1, "preview": "public secret", "is_pinned": false, "html_content": "<b>public secret</b>"
        })).unwrap();
        let old = entry.clone();
        assert!(apply_cleaned_text(&mut entry, "public".into()));
        assert!(entry.html_content.is_none());
        assert_eq!(entry.preview, "public");
        assert_eq!(entry.content_type, "text");
        let mut unrelated_rich = old;
        unrelated_rich.content = "public".into();
        assert!(!matches_capture(&entry, &unrelated_rich, true));
    }

    #[test]
    fn unavailable_attachment_directory_stops_image_capture() {
        let root = std::env::temp_dir().join(format!("cp-capture-{}", uuid::Uuid::new_v4()));
        std::fs::write(&root, b"not a directory").unwrap();
        let mut entry: ClipboardEntry = serde_json::from_value(serde_json::json!({
            "id": 0, "content_type": "image", "content": "", "source_app": "test",
            "timestamp": 1, "preview": "", "is_pinned": false,
        }))
        .unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(capture_image(
            &mut entry,
            Some((bytes.into_inner(), "png".into())),
            &root,
            &mut PendingAttachments::default()
        )
        .is_none());
        assert_eq!(entry.id, 0);
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn localized_html_still_deduplicates_against_its_source() {
        let raw = r#"<img src="https://example.org/image.png">"#;
        use base64::Engine;
        let original =
            base64::engine::general_purpose::STANDARD.encode("https://example.org/image.png");
        let localized =
            format!(r#"<img src="file:///tmp/img_1.png"><!--CP_REMOTE_SRC:{original}-->"#);
        assert!(duplicate_content_matches(
            "caption",
            "rich_text",
            Some(raw),
            "caption",
            "rich_text",
            Some(&localized)
        ));
    }
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
