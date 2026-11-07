// Clipboard operations module
use crate::app_state::SessionHistory;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use crate::services::clipboard::{
    app_likely_word_processor,
    build_exact_paste_html,
    html_has_renderable_rich_body, normalize_plain_text_for_clipboard_paste,
    plain_text_from_tabular_html, repair_html_fragment, rich_paste_needs_exact_text,
    rtf_bytes_from_named_formats, sanitize_tabular_html_for_paste,
    split_rich_html_and_image_fallback, split_rich_html_and_named_formats,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use tauri::{Emitter, Manager, State};
use urlencoding::decode;

/// The whole pasteboard, every item and type (lossless), so an image, rich
/// text or files come back exactly after a batch paste.
fn capture_clipboard_snapshot() -> Option<crate::infrastructure::macos_api::clipboard::PasteboardSnapshot> {
    crate::infrastructure::macos_api::clipboard::snapshot_pasteboard(64 * 1024 * 1024)
}



/// The entry's content naming its file where the history list finds it, so
/// an image the list shows also pastes after the data folder moved. Callers
/// swap it in before anything remembers the paste: the echo hash and the
/// recent paste must name the path that goes on the clipboard (Windows puts
/// it there as a file, macOS for GIFs), or the monitor records the paste as a
/// new copy.
pub(crate) fn with_current_entry_file(
    app_handle: &tauri::AppHandle,
    content_type: &str,
    content: String,
) -> String {
    crate::services::content_handler::app_data_dir(app_handle)
        .and_then(|data_dir| current_entry_file(content_type, &content, &data_dir))
        .unwrap_or(content)
}

/// Where a single-file entry's file is now, if not at its stored path.
fn current_entry_file(
    content_type: &str,
    content: &str,
    data_dir: &std::path::Path,
) -> Option<String> {
    let raw_path = content.trim();
    if !matches!(content_type, "image" | "video" | "file")
        || raw_path.starts_with("data:")
        || raw_path.contains('\n')
    {
        return None;
    }
    let path = crate::database::resolve_entry_file_path(raw_path, data_dir)?;
    (path.as_os_str() != raw_path).then(|| path.to_string_lossy().into_owned())
}

fn resolve_rich_image_fallback_bytes(payload: &str) -> Option<Vec<u8>> {
    let value = payload.trim();

    if value.starts_with("data:image/") {
        let b64_data = value.split(',').nth(1)?;
        if b64_data.is_empty() {
            return None;
        }
        return general_purpose::STANDARD.decode(b64_data).ok();
    }

    let path_raw = if value.starts_with("file://") {
        value.trim_start_matches("file://")
    } else {
        value
    };

    let decoded_path = decode(path_raw)
        .map(|p| p.into_owned())
        .unwrap_or_else(|_| path_raw.to_string());

    if decoded_path.is_empty() {
        return None;
    }

    std::fs::read(decoded_path).ok()
}

fn rich_image_mime_from_bytes(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes).ok() {
        Some(image::ImageFormat::Jpeg) => "image/jpeg",
        Some(image::ImageFormat::Gif) => "image/gif",
        Some(image::ImageFormat::WebP) => "image/webp",
        Some(image::ImageFormat::Bmp) => "image/bmp",
        Some(image::ImageFormat::Png) => "image/png",
        _ => "image/png",
    }
}

fn rich_image_payload_to_data_url(payload: &str) -> Option<String> {
    let value = payload.trim();
    if value.starts_with("data:image/") {
        return Some(value.to_string());
    }

    let bytes = resolve_rich_image_fallback_bytes(value)?;
    let mime = rich_image_mime_from_bytes(&bytes);
    let b64 = general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}

fn html_has_embedded_data_image(html: &str) -> bool {
    static IMG_DATA_RE: OnceLock<Regex> = OnceLock::new();

    IMG_DATA_RE
        .get_or_init(|| {
            Regex::new(r#"(?is)<img\b[^>]*\bsrc\s*=\s*['"]data:image/[^'"]+['"]"#).unwrap()
        })
        .is_match(html)
}

fn append_fallback_image_to_html(html: &str, data_url: &str) -> String {
    let img_tag = format!(r#"<img src="{}" />"#, data_url);
    let trimmed = html.trim();

    if trimmed.is_empty() {
        return format!("<html><body>{}</body></html>", img_tag);
    }

    let lower = html.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + img_tag.len() + 1);
        out.push_str(&html[..idx]);
        if !html[..idx].ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&img_tag);
        out.push_str(&html[idx..]);
        return out;
    }

    let mut out = String::with_capacity(html.len() + img_tag.len() + 1);
    out.push_str(html.trim_end());
    out.push('\n');
    out.push_str(&img_tag);
    out
}

