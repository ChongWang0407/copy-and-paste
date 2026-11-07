mod pipeline;
pub use pipeline::{start_session_file_sweeper, sweep_session_files};
mod utils;

use crate::app_state::SettingsState;
pub use crate::database::DbState;
use arboard::Clipboard;
use base64::Engine;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "macos")]
fn get_platform_clipboard_html() -> Option<String> {
    crate::infrastructure::macos_api::clipboard::get_clipboard_html()
}


#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn get_platform_clipboard_html() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
const MAX_MACOS_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// Larger clipboard images are not recorded: they would be held, hashed and
/// re-encoded on the listener thread and stall later copies.
const MAX_CAPTURE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPTURE_IMAGE_PIXELS: usize = 60_000_000;








/// A file the app itself wrote to the temp folder to paste an image.

/// With file capture off, a copy of exactly one image file (Finder/Explorer,
/// or a chat app's "copy image", which hands over a temp file) is still an
/// image to record; it used to be dropped entirely. The file's own bytes keep
/// a GIF animated.
fn single_image_file(paths: &[String]) -> Option<(Vec<u8>, String)> {
    let [path] = paths else {
        return None;
    };
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "gif" | "png" | "jpg" | "jpeg" | "webp" | "bmp") {
        return None;
    }
    let size = std::fs::metadata(path).ok().filter(|meta| meta.is_file())?.len();
    if size == 0 || size > MAX_CAPTURE_IMAGE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let extension = crate::database::image_extension_from_magic(&bytes)?.to_string();
    Some((bytes, extension))
}

/// Whether `hash` is what the app itself just put on the clipboard (a paste).
/// It stays true for the whole window, not just the first echo: WPS and Office
/// write the pasted value back once more, with their own formats, after we
/// already saw our write, and that copy must not become a new record.
fn recent_app_write(hash: u64) -> bool {
    let last = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
    let alternate = crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst);
    let timestamp = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (last != 0 && (last == hash || alternate == hash)) && now.saturating_sub(timestamp) < 10
}
fn build_rich_image_fallback_data_url(
    width: usize,
    height: usize,
    rgba_bytes: &[u8],
) -> Option<String> {
    let png_bytes = encode_clipboard_image_to_png(width, height, rgba_bytes)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    Some(format!("data:image/png;base64,{}", b64))
}

fn encode_clipboard_image_to_png(
    width: usize,
    height: usize,
    rgba_bytes: &[u8],
) -> Option<Vec<u8>> {
    let img_buf = image::RgbaImage::from_raw(width as u32, height as u32, rgba_bytes.to_vec())?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    img_buf
        .write_to(&mut cursor, image::ImageFormat::Png)
        .ok()?;
    Some(bytes)
}


