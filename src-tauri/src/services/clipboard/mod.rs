mod pipeline;
pub use pipeline::{start_session_file_sweeper, sweep_session_files};
mod utils;

use crate::app_state::SettingsState;
pub use crate::database::DbState;
#[cfg(target_os = "macos")]
use crate::infrastructure::macos_api::{clipboard as native, window::get_active_app_snapshot};
use base64::Engine;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const MAX_CAPTURE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPTURE_IMAGE_PIXELS: usize = 60_000_000;

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

#[cfg(target_os = "macos")]
struct CaptureSnapshot {
    files: Option<Vec<String>>,
    text: Option<String>,
    html: Option<String>,
    rtf: Option<Vec<u8>>,
    image: Option<(Vec<u8>, &'static str)>,
    source_app: String,
    source_app_path: Option<String>,
    timestamp: i64,
    capture_files: bool,
    capture_rich_text: bool,
    persistent: bool,
}

#[cfg(target_os = "macos")]
impl CaptureSnapshot {
    fn read(app: &AppHandle, count: isize) -> Result<Option<Self>, ()> {
        if native::change_count() != count {
            return Err(());
        }
        if native::is_own_change(count) || native::is_clipboard_marked_private() {
            return if native::change_count() == count {
                Ok(None)
            } else {
                Err(())
            };
        }

        let settings = app.state::<SettingsState>();
        let active_app = get_active_app_snapshot();
        let capture_files = settings.capture_files.load(Ordering::Relaxed);
        let capture_rich_text = settings.capture_rich_text.load(Ordering::Relaxed);
        let persistent = settings.persistent.load(Ordering::Relaxed);
        let files = native::get_clipboard_files();
        let mut snapshot = Self {
            files,
            text: None,
            html: None,
            rtf: None,
            image: None,
            source_app: active_app.app_name,
            source_app_path: active_app.process_path,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            capture_files,
            capture_rich_text,
            persistent,
        };
        if snapshot.files.is_none() {
            snapshot.text =
                native::get_clipboard_text().filter(|s| s.len() <= MAX_PIPELINE_TEXT_BYTES);
            if capture_rich_text {
                snapshot.html =
                    native::get_clipboard_html().filter(|s| s.len() <= MAX_PIPELINE_HTML_BYTES);
                snapshot.rtf = native::get_clipboard_rtf(MAX_PIPELINE_TEXT_BYTES);
            }
            snapshot.image = native::get_clipboard_image_bytes(MAX_CAPTURE_IMAGE_BYTES);
        }
        if native::is_clipboard_marked_private() {
            return if native::change_count() == count {
                Ok(None)
            } else {
                Err(())
            };
        }
        if native::change_count() != count {
            return Err(());
        }
        Ok(Some(snapshot))
    }

    fn into_data(&mut self) -> Option<ClipboardData> {
        if let Some(files) = self.files.take() {
            if self.capture_files {
                return Some(ClipboardData::Files(files));
            }
            let (bytes, extension) = single_image_file(&files)?;
            return Some(ClipboardData::Image { bytes, extension });
        }

        let image = self.image.take().and_then(|(bytes, extension)| {
            if extension == "tiff" {
                native::encode_clipboard_tiff(&bytes, MAX_CAPTURE_IMAGE_PIXELS)
                    .map(|png| (png, "png"))
            } else {
                Some((bytes, extension))
            }
        });
        let mut html = self.html.take();
        if html.as_ref().is_none_or(|html| html.trim().is_empty()) {
            if let Some(rtf) = &self.rtf {
                html = native::convert_rtf_to_html(rtf);
            }
        }
        let has_rich_html = html.as_ref().is_some_and(|html| {
            !html.trim().is_empty() && !(image.is_some() && utils::html_is_image_only(html))
        });
        if !has_rich_html {
            if let Some((bytes, extension)) = image {
                return Some(ClipboardData::Image {
                    bytes,
                    extension: extension.to_string(),
                });
            }
        }
        let payload = utils::resolve_clipboard_text_capture(
            self.text.as_deref(),
            html.as_deref(),
            None,
            self.capture_rich_text,
            &self.source_app,
            self.source_app_path.as_deref(),
        )?;
        let Some(mut html) = payload.html.filter(|html| !html.trim().is_empty()) else {
            return Some(ClipboardData::Text(payload.text));
        };

        if let Some((bytes, extension)) = image {
            if utils::should_attach_rich_image_fallback_on_capture(
                &self.source_app,
                self.source_app_path.as_deref(),
            ) {
                let mime = if extension == "jpg" {
                    "jpeg"
                } else {
                    extension
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                html = utils::attach_rich_image_fallback(
                    &html,
                    &format!("data:image/{mime};base64,{encoded}"),
                );
            }
        }
        if let Some(rtf) = self.rtf.take() {
            html = utils::attach_rich_named_formats(
                &html,
                &[
                    crate::infrastructure::windows_api::win_clipboard::NamedClipboardFormat {
                        name: "Rich Text Format".to_string(),
                        data: rtf,
                    },
                ],
            );
        }
        Some(ClipboardData::RichText {
            text: payload.text,
            html,
        })
    }
}

pub fn start_clipboard_monitor(app: AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let snapshot_app = app.clone();
        let mut last_timestamp = 0;
        let pipeline = ClipboardPipeline::new();
        crate::services::clipboard_listener::listen_clipboard(
            move |count| {
                let mut snapshot = CaptureSnapshot::read(&snapshot_app, count)?;
                if let Some(snapshot) = &mut snapshot {
                    snapshot.timestamp = snapshot.timestamp.max(last_timestamp + 1);
                    last_timestamp = snapshot.timestamp;
                }
                Ok(snapshot)
            },
            move |mut snapshot| {
                let Some(data) = snapshot.into_data().filter(payload_within_limits) else {
                    return;
                };
                let mut context = PipelineContext::new(
                    app.clone(),
                    data,
                    snapshot.source_app,
                    snapshot.source_app_path,
                    snapshot.timestamp,
                    snapshot.persistent,
                );
                pipeline.execute(&mut context);
            },
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
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
pub(crate) use utils::truncate_entry_for_ui;
pub use utils::{
    app_likely_word_processor, attach_rich_image_fallback, attach_rich_named_formats,
    build_clipboard_text_fingerprint, build_entry_preview, build_exact_paste_html,
    derive_rich_text_content, detect_content_type, embed_local_images, encode_cf_html,
    entry_matches_search, extract_animated_image_data_url_from_html,
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

fn payload_within_limits(data: &ClipboardData) -> bool {
    let within_limits = match data {
        ClipboardData::Text(text) => text.len() <= MAX_PIPELINE_TEXT_BYTES,
        ClipboardData::RichText { text, html } => {
            text.len() <= MAX_PIPELINE_TEXT_BYTES && html.len() <= MAX_PIPELINE_HTML_BYTES
        }
        _ => true,
    };
    if !within_limits {
        eprintln!(
            ">>> [CLIPBOARD] Skip pipeline: payload exceeds text/html limits (text:{} bytes, html:{} bytes)",
            MAX_PIPELINE_TEXT_BYTES,
            MAX_PIPELINE_HTML_BYTES
        );
    }
    within_limits
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