/// The HTML to paste plus the picture the entry was copied with, if any.
/// Office and WPS put a bitmap of the selection next to the HTML, and each
/// target picks the format it can use (WeChat the picture, Word the table).
/// Pasting both does the same as pasting from the source app, instead of
/// guessing per target which one to give.
fn materialize_rich_html_for_paste(
    html: &str,
    fallback_image_payload: Option<&str>,
) -> (String, Option<Vec<u8>>) {
    let html_with_embedded_images = crate::services::clipboard::embed_local_images(html);
    let fallback_bytes = fallback_image_payload.and_then(resolve_rich_image_fallback_bytes);

    if html_has_embedded_data_image(&html_with_embedded_images)
        || html_has_renderable_rich_body(&html_with_embedded_images)
    {
        return (html_with_embedded_images, fallback_bytes);
    }

    // Nothing to show in the HTML itself: put the picture in it too.
    let Some(data_url) = fallback_image_payload.and_then(rich_image_payload_to_data_url) else {
        return (html_with_embedded_images, fallback_bytes);
    };
    (
        append_fallback_image_to_html(&html_with_embedded_images, &data_url),
        fallback_bytes,
    )
}


/// Whether a paste keeps the entry's formatting. An explicit choice wins: a
/// click keeps the format, a right-click or the plain text hotkey drops it.
/// Callers with no choice of their own (the sequential paste queue) keep the
/// format of rich text entries too. An entry with no text at all (only
/// pictures) keeps it either way: as plain text it pasted nothing.
pub(crate) fn resolve_paste_with_format(
    content: &str,
    content_type: &str,
    html_content: Option<&str>,
    paste_with_format: Option<bool>,
) -> bool {
    let Some(html) = html_content else {
        return false;
    };
    if content.is_empty() && !html.trim().is_empty() {
        return true;
    }
    paste_with_format
        .unwrap_or_else(|| content_type == "rich_text" && !rich_paste_needs_exact_text(content, html))
}

/// Simulating Cmd+V needs Accessibility on macOS; elsewhere it always works.
fn paste_permission_missing() -> bool {
    #[cfg(target_os = "macos")]
    return !crate::infrastructure::macos_api::permissions::has_accessibility_permission();
}

fn require_paste_permission(app: &tauri::AppHandle) -> AppResult<()> {
    if paste_permission_missing() {
        let _ = app.emit("accessibility-permission-required", ());
        return Err(AppError::Validation("accessibility_permission_required".into()));
    }
    Ok(())
}

async fn copy_to_clipboard_inner(
    app_handle: tauri::AppHandle,
    state: &DbState,
    session: &SessionHistory,
    mut content: String,
    mut content_type: String,
    paste: bool,
    id: i64,
    delete_after_use: bool,
    paste_with_format: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    // Without paste permission the item is still copied, so the user can press
    // Cmd+V themselves (what the permission prompt tells them); only the
    // window hand-off and the simulated keystroke are skipped.
    let paste_blocked = paste && paste_permission_missing();
    let paste = paste && !paste_blocked;
    println!(
        "[DEBUG] copy_to_clipboard called: id={}, paste={}, content_type={}, content_len={}",
        id,
        paste,
        content_type,
        content.len()
    );

    let mut html_content: Option<String> = None;

    // 0. Resolve full content if ID is provided and content is placeholder/truncated
    if id != 0 {
        if id > 0 {
            // Fetch from Database
            if let Ok(Some((full_content, ctype, html))) =
                state.repo.get_entry_content_with_html(id)
            {
                content = full_content;
                html_content = html;
                content_type = ctype;
            }
        } else {
            // Fetch from Session
            let session_items = session.0.lock().unwrap();
            if let Some(item) = session_items.iter().find(|i| i.id == id) {
                content = item.content.clone();
                html_content = item.html_content.clone();
                content_type = item.content_type.clone();
            }
        }
    }
    content = with_current_entry_file(&app_handle, &content_type, content);

    // 1. Handle Window Visibility and Focus
    if paste {
        handle_window_focus_for_paste(&app_handle).await?;
    }

    // 2. Copy to system clipboard
    prepare_clipboard_payload(
        &content,
        &content_type,
        html_content.as_deref(),
        resolve_paste_with_format(
            &content,
            &content_type,
            html_content.as_deref(),
            paste_with_format,
        ),
    )
    .await?;

    if paste_blocked {
        // Emits the event that opens the permission prompt; the error tells the
        // frontend not to treat this as a completed paste.
        return require_paste_permission(&app_handle);
    }

    // 3. Perform paste action if requested
    if paste {
        perform_paste_action(&app_handle, &state, id, delete_after_use, move_to_top).await?;
    }

    Ok(())
}