pub fn start_clipboard_monitor(app_handle: AppHandle) {
    use std::sync::{Arc, Mutex};

    // Initial state for deduplication and self-copy detection.
    let mut last_image_hash = 0u64;

    if let Ok(mut cb) = Clipboard::new() {
        if let Ok(img) = cb.get_image() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            img.bytes.hash(&mut hasher);
            last_image_hash = hasher.finish();
        }
    }

    struct MonitorState {
        last_image_hash: u64,
        last_image_time_ms: u64,
        last_content_hash: u64,
        last_process_time_ms: u64,
        last_sequence: i64,
    }

    impl MonitorState {
        /// The same image event again within a few seconds (apps often publish
        /// one copy as several clipboard updates). Only briefly: copying the
        /// same image again later, or after deleting its entry, must record it.
        fn is_repeat_image(&self, hash: u64, now_ms: u64) -> bool {
            hash == self.last_image_hash && now_ms.saturating_sub(self.last_image_time_ms) < 10_000
        }

        fn remember_image(&mut self, hash: u64, now_ms: u64) {
            self.last_image_hash = hash;
            self.last_image_time_ms = now_ms;
        }
    }

    let state = Arc::new(Mutex::new(MonitorState {
        last_image_hash,
        last_image_time_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        last_content_hash: 0,
        last_process_time_ms: 0,
        last_sequence: -1,
    }));

    let app_clone = app_handle.clone();
    let state_lock = state.clone();

    // Start the native clipboard listener
    crate::services::clipboard_listener::listen_clipboard(Arc::new(move || {
        let app = app_clone.clone();
        // A panic in an earlier capture must not stop recording for good.
        let mut monitor_state = state_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        // 1. Check for pause
        if crate::CLIPBOARD_MONITOR_PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        // The app put the user's previous clipboard back (transient paste).
        #[cfg(target_os = "macos")]
        if crate::infrastructure::macos_api::clipboard::is_own_change() {
            return;
        }

        // Nothing was written since the last capture: an event that queued up
        // while a slow capture ran. (A 2 s same-content window used to be the
        // only guard, so a capture longer than that was processed twice.)
        #[cfg(target_os = "macos")]
        let sequence = crate::infrastructure::macos_api::clipboard::change_count() as i64;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if sequence == monitor_state.last_sequence {
                return;
            }
            monitor_state.last_sequence = sequence;
        }

        // The app is writing, or just wrote, the clipboard for a paste.


        // Snapshot file URLs immediately. Finder can replace the pasteboard
        // again while we wait for richer text formats, making an earlier file
        // event otherwise observe a later file and lose the intermediate one.
        #[cfg(target_os = "macos")]
        let clipboard_files_snapshot =
            crate::infrastructure::macos_api::clipboard::get_clipboard_files();

        // Give source apps (especially Excel) a brief moment to finish writing
        // non-file payloads. File URLs are already complete in the event snapshot.
        #[cfg(target_os = "macos")]
        if clipboard_files_snapshot.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        std::thread::sleep(std::time::Duration::from_millis(20));

        // Passwords copied from password managers are marked private; never record them.
        // Checked after the settle delay so markers written alongside the text are visible.
        #[cfg(target_os = "macos")]
        if crate::infrastructure::macos_api::clipboard::is_clipboard_marked_private() {
            return;
        }

        // Initialize clipboard for this thread
        #[cfg(target_os = "macos")]
        let mut clipboard = Clipboard::new().ok();
        #[cfg(target_os = "macos")]
        let text_from_clipboard = crate::infrastructure::macos_api::clipboard::get_clipboard_text();

        // The image in its original encoding (GIF, PNG…); see
        // get_clipboard_image_bytes. Not read for file copies.
        #[cfg(target_os = "macos")]
        let macos_raw_image = if clipboard_files_snapshot.is_none() {
            crate::infrastructure::macos_api::clipboard::get_clipboard_image_bytes()
                .filter(|(bytes, _)| bytes.len() <= MAX_CAPTURE_IMAGE_BYTES)
        } else {
            None
        };
        // Decoding the TIFF only when nothing better is there: it flattens GIFs
        // and costs a full decode, and for file copies it's just the file icon.
        #[cfg(target_os = "macos")]
        let clipboard_image = if clipboard_files_snapshot.is_none() && macos_raw_image.is_none() {
            clipboard
                .as_mut()
                .and_then(|cb| cb.get_image().ok())
                .filter(|image| image.width * image.height <= MAX_CAPTURE_IMAGE_PIXELS)
        } else {
            None
        };

        // Calculate hash of current clipboard content
        let current_content_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();

            // Hash text content if available
            #[cfg(target_os = "macos")]
            {
                if let Some(file_paths) = clipboard_files_snapshot.as_ref() {
                    file_paths.hash(&mut hasher);
                }
                if let Some(text) = &text_from_clipboard {
                    if text.len() > MAX_MACOS_TEXT_BYTES {
                        "__TEXT_TOO_LARGE__".hash(&mut hasher);
                    } else {
                        text.hash(&mut hasher);
                    }
                }
                if let Some(html) = get_platform_clipboard_html() {
                    if html.len() > MAX_MACOS_TEXT_BYTES {
                        "__HTML_TOO_LARGE__".hash(&mut hasher);
                    } else {
                        html.hash(&mut hasher);
                    }
                } else if crate::infrastructure::macos_api::clipboard::get_clipboard_rtf().is_some()
                {
                    "__RTF__".hash(&mut hasher);
                }
            }

            if let Some(image) = &clipboard_image {
                image.bytes.hash(&mut hasher);
            }
            #[cfg(target_os = "macos")]
            if let Some((bytes, _)) = &macos_raw_image {
                bytes.hash(&mut hasher);
            }

            hasher.finish()
        };

        // The clipboard image as a data URL for rich-text fallbacks: its original
        // bytes when we have them (a GIF stays animated), else the bitmap as PNG.
        #[cfg(target_os = "macos")]
        let has_clipboard_image = clipboard_image.is_some() || macos_raw_image.is_some();
        let rich_image_fallback_url = || -> Option<String> {
            #[cfg(target_os = "macos")]
            if let Some((bytes, extension)) = macos_raw_image.as_ref() {
                let mime = if *extension == "jpg" { "jpeg" } else { extension };
                return Some(format!(
                    "data:image/{};base64,{}",
                    mime,
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ));
            }
            clipboard_image.as_ref().and_then(|image| {
                build_rich_image_fallback_data_url(image.width, image.height, &image.bytes)
            })
        };

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        // One OS copy may publish several format-change events. Suppress that burst, but
        // still allow the user to deliberately copy the same value again later.
        if current_content_hash == monitor_state.last_content_hash
            && current_content_hash != 0
            && now_ms.saturating_sub(monitor_state.last_process_time_ms) < 2000
        {
            return;
        }

        monitor_state.last_content_hash = current_content_hash;
        monitor_state.last_process_time_ms = now_ms;

        let mut handled = false;

        // --- Core processing logic (same as before) ---

        // 1. Check Files (macOS)
        #[cfg(target_os = "macos")]
        {
            if let Some(file_paths) = clipboard_files_snapshot {
                let file_content = file_paths.join("\n");
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                use std::hash::{Hash, Hasher};
                utils::normalize_clipboard_line_endings(&file_content).hash(&mut hasher);
                let current_hash = hasher.finish();

                let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
                let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                if (last_app_hash != 0 && current_hash == last_app_hash)
                    && (now_secs - last_app_time) < 10
                {
                    return;
                }

                let settings = app.state::<SettingsState>();
                if settings.capture_files.load(Ordering::Relaxed) {
                    process_new_entry(&app, ClipboardData::Files(file_paths), None);
                } else if let Some((bytes, extension)) = single_image_file(&file_paths) {
                    process_new_entry(&app, ClipboardData::Image { bytes, extension }, None);
                }
                // Whether we captured or skipped, we MUST mark as handled so the file
                // icon (image) and filename (text) are NOT captured.
                return;
            }
        }

        // Explorer publishes copied files as CF_HDROP. Handle this before
        // arboard's text/image fallbacks so a file copy is not mistaken for
        // its path text or thumbnail.

        #[cfg(target_os = "macos")]
        let clipboard_html_snapshot = {
            let settings = app.state::<SettingsState>();
            if settings.capture_rich_text.load(Ordering::Relaxed) {
                let mut html = get_platform_clipboard_html();
                if html
                    .as_ref()
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
                {
                    if let Some(rtf) =
                        crate::infrastructure::macos_api::clipboard::get_clipboard_rtf()
                    {
                        html =
                            crate::infrastructure::macos_api::clipboard::convert_rtf_to_html(&rtf);
                    }
                }
                html
            } else {
                None
            }
        };

        #[cfg(target_os = "macos")]
        let clipboard_rtf_snapshot =
            crate::infrastructure::macos_api::clipboard::get_clipboard_rtf();

        // Office/WPS publish HTML, RTF and private formats in several clipboard updates.
        // Probe those formats before the bitmap fallback, including HTML-only payloads.

        // Preserve native encoded image bytes. In particular, do not flatten animated GIFs.

        // 2. Check Image
        if !handled {
            let settings = app.state::<SettingsState>();
            let rich_text_enabled = settings.capture_rich_text.load(Ordering::Relaxed);
            // HTML that only wraps the copied picture (browsers' "Copy image")
            // isn't rich text: record the image itself.
            #[cfg(target_os = "macos")]
            let has_rich_html = rich_text_enabled
                && clipboard_html_snapshot
                    .as_ref()
                    .map(|html| {
                        !html.trim().is_empty()
                            && !(has_clipboard_image && utils::html_is_image_only(html))
                    })
                    .unwrap_or(false);

            // Rich text wins over image when rich HTML exists; image remains fallback for pure image content.
            if !has_rich_html {
                #[cfg(target_os = "macos")]
                if !handled {
                    if let Some((bytes, extension)) = macos_raw_image.as_ref() {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        use std::hash::{Hash, Hasher};
                        bytes.hash(&mut hasher);
                        let hash = hasher.finish();
                        if !monitor_state.is_repeat_image(hash, now_ms) {
                            if !recent_app_write(hash) {
                                process_new_entry(
                                    &app,
                                    ClipboardData::Image {
                                        bytes: bytes.clone(),
                                        extension: extension.to_string(),
                                    },
                                    None,
                                );
                                handled = true;
                            }
                            monitor_state.remember_image(hash, now_ms);
                        }
                    }
                }
                if !handled {
                    if let Some(image) = clipboard_image.as_ref() {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        use std::hash::{Hash, Hasher};
                        image.bytes.hash(&mut hasher);
                        let hash = hasher.finish();

                        if !monitor_state.is_repeat_image(hash, now_ms) {
                            if recent_app_write(hash) {
                                // Our own paste coming back.
                            } else if let Some(png_bytes) = encode_clipboard_image_to_png(
                                image.width,
                                image.height,
                                &image.bytes,
                            ) {
                                process_new_entry(
                                    &app,
                                    ClipboardData::Image {
                                        bytes: png_bytes,
                                        extension: "png".to_string(),
                                    },
                                    None,
                                );
                                handled = true;
                            }
                            monitor_state.remember_image(hash, now_ms);
                        }
                    }

                }
            }
        }

        if !handled {
            #[cfg(target_os = "macos")]
            {
                let settings = app.state::<SettingsState>();
                let rich_text_enabled = settings.capture_rich_text.load(Ordering::Relaxed);
                let active_app =
                    crate::infrastructure::macos_api::window::get_active_app_snapshot();

                let resolved = utils::resolve_clipboard_text_capture(
                    text_from_clipboard.as_deref(),
                    clipboard_html_snapshot.as_deref(),
                    None,
                    rich_text_enabled,
                    &active_app.app_name,
                    active_app.process_path.as_deref(),
                );

                if let Some(payload) = resolved {
                    let text = payload.text;
                    if text.len() > MAX_MACOS_TEXT_BYTES {
                        eprintln!(
                            ">>> [CLIPBOARD] Skip capture: text exceeds {} bytes",
                            MAX_MACOS_TEXT_BYTES
                        );
                        return;
                    }

                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    use std::hash::{Hash, Hasher};
                    utils::normalize_clipboard_line_endings(&text).hash(&mut hasher);
                    let current_hash = hasher.finish();

                    let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
                    let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
                    let now_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if (last_app_hash != 0
                        && (current_hash == last_app_hash
                            || current_hash == crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst)))
                        && (now_secs - last_app_time) < 10
                    {
                        return;
                    }

                    if rich_text_enabled {
                        if let Some(mut html) = payload.html {
                            if !html.trim().is_empty() {
                                if has_clipboard_image {
                                    if utils::should_attach_rich_image_fallback_on_capture(
                                        &active_app.app_name,
                                        active_app.process_path.as_deref(),
                                    ) {
                                        if let Some(data_url) = rich_image_fallback_url() {
                                            html =
                                                utils::attach_rich_image_fallback(&html, &data_url);
                                        }
                                    }
                                }

                                if let Some(rtf_bytes) = clipboard_rtf_snapshot.clone() {
                                    html = utils::attach_rich_named_formats(
                                        &html,
                                        &[crate::infrastructure::windows_api::win_clipboard::NamedClipboardFormat {
                                            name: "Rich Text Format".to_string(),
                                            data: rtf_bytes,
                                        }],
                                    );
                                }

                                process_new_entry(
                                    &app,
                                    ClipboardData::RichText {
                                        text: text.clone(),
                                        html,
                                    },
                                    None,
                                );
                                handled = true;
                            }
                        }
                    }

                    if !handled {
                        if last_app_hash != 0 {
                            crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                        }
                        process_new_entry(&app, ClipboardData::Text(text), None);
                    }
                }
            }

        }
    }));
}



