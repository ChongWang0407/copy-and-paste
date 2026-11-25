//! Text in copied images (粘贴为文字, and search), with Apple's Vision
//! framework: offline, free, reads Simplified / Traditional Chinese and
//! English. The approach follows Maccy (MIT, © Alex Rodionov): recognize in
//! the background when the image is copied and keep the result with the entry.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use base64::Engine;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::app_state::{AppDataDir, SessionHistory};
use crate::database::DbState;
use crate::domain::models::{ClipboardEntry, EntryIdentity, EntryVersion};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::services::content_handler::FileVersion;

/// Larger files aren't screenshots of text, and tie up the worker.
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const LANGUAGES: [&str; 3] = ["zh-Hans", "zh-Hant", "en-US"];

/// Recognizes the text in encoded image bytes (PNG, JPEG, HEIC, first GIF frame).
pub fn recognize_text(bytes: &[u8]) -> Result<String, String> {
    autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let options = NSDictionary::<VNImageOption, AnyObject>::new();
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &options,
        );

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let languages: Vec<Retained<NSString>> =
            LANGUAGES.iter().map(|tag| NSString::from_str(tag)).collect();
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
        request.setAutomaticallyDetectsLanguage(true);

        let base: Retained<VNRequest> = Retained::into_super(Retained::into_super(request.clone()));
        handler
            .performRequests_error(&NSArray::from_retained_slice(&[base]))
            .map_err(|error| error.localizedDescription().to_string())?;

        let Some(observations) = request.results() else {
            return Ok(String::new());
        };
        let lines: Vec<String> = observations
            .iter()
            .filter_map(|observation| observation.topCandidates(1).firstObject())
            .map(|candidate| candidate.string().to_string())
            .collect();
        Ok(lines.join("\n"))
    })
}

fn image_bytes(
    content: &str,
    data_dir: &Path,
) -> Option<(Vec<u8>, Option<(PathBuf, FileVersion)>)> {
    if let Some((_, encoded)) = content
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    {
        return Some((
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?,
            None,
        ));
    }
    let path = crate::database::resolve_entry_file_path(content, data_dir)?;
    let version = FileVersion::read(&path)?;
    if version.len > MAX_IMAGE_BYTES {
        return None;
    }
    Some((std::fs::read(&path).ok()?, Some((path, version))))
}

fn store_result(
    app: &AppHandle,
    version: &EntryVersion,
    file: Option<(PathBuf, FileVersion)>,
    text: &str,
) -> Result<bool, String> {
    let file_matches = || {
        file.as_ref()
            .is_none_or(|(path, version)| FileVersion::read(path) == Some(*version))
    };
    let saved = if version.id > 0 {
        let db = app.state::<DbState>();
        let conn = db.conn.lock().unwrap();
        file_matches() && db.repo.store_ocr_for_version(&conn, version, text)?
    } else {
        let session = app.state::<SessionHistory>();
        let mut items = session.0.lock().unwrap();
        if let Some(item) = items
            .iter_mut()
            .find(|item| version.matches(item) && file_matches())
        {
            item.ocr_text = Some(text.to_string());
            true
        } else {
            false
        }
    };
    if saved {
        let _ = app.emit("clipboard-ocr-ready", version.id);
    }
    Ok(saved)
}

struct OcrJob {
    version: EntryVersion,
    data_dir: PathBuf,
}

static OCR_QUEUE: OnceLock<Sender<OcrJob>> = OnceLock::new();

/// One background thread: copying many images at once must not spike the CPU.
fn queue(app: &AppHandle) -> &'static Sender<OcrJob> {
    OCR_QUEUE.get_or_init(|| {
        let (sender, receiver) = channel::<OcrJob>();
        let app = app.clone();
        std::thread::spawn(move || {
            for job in receiver {
                let Some((bytes, file)) = image_bytes(&job.version.content, &job.data_dir) else {
                    continue;
                };
                if let Err(error) = recognize_text(&bytes)
                    .and_then(|text| store_result(&app, &job.version, file, &text))
                {
                    eprintln!("[ocr] entry {} failed: {error}", job.version.id);
                }
            }
        });
        sender
    })
}

pub fn enqueue(app: &AppHandle, entry: &ClipboardEntry) {
    if entry.id == 0 || entry.ocr_text.is_some() {
        return;
    }
    if entry.id > 0
        && matches!(
            app.state::<DbState>().repo.get_ocr_text(entry.id),
            Ok(Some(_))
        )
    {
        return;
    }
    let data_dir = app.state::<AppDataDir>().0.lock().unwrap().clone();
    let _ = queue(app).send(OcrJob {
        version: EntryVersion::from_entry(entry),
        data_dir,
    });
}

/// The entry's text, recognizing it now if the background job hasn't finished.
pub async fn text_for_entry(
    app: &AppHandle,
    state: &DbState,
    session: &SessionHistory,
    id: i64,
    created_at: Option<i64>,
) -> AppResult<(String, EntryIdentity)> {
    let (stored, version) = if id > 0 {
        let conn = state.conn.lock().unwrap();
        let entry = state.repo.get_entry_by_id_with_conn(&conn, id)?;
        let stored = state.repo.get_ocr_text_with_conn(&conn, id)?;
        (stored, entry.as_ref().map(EntryVersion::from_entry))
    } else {
        let items = session.0.lock().unwrap();
        let item = items.iter().find(|item| item.id == id);
        (
            item.and_then(|item| item.ocr_text.clone()),
            item.map(EntryVersion::from_entry),
        )
    };
    let version = version
        .filter(|version| created_at.is_none_or(|expected| version.identity().created_at == expected))
        .ok_or_else(|| AppError::Validation("content_unavailable".into()))?;
    let identity = version.identity();
    if let Some(text) = stored {
        return Ok((text, identity));
    }
    let data_dir = app.state::<AppDataDir>().0.lock().unwrap().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, file) = image_bytes(&version.content, &data_dir)
            .ok_or_else(|| AppError::Validation("file_missing".to_string()))?;
        let text = recognize_text(&bytes).map_err(AppError::Internal)?;
        if store_result(&handle, &version, file, &text)? {
            Ok((text, identity))
        } else {
            Err(AppError::Validation(
                "Image changed during text recognition".into(),
            ))
        }
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[cfg(test)]
mod tests {
    /// Needs a real image: OCR_TEST_IMAGE=/path/to.png cargo test -- --ignored ocr
    #[test]
    #[ignore]
    fn reads_chinese_and_english_from_an_image() {
        let path = std::env::var("OCR_TEST_IMAGE").expect("OCR_TEST_IMAGE");
        let text = super::recognize_text(&std::fs::read(path).unwrap()).unwrap();
        assert!(text.contains("粘贴到位"), "{text}");
        assert!(text.contains("OCR"), "{text}");
    }
}