/// Held from writing the clipboard until the paste keystroke has gone out.
/// Without it two quick pastes (sequential-paste presses, fast clicks on
/// history items) interleave: the second write lands before the first Cmd/Ctrl+V,
/// so one item is pasted twice and the other never.
pub static PASTE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());



#[tauri::command]
pub async fn copy_to_clipboard(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    content: String,
    content_type: String,
    paste: bool,
    id: i64,
    delete_after_use: bool,
    paste_with_format: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    let _paste_guard = PASTE_LOCK.lock().await;
    copy_to_clipboard_inner(
        app_handle,
        &state,
        &session,
        content,
        content_type,
        paste,
        id,
        delete_after_use,
        paste_with_format,
        move_to_top,
    )
    .await
}



/// An entry's full content, type and HTML; files resolved to where they are now.
fn entry_full(
    app_handle: &tauri::AppHandle,
    state: &DbState,
    session: &SessionHistory,
    id: i64,
) -> Option<(String, String, Option<String>)> {
    let entry = if id > 0 {
        state.repo.get_entry_content_with_html(id).ok().flatten()
    } else {
        let items = session.0.lock().unwrap();
        items
            .iter()
            .find(|item| item.id == id)
            .map(|item| (item.content.clone(), item.content_type.clone(), item.html_content.clone()))
    };
    entry.map(|(content, kind, html)| {
        (with_current_entry_file(app_handle, &kind, content), kind, html)
    })
}

async fn paste_plain_string(
    app_handle: &tauri::AppHandle,
    state: &DbState,
    text: &str,
    source_id: i64,
    delete_after_use: bool,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    require_paste_permission(app_handle)?;
    handle_window_focus_for_paste(app_handle).await?;
    prepare_clipboard_payload(text, "text", None, false).await?;
    perform_paste_action(app_handle, state, source_id, delete_after_use, move_to_top).await
}

/// Pastes content that isn't a history entry (常用). Nothing to delete or move.
pub async fn paste_stored_content(
    app_handle: &tauri::AppHandle,
    state: &DbState,
    content: &str,
    content_type: &str,
    html: Option<&str>,
) -> AppResult<()> {
    let _paste_guard = PASTE_LOCK.lock().await;
    require_paste_permission(app_handle)?;
    handle_window_focus_for_paste(app_handle).await?;
    prepare_clipboard_payload(content, content_type, html, html.is_some()).await?;
    perform_paste_action(app_handle, state, 0, false, Some(false)).await
}

/// Pastes any text: an edited entry, joined batch text. `source_id` is the
/// entry it came from, for 粘贴后自动删除 / 置顶; 0 for none.
#[tauri::command]
pub async fn paste_text(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    text: String,
    source_id: Option<i64>,
    delete_after_use: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    if text.is_empty() {
        return Ok(());
    }
    let _paste_guard = PASTE_LOCK.lock().await;
    paste_plain_string(
        &app_handle,
        &state,
        &text,
        source_id.unwrap_or(0),
        delete_after_use.unwrap_or(false),
        move_to_top,
    )
    .await
}

/// 选择粘贴格式 › Markdown 文本.
#[tauri::command]
pub async fn paste_entry_as_markdown(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
    delete_after_use: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    let _paste_guard = PASTE_LOCK.lock().await;
    let (content, content_type, html) = entry_full(&app_handle, &state, &session, id)
        .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
    let markdown = crate::services::markdown::entry_to_markdown(&content, &content_type, html.as_deref());
    paste_plain_string(
        &app_handle,
        &state,
        &markdown,
        id,
        delete_after_use.unwrap_or(false),
        move_to_top,
    )
    .await
}

/// 粘贴为文字: the text recognized in an image. Copy-time OCR usually has it
/// ready; if not, recognize now.
#[tauri::command]
pub async fn paste_image_text(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
    delete_after_use: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    let _paste_guard = PASTE_LOCK.lock().await;
    let text = crate::services::ocr::text_for_entry(&app_handle, &state, &session, id).await?;
    if text.trim().is_empty() {
        return Err(AppError::Validation("ocr_no_text".to_string()));
    }
    paste_plain_string(
        &app_handle,
        &state,
        &text,
        id,
        delete_after_use.unwrap_or(false),
        move_to_top,
    )
    .await
}

/// One step of a batch paste.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BatchSegment {
    Text { text: String },
    Entry { id: i64 },
}

/// Pause between steps of a batch paste. Slow (Electron) apps read the
/// clipboard late, and the next step must not overwrite it before they do.
const BATCH_STEP_DELAY_MS: u64 = 350;
/// Time for the target to read the last step before the clipboard is restored.
const BATCH_RESTORE_DELAY_MS: u64 = 450;

/// Clears the pause even if a step fails.
struct MonitorPause;