pub fn capture_preserved_named_formats_from_clipboard(
    _source_snapshot: Option<&crate::infrastructure::windows_api::window_tracker::ActiveAppInfo>,
) -> Vec<crate::infrastructure::windows_api::win_clipboard::NamedClipboardFormat> {
    vec![]
}


pub fn clipboard_image_fallback_data_url() -> Option<String> {
    None
}

pub use pipeline::{ClipboardData, ClipboardPipeline, PipelineContext};
pub use utils::{
    app_likely_word_processor, attach_rich_image_fallback, attach_rich_named_formats,
    build_clipboard_text_fingerprint, build_entry_preview, build_exact_paste_html,
    derive_rich_text_content, detect_content_type, embed_local_images, encode_cf_html,
    entry_matches_search, entry_searchable_text, extract_animated_image_data_url_from_html,
    extract_animated_image_data_url_from_text, extract_first_image_data_url_from_html,
    html_has_renderable_rich_body, normalize_clipboard_line_endings,
    normalize_plain_text_for_clipboard_paste, parse_cf_html, plain_text_from_tabular_html,
    plain_text_requires_exact_paste, repair_html_fragment, rich_paste_needs_exact_text,
    rtf_bytes_from_named_formats,
    sanitize_tabular_html_for_paste, should_attach_rich_image_fallback_on_capture,
    split_rich_html_and_image_fallback,
    split_rich_html_and_named_formats, truncate_html_for_preview,
};