impl MonitorPause {
    fn start() -> Self {
        crate::CLIPBOARD_MONITOR_PAUSED.store(true, Ordering::Relaxed);
        Self
    }
}

impl Drop for MonitorPause {
    fn drop(&mut self) {
        crate::CLIPBOARD_MONITOR_PAUSED.store(false, Ordering::Relaxed);
    }
}

/// 批量粘贴: runs the steps in order with one focus hand-off, then puts the
/// clipboard back. The monitor is paused so no step is recorded as a new
/// entry. `source_ids` get the usual after-paste actions.
#[tauri::command]
pub async fn paste_batch(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    segments: Vec<BatchSegment>,
    source_ids: Vec<i64>,
    delete_after_use: Option<bool>,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    if segments.is_empty() {
        return Ok(());
    }
    let _paste_guard = PASTE_LOCK.lock().await;
    require_paste_permission(&app_handle)?;

    let previous_clipboard = capture_clipboard_snapshot();
    let pause = MonitorPause::start();
    handle_window_focus_for_paste(&app_handle).await?;

    let total = segments.len();
    for (index, segment) in segments.into_iter().enumerate() {
        match segment {
            BatchSegment::Text { text } => {
                prepare_clipboard_payload(&text, "text", None, false).await?;
            }
            BatchSegment::Entry { id } => {
                let (content, content_type, html) = entry_full(&app_handle, &state, &session, id)
                    .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
                prepare_clipboard_payload(&content, &content_type, html.as_deref(), html.is_some())
                    .await?;
            }
        }
        ensure_paste_target_frontmost().await;
        send_paste_keystroke();
        if index + 1 < total {
            tokio::time::sleep(std::time::Duration::from_millis(BATCH_STEP_DELAY_MS)).await;
        }
    }

    hide_window_after_paste(&app_handle).await;
    tokio::time::sleep(std::time::Duration::from_millis(BATCH_RESTORE_DELAY_MS)).await;
    if let Some(snapshot) = previous_clipboard {
        crate::infrastructure::macos_api::clipboard::restore_pasteboard(&snapshot)
            .map_err(AppError::Internal)?;
    }
    drop(pause);

    for id in source_ids {
        handle_post_paste_actions(
            &app_handle,
            &state,
            id,
            delete_after_use.unwrap_or(false),
            move_to_top,
        )?;
    }
    Ok(())
}

async fn handle_window_focus_for_paste(app_handle: &tauri::AppHandle) -> AppResult<()> {
    let window_pinned = crate::WINDOW_PINNED.load(Ordering::Relaxed);
    let mut window_was_visible = false;

    // 1. Hide (unless pinned) before we hand control back to the target app.
    if let Some(window) = app_handle.get_webview_window("main") {
        window_was_visible = window.is_visible().unwrap_or(false);
        crate::infrastructure::macos_api::window::set_window_focusable(&window, false);
        if !window_pinned {
            crate::app::window_manager::before_hide(app_handle);
            let _ = window.set_always_on_top(false);
            let _ = crate::infrastructure::macos_api::window::hide_clipboard_panel(app_handle);
        }
    }

    // 2. Hand focus back to the target app before sending the paste keystroke.
    if window_was_visible || window_pinned {

        #[cfg(target_os = "macos")]
        {
            let activated = handoff_macos_focus_for_paste();
            tokio::time::sleep(std::time::Duration::from_millis(if activated {
                60
            } else {
                8
            }))
            .await;
        }

    } else {
        // The window never took focus: only wait for the hide animation and hotkey key-up to settle.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    Ok(())
}

/// Give keyboard ownership back to the paste target without needlessly
/// re-activating its whole application. A non-activating NSPanel can be key
/// while the target remains frontmost; calling `activateWithOptions` in that
/// state reorders/flashes the target application's windows.
#[cfg(target_os = "macos")]
fn handoff_macos_focus_for_paste() -> bool {
    if let Some(app_handle) = crate::global_state::GLOBAL_APP_HANDLE.get() {
        let _ = crate::infrastructure::macos_api::window::resign_clipboard_panel_key(app_handle);
    }

    let prev_pid = crate::global_state::LAST_ACTIVE_APP_PID.load(Ordering::Relaxed);
    if prev_pid != 0 {
        let frontmost = crate::infrastructure::macos_api::window::get_active_app_snapshot();
        if frontmost.pid == prev_pid {
            return false;
        }

        if crate::infrastructure::macos_api::apps::activate_app_by_pid_soft(prev_pid as i32) {
            return true;
        }
        if crate::infrastructure::macos_api::apps::activate_app_by_pid(prev_pid as i32) {
            return true;
        }
    }

    let prev_app = crate::global_state::get_last_active_app_name();
    if prev_app.trim().is_empty() {
        return false;
    }
    crate::infrastructure::macos_api::apps::activate_app_by_name(&prev_app)
}

#[cfg(target_os = "macos")]
fn macos_previous_app_is_frontmost() -> bool {
    let prev_pid = crate::global_state::LAST_ACTIVE_APP_PID.load(Ordering::Relaxed);
    prev_pid != 0
        && crate::infrastructure::macos_api::window::get_active_app_snapshot().pid == prev_pid
}

fn calculate_content_hash(content: &str) -> (u64, u64) {
    let normalized = crate::services::clipboard::normalize_clipboard_line_endings(content);
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let content_hash = hasher.finish();

    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    (content_hash, current_time)
}

pub async fn prepare_clipboard_payload(
    content: &str,
    content_type: &str,
    html_content: Option<&str>,
    paste_with_format: bool,
) -> AppResult<()> {
    let (content_hash, current_time) = calculate_content_hash(content);
    crate::LAST_APP_SET_HASH.store(content_hash, Ordering::SeqCst);
    crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
    crate::LAST_APP_SET_TIMESTAMP.store(current_time, Ordering::SeqCst);

    copy_content_to_system_clipboard(
        content,
        content_type,
        html_content,
        paste_with_format,
        content_hash,
        current_time,
    )
    .await
}

async fn copy_content_to_system_clipboard(
    content: &str,
    content_type: &str,
    html_content: Option<&str>,
    paste_with_format: bool,
    content_hash: u64,
    current_time: u64,
) -> AppResult<()> {
    match content_type {
        "image" | "video" | "file" => {
            if content_hash == 0 {
                crate::LAST_APP_SET_HASH.store(1, Ordering::SeqCst);
            }

            let file_paths = content
                .lines()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let is_file_path_payload = !content.starts_with("data:")
                && !file_paths.is_empty()
                && file_paths
                    .iter()
                    .all(|path| std::path::Path::new(path).is_absolute());

            if is_file_path_payload {
                if content_type == "image" {
                    // For image type with local path, read pixels for better compatibility with chat apps
                    let image_path = &file_paths[0];
                    let bytes = std::fs::read(image_path).map_err(|_| {
                        AppError::Validation("图片文件已不存在，无法粘贴。".to_string())
                    })?;
                    let (primary_hash, _secondary_hash) =
                        copy_image_bytes_to_clipboard(bytes, current_time, Some(image_path))?;
                    // Keep LAST_APP_SET_HASH as content_hash (path hash)
                    // Store pixel/byte hash in HASH_ALT
                    crate::LAST_APP_SET_HASH_ALT.store(primary_hash, Ordering::SeqCst);
                } else {
                    // Paste the files that still exist. A missing one made
                    // Explorer/Finder abort or error while the app said it worked.
                    let total = file_paths.len();
                    let file_paths: Vec<String> = file_paths
                        .into_iter()
                        .filter(|path| std::path::Path::new(path).exists())
                        .collect();
                    if file_paths.is_empty() {
                        return Err(AppError::Validation(
                            "这些文件都已不存在，无法粘贴。".to_string(),
                        ));
                    }
                    if file_paths.len() != total {
                        // What the monitor will read back, so it isn't re-recorded.
                        let (hash, _) = calculate_content_hash(&file_paths.join("\n"));
                        crate::LAST_APP_SET_HASH.store(hash, Ordering::SeqCst);
                    }


                    #[cfg(target_os = "macos")]
                    {
                        crate::infrastructure::macos_api::clipboard::set_clipboard_files(
                            file_paths,
                        )
                        .map_err(AppError::Internal)?;
                    }

                }
            } else if content_type == "image" {
                let b64_data = if content.starts_with("data:image") {
                    content.split(',').nth(1).unwrap_or(content)
                } else {
                    content
                };

                let bytes = general_purpose::STANDARD
                    .decode(b64_data)
                    .map_err(|e| AppError::Internal(format!("无法读取图片内容。\n详情：{}", e)))?;

                let (primary_hash, _secondary_hash) =
                    copy_image_bytes_to_clipboard(bytes, current_time, None)?;
                // Keep LAST_APP_SET_HASH as content_hash (dataurl hash)
                // Store pixel/byte hash in HASH_ALT
                crate::LAST_APP_SET_HASH_ALT.store(primary_hash, Ordering::SeqCst);
            } else {
                let mut clipboard = arboard::Clipboard::new().map_err(AppError::from)?;
                clipboard
                    .set_text(content.to_string())
                    .map_err(AppError::from)?;
            }
        }
        ct if ct == "rich_text" || (paste_with_format && html_content.is_some()) => {
            if let Some(html) = html_content {
                if paste_with_format {
                    let (html_without_formats, named_formats) =
                        split_rich_html_and_named_formats(html);
                    let (clean_html, fallback_image_data_url) =
                        split_rich_html_and_image_fallback(&html_without_formats);
                    let base_html = if clean_html.trim().is_empty() {
                        repair_html_fragment(html)
                    } else {
                        repair_html_fragment(&clean_html)
                    };
                    let (final_html, image_bytes) = materialize_rich_html_for_paste(
                        &base_html,
                        fallback_image_data_url.as_deref(),
                    );
                    let mut paste_html = sanitize_tabular_html_for_paste(&final_html);
                    let mut paste_plain = plain_text_from_tabular_html(&paste_html)
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| normalize_plain_text_for_clipboard_paste(content));
                    if rich_paste_needs_exact_text(content, &paste_html) {
                        paste_plain = normalize_plain_text_for_clipboard_paste(content);
                        paste_html =
                            sanitize_tabular_html_for_paste(&build_exact_paste_html(content));
                    }

                    #[cfg(target_os = "macos")]
                    {
                        let paste_target = (
                            crate::global_state::get_last_active_app_name(),
                            None::<String>,
                        );
                        let paste_rtf = if app_likely_word_processor(
                            &paste_target.0,
                            paste_target.1.as_deref(),
                        ) {
                            rtf_bytes_from_named_formats(&named_formats)
                        } else {
                            None
                        };
                        crate::infrastructure::macos_api::clipboard::set_clipboard_text_html_rtf_and_image(
                            &paste_plain,
                            &paste_html,
                            paste_rtf,
                            image_bytes,
                        )
                        .map_err(AppError::Internal)?;
                    }


                    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
                    {
                        let _ = image_bytes;
                        let mut clipboard = arboard::Clipboard::new().map_err(AppError::from)?;
                        clipboard
                            .set_html(paste_html, Some(paste_plain))
                            .map_err(AppError::from)?;
                    }
                } else {
                    copy_text_with_retry(content).await?;
                }
            } else {
                copy_text_with_retry(content).await?;
            }
        }
        _ => {
            copy_text_with_retry(content).await?;
        }
    }

    Ok(())
}