const MAX_PIPELINE_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIPELINE_HTML_BYTES: usize = 16 * 1024 * 1024;

pub fn process_new_entry(
    app_handle: &AppHandle,
    data: ClipboardData,
    source_override: Option<String>,
) {
    let payload_too_large = match &data {
        ClipboardData::Text(text) => text.len() > MAX_PIPELINE_TEXT_BYTES,
        ClipboardData::RichText { text, html } => {
            text.len() > MAX_PIPELINE_TEXT_BYTES || html.len() > MAX_PIPELINE_HTML_BYTES
        }
        _ => false,
    };

    if payload_too_large {
        eprintln!(
            ">>> [CLIPBOARD] Skip pipeline: payload exceeds text/html limits (text:{} bytes, html:{} bytes)",
            MAX_PIPELINE_TEXT_BYTES,
            MAX_PIPELINE_HTML_BYTES
        );
        return;
    }

    let mut ctx = PipelineContext::new(app_handle.clone(), data);
    if let Some(source) = source_override {
        ctx.source_app = source;
        ctx.source_app_path = None;
    }

    let pipeline = ClipboardPipeline::new();
    pipeline.execute(&mut ctx);
}


#[cfg(all(test, target_os = "windows"))]
mod named_format_tests {
    use super::is_ignored_named_clipboard_format;

    #[test]
    fn ole_link_formats_are_not_read() {
        for name in ["Link Source", "ObjectLink", "OwnerLink", "Native", "Embed Source", "Link Source Descriptor"] {
            assert!(is_ignored_named_clipboard_format(name), "{name}");
        }
        for name in ["Rich Text Format", "XML Spreadsheet", "Biff12", "Csv", "Kingsoft Data Descriptor"] {
            assert!(!is_ignored_named_clipboard_format(name), "{name}");
        }
    }
}