fn image_extension_for_bytes(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 3 && &bytes[0..3] == b"GIF" {
        "gif"
    } else if bytes.len() >= 8
        && bytes[0] == 0x89
        && &bytes[1..4] == b"PNG"
        && bytes[4] == 0x0D
        && bytes[5] == 0x0A
        && bytes[6] == 0x1A
        && bytes[7] == 0x0A
    {
        "png"
    } else if bytes.len() >= 3 && &bytes[0..3] == b"\xFF\xD8\xFF" {
        "jpg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        "png"
    }
}

/// Writes the picture of a rich entry as image formats only. Unlike a plain
/// image paste there is no file next to it: a copied table is not a file, and
/// chat apps would send one as an attachment.



fn encode_png_from_rgba(rgba: &image::RgbaImage) -> AppResult<Vec<u8>> {
    let mut png_buf = Vec::new();
    image::DynamicImage::ImageRgba8(rgba.clone())
        .write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )
        .map_err(|e| AppError::Internal(format!("无法生成图片。\n详情：{}", e)))?;
    Ok(png_buf)
}

/// macOS: write the stored bytes as they are (see set_clipboard_image). Only
/// non-PNG images are decoded, for a static PNG of their first frame. The echo
/// hash is the hash of what the monitor reads back first: the GIF bytes for a
/// GIF, otherwise the PNG (it used to compare a GIF-bytes hash with a hash of
/// decoded pixels, so every GIF paste came back as a new flattened entry).
#[cfg(target_os = "macos")]
fn copy_image_bytes_to_clipboard_macos(
    bytes: &[u8],
    is_gif: bool,
    source_path: Option<&str>,
) -> AppResult<(u64, u64)> {
    let extension = image_extension_for_bytes(bytes);
    let static_png = if extension == "png" {
        None
    } else {
        let first_frame = image::load_from_memory(bytes)
            .map_err(|e| AppError::Internal(format!("无法打开图片。\n详情：{}", e)))?
            .to_rgba8();
        Some(encode_png_from_rgba(&first_frame)?)
    };
    // The file itself lets chat apps and Finder take the animated original.
    let file_path = source_path
        .map(str::trim)
        .filter(|path| is_gif && !path.is_empty() && std::path::Path::new(path).is_file());
    crate::infrastructure::macos_api::clipboard::set_clipboard_image(
        bytes,
        extension,
        static_png.as_deref(),
        file_path,
    )
    .map_err(AppError::Internal)?;

    let echo_bytes: &[u8] = if is_gif { bytes } else { static_png.as_deref().unwrap_or(bytes) };
    let mut hasher = DefaultHasher::new();
    echo_bytes.to_vec().hash(&mut hasher);
    Ok((hasher.finish(), 0))
}

fn copy_image_bytes_to_clipboard(
    bytes: Vec<u8>,
    current_time: u64,
    source_path: Option<&str>,
) -> AppResult<(u64, u64)> {
    // Check if it's a GIF by magic number
    let is_gif = bytes.len() > 3 && &bytes[0..3] == b"GIF";

    #[cfg(target_os = "macos")]
    {
        crate::LAST_APP_SET_TIMESTAMP.store(current_time, Ordering::SeqCst);
        return copy_image_bytes_to_clipboard_macos(&bytes, is_gif, source_path);
    }

    #[allow(unreachable_code)]
    let (width, height, raw_bytes) = {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| AppError::Internal(format!("无法打开图片。\n详情：{}", e)))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        (w, h, img.into_raw())
    };

    crate::LAST_APP_SET_TIMESTAMP.store(current_time, Ordering::SeqCst);

    let (primary_hash, secondary_hash) = if is_gif {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let byte_hash = hasher.finish();

        // Calculate pixel hash of the first frame as a secondary fingerprint
        let pixel_count = (width as u64) * (height as u64);
        let mut h = pixel_count;
        if !raw_bytes.is_empty() {
            h = h
                .wrapping_add(raw_bytes[0] as u64)
                .wrapping_add(raw_bytes[raw_bytes.len() / 2] as u64)
                .wrapping_add(raw_bytes[raw_bytes.len() - 1] as u64);
        }
        (byte_hash, h)
    } else {
        // Hash full pixel bytes so the monitor can skip our own image copy
        let mut hasher = DefaultHasher::new();
        raw_bytes.hash(&mut hasher);
        let byte_hash = hasher.finish();
        (byte_hash, 0)
    };


    {
        let _ = source_path;
        let mut clipboard = arboard::Clipboard::new().map_err(AppError::from)?;
        clipboard
            .set_image(arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: raw_bytes.into(),
            })
            .map_err(AppError::from)?;
    }

    Ok((primary_hash, secondary_hash))
}

async fn copy_text_with_retry(content: &str) -> AppResult<()> {
    println!("[DEBUG] Copying text to clipboard: {} chars", content.len());
    // Use the platform line-ending conversion without trimming. In particular,
    // trailing spaces and tabs are part of the clipboard payload and must survive
    // a history-item click exactly as captured.
    let paste_text = normalize_plain_text_for_clipboard_paste(content);

    #[cfg(target_os = "macos")]
    {
        crate::infrastructure::macos_api::clipboard::set_clipboard_text_and_html(&paste_text, "")
            .map_err(AppError::Internal)?;
    }


    Ok(())
}

/// This is only a recovery path. The normal path is already frontmost and
/// proceeds immediately after the initial 8 ms responder handoff.
async fn ensure_paste_target_frontmost() {
    if !macos_previous_app_is_frontmost() && handoff_macos_focus_for_paste() {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
}

async fn perform_paste_action(
    app_handle: &tauri::AppHandle,
    state: &DbState,
    id: i64,
    delete_after_use: bool,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    ensure_paste_target_frontmost().await;

    send_paste_keystroke();

    // Hide after paste if not pinned
    hide_window_after_paste(app_handle).await;

    // Handle post-paste actions
    handle_post_paste_actions(app_handle, state, id, delete_after_use, move_to_top)?;

    Ok(())
}

async fn hide_window_after_paste(app_handle: &tauri::AppHandle) {
    if crate::WINDOW_PINNED.load(Ordering::Relaxed) {
        return;
    }
    if app_handle.get_webview_window("main").is_some() {
        crate::app::window_manager::before_hide(app_handle);
        let _ = crate::infrastructure::macos_api::window::hide_clipboard_panel(app_handle);
        crate::NAVIGATION_ENABLED.store(false, Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

pub fn send_paste_keystroke() {
    // Focus was already handed off and allowed to settle before the clipboard
    // payload was prepared. Do not schedule another NSPanel resign here: it
    // can race the Command+V event.
    if !crate::infrastructure::macos_api::permissions::has_accessibility_permission() {
        println!("[WARN] Accessibility permission missing; open Settings → Privacy → Accessibility");
        return;
    }
    if !crate::infrastructure::macos_api::permissions::send_command_v() {
        println!("[WARN] Native Command+V dispatch failed");
    }
}

fn handle_post_paste_actions(
    app_handle: &tauri::AppHandle,
    state: &DbState,
    id: i64,
    delete_after_use: bool,
    move_to_top: Option<bool>,
) -> AppResult<()> {
    if delete_after_use {
        // Fetch metadata to check for pinned or tagging protection
        let is_protected = if id != 0 {
            if id > 0 {
                state
                    .repo
                    .get_entry_by_id(id)
                    .map(|e| e.is_some_and(|i| i.is_pinned || !i.tags.is_empty()))
                    .unwrap_or(false)
            } else {
                let session = app_handle.state::<crate::app_state::SessionHistory>();
                let s = session.0.lock().unwrap();
                s.iter()
                    .find(|i| i.id == id)
                    .map(|i| i.is_pinned || !i.tags.is_empty())
                    .unwrap_or(false)
            }
        } else {
            false
        };

        if !is_protected {
            // First remove from SessionHistory
            let session = app_handle.state::<crate::app_state::SessionHistory>();
            {
                let mut s = session.0.lock().unwrap();
                if let Some(pos) = s.iter().position(|i| i.id == id) {
                    s.remove(pos);
                }
            }

            if id > 0 {
                // Cleanup persistent file and DB entry if needed
                let app_data = app_handle.state::<crate::app_state::AppDataDir>();
                let data_dir = app_data.0.lock().unwrap();

                if state.repo.delete_after_paste(id, Some(&data_dir)).is_ok() {
                    let _ = app_handle.emit("clipboard-removed", id);
                }
            } else {
                let _ = app_handle.emit("clipboard-removed", id);
            }
        }
    } else if id > 0 {
        let _ = state.repo.increment_use_count(id);

        let should_move_to_top = match move_to_top {
            Some(val) => val,
            None => state
                .settings_repo
                .get("app.move_to_top_after_paste")
                .ok()
                .flatten()
                .map(|v| v == "true")
                .unwrap_or(false),
        };

        if should_move_to_top {
            let _ = state.repo.touch_entry(id, Utc::now().timestamp_millis());
        }
    }

    Ok(())
}



#[cfg(test)]
mod tests {
    use super::{current_entry_file, resolve_paste_with_format};

    const HTML: Option<&str> = Some("<a href=\"https://www.baidu.com\">点击跳转百度</a>");

    #[test]
    fn an_explicit_choice_wins() {
        assert!(!resolve_paste_with_format("点击跳转百度", "rich_text", HTML, Some(false)));
        assert!(resolve_paste_with_format("点击跳转百度", "rich_text", HTML, Some(true)));
    }

    #[test]
    fn no_choice_keeps_the_format_of_rich_text_entries() {
        assert!(resolve_paste_with_format("点击跳转百度", "rich_text", HTML, None));
        assert!(!resolve_paste_with_format(" 点击跳转百度", "rich_text", HTML, None));
        assert!(!resolve_paste_with_format("点击跳转百度", "rich_text", None, Some(true)));
    }

    #[test]
    fn an_entry_without_text_keeps_its_pictures_as_plain_text() {
        let pictures = Some("<img src=\"data:image/png;base64,AAAA\">");
        assert!(resolve_paste_with_format("", "rich_text", pictures, Some(false)));
        // Whitespace is text the user copied (#97); it pastes as it is.
        assert!(!resolve_paste_with_format("  ", "rich_text", pictures, Some(false)));
    }

    #[test]
    fn an_image_saved_under_a_moved_data_folder_pastes_from_this_one() {
        let root = std::env::temp_dir().join(format!("cp-paste-file-{}", uuid::Uuid::new_v4()));
        let data_dir = root.join("com.copypaste");
        std::fs::create_dir_all(data_dir.join("attachments")).unwrap();
        let moved = data_dir.join("attachments").join("img_1.png");
        std::fs::write(&moved, b"png").unwrap();
        let old_path = root
            .join("Application Support")
            .join("com.old.copypaste")
            .join("attachments")
            .join("img_1.png");
        let old_path = old_path.to_string_lossy();
        let moved_path = Some(moved.to_string_lossy().into_owned());

        assert_eq!(current_entry_file("image", &old_path, &data_dir), moved_path);
        assert_eq!(current_entry_file("file", &old_path, &data_dir), moved_path);
        // A file at its stored path keeps the content, and with it the
        // paste's echo hash, as it is.
        let stored = moved.to_string_lossy();
        assert_eq!(current_entry_file("image", &stored, &data_dir), None);
        // Gone from both: the paste reports the missing file as before.
        let gone = old_path.replace("img_1", "img_2");
        assert_eq!(current_entry_file("image", &gone, &data_dir), None);
        // Image data, text and multi-file lists are not looked up.
        assert_eq!(current_entry_file("image", "data:image/png;base64,iVBORw0KGgo=", &data_dir), None);
        assert_eq!(current_entry_file("text", &old_path, &data_dir), None);
        let two_files = format!("{old_path}\n{gone}");
        assert_eq!(current_entry_file("file", &two_files, &data_dir), None);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
