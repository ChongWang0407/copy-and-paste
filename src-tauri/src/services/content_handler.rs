// Content handler module for opening various content types.
use crate::database::DbState;
use crate::domain::models::{ClipboardEntry, EntryVersion};
use crate::error::AppError;
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;
use std::io::Read;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex, Once};
use std::time::{Duration, Instant, SystemTime};
use tauri::{Emitter, Manager, State};

#[derive(Clone)]
struct WatchedEntryInfo {
    app: tauri::AppHandle,
    version: EntryVersion,
    path: std::path::PathBuf,
    generation: u64,
    last_activity: Instant,
    last_file: Option<FileVersion>,
    content_type: String,
    keep_path: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileVersion {
    modified: SystemTime,
    pub len: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileVersion {
    pub fn read(path: &std::path::Path) -> Option<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

static WATCHED_PREVIEW_ENTRIES: LazyLock<Mutex<HashMap<i64, WatchedEntryInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PREVIEW_WATCH_READY: Condvar = Condvar::new();

#[tauri::command]
pub async fn open_content(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    id: i64,
    mut content: String,
    content_type: String,
) -> Result<(), AppError> {
    let mut html_content = None;
    let mut original_entry = None;
    // 0. Resolve full content if ID is provided and content is placeholder/truncated
    if id != 0 {
        if id > 0 {
            // Fetch from Database
            if let Ok(Some(entry)) = state.repo.get_entry_by_id(id) {
                content = entry.content.clone();
                html_content = entry.html_content.clone();
                original_entry = Some(entry);
            }
        } else {
            // Fetch from Session
            let session = app_handle.state::<crate::app_state::SessionHistory>();
            let session_items = session.0.lock().unwrap();
            if let Some(item) = session_items.iter().find(|i| i.id == id) {
                content = item.content.clone();
                html_content = item.html_content.clone();
                original_entry = Some(item.clone());
            }
        }
    }

    // Increment use count if ID is provided
    if id > 0 {
        let _ = state.repo.increment_use_count(id);
    }

    let app_path = get_app_path_for_content_type(&state, &content_type)?;
    let filename = if id != 0 {
        format!("CopyPaste_Clip_entry_{}_{}", id, uuid::Uuid::new_v4())
    } else {
        static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);
        let count = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "CopyPaste_Clip_temp_{}_{:x}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            count
        )
    };

    // Handle links/URLs
    if content_type == "link" || content_type == "url" {
        return handle_url_content(&app_path, &content).await;
    }

    // Several copied files open together, as a Finder or Explorer selection does.
    // Only the first one used to open.
    if content_type == "file" {
        let paths = existing_file_paths(&content);
        if paths.len() > 1 {
            launch_files_with_app(&app_path, &paths).await?;
            return Ok(());
        }
    }

    // Check if content points to existing file(s)
    let mut entry_file = None;
    if is_file_type(&content_type) && !content.starts_with("data:image") {
        if let Some(path) = entry_file_path(&app_handle, &content) {
            let data_dir = app_data_dir(&app_handle);
            entry_file = Some(file_to_open(&content_type, path, data_dir.as_deref())?);
        } else if names_missing_file(&content_type, &content) {
            return Err(AppError::Validation(FILE_MISSING.into()));
        }
    }

    // Otherwise write the entry's content to a temp file.
    let (temp_path, use_direct_path) = match entry_file {
        Some(file) => (file.path, file.is_original),
        None => (
            create_temp_file(
                &content,
                &html_content,
                &content_type,
                &filename,
                std::env::temp_dir(),
            )?,
            false,
        ),
    };

    let path_str = temp_path.to_str().unwrap().to_string();
    let file_path_clone = temp_path.clone();
    let initial_file = FileVersion::read(&file_path_clone);

    // Launch the file with appropriate application
    launch_file_with_app(
        &app_path,
        &temp_path,
        &path_str,
        &content_type,
        use_direct_path,
    )
    .await?;

    // Watch temp files and external originals so list previews stay in sync (#117).
    if let Some(entry) = original_entry.filter(|entry| entry.content_type != "rich_text") {
        register_watch(
            app_handle,
            &entry,
            file_path_clone,
            use_direct_path,
            initial_file,
        );
    }

    Ok(())
}

/// The card's right-click 分享: the system share menu, as Finder's 共享… shows it,
/// anchored to the card (`anchor`, page pixels).
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn share_content(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    id: i64,
    mut content: String,
    content_type: String,
    anchor: crate::infrastructure::macos_api::share::AnchorRect,
) -> Result<(), AppError> {
    use crate::infrastructure::macos_api::share::{show_share_picker, ShareItem};

    if id > 0 {
        if let Ok(Some((full_content, _, _))) = state.repo.get_entry_content_with_html(id) {
            content = full_content;
        }
    } else if id < 0 {
        let session = app_handle.state::<crate::app_state::SessionHistory>();
        let session_items = session.0.lock().unwrap();
        if let Some(item) = session_items.iter().find(|i| i.id == id) {
            content = item.content.clone();
        }
    }

    let item = match content_type.as_str() {
        "file" | "video" => {
            let mut paths = existing_file_paths(&content);
            if paths.is_empty() {
                paths.extend(entry_file_path(&app_handle, &content));
            }
            if paths.is_empty() {
                return Err(AppError::Validation(FILE_MISSING.into()));
            }
            ShareItem::Files(paths)
        }
        "image" => {
            let path = if content.starts_with("data:image") {
                None
            } else {
                entry_file_path(&app_handle, &content)
            };
            let path = match path {
                Some(path) => path,
                None if names_missing_file("image", &content) => {
                    return Err(AppError::Validation(FILE_MISSING.into()));
                }
                // Image data, not a file: share a temporary picture file.
                None => create_temp_file(
                    &content,
                    &None,
                    "image",
                    &format!("CopyPaste_Share_{}", id.unsigned_abs()),
                    std::env::temp_dir(),
                )?,
            };
            ShareItem::Files(vec![path])
        }
        "link" | "url" => ShareItem::Url(content),
        _ => ShareItem::Text(content),
    };

    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| AppError::Internal("Main window not found".into()))?;
    show_share_picker(&window, item, anchor).map_err(AppError::Internal)
}

/// Watch an on-disk original and refresh its preview when it changes.
pub fn register_file_preview_watch(
    app: tauri::AppHandle,
    entry: &ClipboardEntry,
    path: std::path::PathBuf,
    keep_path: bool,
) {
    let initial = FileVersion::read(&path);
    register_watch(app, entry, path, keep_path, initial);
}

fn register_watch(
    app: tauri::AppHandle,
    entry: &ClipboardEntry,
    path: std::path::PathBuf,
    keep_path: bool,
    last_file: Option<FileVersion>,
) {
    static NEXT_WATCH: AtomicU64 = AtomicU64::new(1);
    static WORKER: Once = Once::new();
    let mut watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
    if let Some(existing) = watched.get_mut(&entry.id) {
        if existing.version.matches(entry)
            && (existing.path == path || (keep_path && !existing.keep_path))
        {
            existing.last_activity = Instant::now();
            return;
        }
    }
    watched.insert(
        entry.id,
        WatchedEntryInfo {
            app,
            version: EntryVersion::from_entry(entry),
            path,
            generation: NEXT_WATCH.fetch_add(1, Ordering::Relaxed),
            last_activity: Instant::now(),
            last_file,
            content_type: entry.content_type.clone(),
            keep_path,
        },
    );
    PREVIEW_WATCH_READY.notify_one();
    WORKER.call_once(|| {
        std::thread::spawn(watch_files);
    });
}

fn watch_files() {
    const RETENTION: Duration = Duration::from_secs(3600);
    loop {
        let watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
        drop(
            PREVIEW_WATCH_READY
                .wait_while(watched, |watches| watches.is_empty())
                .unwrap(),
        );
        std::thread::sleep(Duration::from_secs(2));
        let watches: Vec<_> = {
            let mut watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
            watched.retain(|_, watch| watch.last_activity.elapsed() < RETENTION);
            watched.values().cloned().collect()
        };
        for watch in watches {
            let file = FileVersion::read(&watch.path);
            if file == watch.last_file {
                continue;
            }
            let updated = if watch.keep_path {
                touch_external_file_preview(&watch, file.is_some())
            } else if file.is_some() {
                let Some((content, preview)) = read_changed_file(&watch.path, &watch.content_type)
                else {
                    continue;
                };
                update_database_with_changes(&watch, content, &preview)
            } else {
                continue; // An editor may be replacing the file by rename.
            };
            let updated = match updated {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!(">>> [CLIPBOARD] Could not refresh file preview: {error}");
                    continue;
                }
            };
            let mut watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
            let Some(current) = watched
                .get_mut(&watch.version.id)
                .filter(|item| item.generation == watch.generation)
            else {
                continue;
            };
            if let Some(entry) = updated {
                current.last_file = file;
                current.last_activity = Instant::now();
                current.version = EntryVersion::from_entry(&entry);
                let _ = watch.app.emit(
                    "clipboard-updated",
                    crate::services::clipboard::truncate_entry_for_ui(entry.clone()),
                );
                if entry.content_type == "image" && file.is_some() {
                    crate::services::ocr::enqueue(&watch.app, &entry);
                }
            } else if watch.keep_path || file.is_some() {
                watched.remove(&watch.version.id);
            }
        }
    }
}

fn get_app_path_for_content_type(
    state: &State<'_, DbState>,
    content_type: &str,
) -> Result<Option<String>, AppError> {
    let setting_key = format!("app.{}", content_type);
    let mut val = state
        .settings_repo
        .get(&setting_key)
        .map_err(AppError::from)?
        .filter(|s| !s.trim().is_empty());

    // Fallback: If 'code' app is not set, use 'text' app
    if val.is_none() && content_type == "code" {
        val = state
            .settings_repo
            .get("app.text")
            .map_err(AppError::from)?
            .filter(|s| !s.trim().is_empty());
    }
    Ok(val)
}

async fn handle_url_content(app_path: &Option<String>, content: &str) -> Result<(), AppError> {
    if let Some(app) = app_path {

        {
            launch_target_with_app(app, content).await?;
        }
    } else {
        launch_with_default_app(content, "url", false)?;
    }
    Ok(())
}


fn is_file_type(content_type: &str) -> bool {
    matches!(content_type, "file" | "video" | "image")
}

pub(crate) fn existing_file_path_from_content(content: &str) -> Option<std::path::PathBuf> {
    get_existing_file_path(content)
}

/// Error detail the frontend shows as "the file was moved or deleted".
const FILE_MISSING: &str = "file_missing";

/// The file an entry names (its first line), found where the history list
/// finds it, including attachments of a data folder that has since moved.
fn entry_file_path(app_handle: &tauri::AppHandle, content: &str) -> Option<std::path::PathBuf> {
    let first_line = content.lines().next().unwrap_or(content);
    match app_data_dir(app_handle) {
        Some(data_dir) => crate::database::resolve_entry_file_path(first_line, &data_dir),
        None => get_existing_file_path(content),
    }
}

/// No file was found for the entry: a file or video entry, or an image saved
/// as a file path rather than as image data. Such a path must not be decoded
/// as image data ("Invalid symbol 32, offset 33" for the space in
/// "Application Support"), and a file entry opened a text file of its path.
fn names_missing_file(content_type: &str, content: &str) -> bool {
    content_type != "image" || general_purpose::STANDARD.decode(content.trim()).is_err()
}

fn get_existing_file_path(content: &str) -> Option<std::path::PathBuf> {
    let first_line = content.lines().next().unwrap_or(content).trim();
    let path = std::path::Path::new(first_line);
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// Every line of a file entry that names a file still on disk.
fn existing_file_paths(content: &str) -> Vec<std::path::PathBuf> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

/// Opens each file; one that fails does not stop the rest.
async fn launch_files_with_app(
    app_path: &Option<String>,
    paths: &[std::path::PathBuf],
) -> Result<(), AppError> {
    let mut first_error = None;
    for path in paths {
        let path_str = path.to_string_lossy();
        if let Err(e) = launch_file_with_app(app_path, path, &path_str, "file", true).await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn create_temp_file(
    content: &str,
    html_content: &Option<String>,
    content_type: &str,
    filename: &str,
    mut temp_path: std::path::PathBuf,
) -> Result<std::path::PathBuf, AppError> {
    match content_type {
        "image" => {
            let is_gif = content.contains("image/gif");
            let extension = if is_gif { "gif" } else { "png" };
            temp_path.push(format!("{}.{}", filename, extension));

            let b64_data = if content.starts_with("data:image") {
                content.split(',').nth(1).unwrap_or(content)
            } else {
                content
            };
            let bytes = general_purpose::STANDARD
                .decode(b64_data)
                .map_err(|e| e.to_string())?;

            if is_gif {
                // For GIFs, write raw bytes directly to preserve animation
                std::fs::write(&temp_path, &bytes).map_err(AppError::from)?;
            } else {
                // For other images, use image crate to ensure standard peak format
                let img = image::load_from_memory(&bytes).map_err(AppError::from)?;
                img.save(&temp_path).map_err(AppError::from)?;
            }
        }
        "rich_text" => {
            temp_path.push(format!("{}.html", filename));
            let mut file = std::fs::File::create(&temp_path).map_err(AppError::from)?;
            use std::io::Write;
            let final_content = html_content.as_deref().unwrap_or(content);
            file.write_all(final_content.as_bytes())
                .map_err(|e| AppError::IO(e.to_string()))?;
        }
        "url" | "link" => {
            temp_path.push(format!("{}.html", filename));
            let mut file = std::fs::File::create(&temp_path).map_err(AppError::from)?;
            use std::io::Write;
            file.write_all(content.as_bytes())
                .map_err(|e| AppError::IO(e.to_string()))?;
        }
        _ => {
            temp_path.push(format!("{}.txt", filename));
            let mut file = std::fs::File::create(&temp_path).map_err(AppError::from)?;
            use std::io::Write;
            file.write_all(content.as_bytes())
                .map_err(|e| AppError::IO(e.to_string()))?;
        }
    }
    Ok(temp_path)
}

async fn launch_file_with_app(
    app_path: &Option<String>,
    temp_path: &std::path::Path,
    path_str: &str,
    content_type: &str,
    use_direct_path: bool,
) -> Result<(), AppError> {
    if let Some(app) = app_path {
        let target = temp_path.to_string_lossy().to_string();
        launch_target_with_app(app, &target).await?;
    } else {
        launch_with_default_app(path_str, content_type, use_direct_path)?;
    }
    Ok(())
}

async fn launch_target_with_app(app: &str, target: &str) -> Result<(), AppError> {

    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new(app);
        let is_app_bundle = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("app"))
            .unwrap_or(false);

        if is_app_bundle || !path.exists() {
            Command::new("open")
                .args(["-a", app, target])
                .spawn()
                .map_err(|e| AppError::Internal(format!("无法打开应用。\n详情：{}", e)))?;
            return Ok(());
        }

        Command::new(app)
            .arg(target)
            .spawn()
            .map_err(|e| AppError::Internal(format!("无法打开应用。\n详情：{}", e)))?;
        return Ok(());
    }


    #[allow(unreachable_code)]
    Ok(())
}

fn launch_with_default_app(
    path_str: &str,
    _content_type: &str,
    _use_direct_path: bool,
) -> Result<(), AppError> {

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(path_str)
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to open file: {}", e)))?;


    Ok(())
}

fn touch_external_file_preview(
    watch: &WatchedEntryInfo,
    exists: bool,
) -> Result<Option<ClipboardEntry>, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let id = watch.version.id;
    let is_image = watch.content_type == "image";
    let hash =
        is_image.then(|| crate::database::calc_image_hash(&watch.version.content).unwrap_or(0));
    let watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
    if !watch_is_current(&watched, watch) {
        return Ok(None);
    }
    if id > 0 {
        let state = watch.app.state::<DbState>();
        let conn = state.conn.lock().unwrap();
        let Some(entry) = state.repo.get_entry_by_id_with_conn(&conn, id)? else {
            return Ok(None);
        };
        if !watch.version.matches(&entry) {
            return Ok(None);
        }
        state.repo.refresh_file_preview(&conn, id, now, hash)?;
        let mut entry = state
            .repo
            .get_entry_by_id_with_conn(&conn, id)?
            .expect("entry still exists");
        entry.file_preview_exists = exists;
        Ok(Some(entry))
    } else {
        let session = watch.app.state::<crate::app_state::SessionHistory>();
        let mut history = session.0.lock().unwrap();
        if let Some(entry) = history
            .iter_mut()
            .find(|entry| watch.version.matches(entry))
        {
            entry.timestamp = now;
            entry.file_preview_exists = exists;
            if is_image {
                entry.ocr_text = None;
            }
            return Ok(Some(entry.clone()));
        }
        Ok(None)
    }
}

fn read_changed_file(file_path: &std::path::Path, content_type: &str) -> Option<(String, String)> {
    let mut new_content = String::new();
    let success = if content_type == "image" {
        read_image_file(file_path, &mut new_content)
    } else {
        read_text_file(file_path, &mut new_content)
    };

    if success {
        let preview = generate_preview(&new_content);
        Some((new_content, preview))
    } else {
        None
    }
}

fn read_image_file(file_path: &std::path::Path, new_content: &mut String) -> bool {
    if let Ok(mut f) = std::fs::File::open(file_path) {
        let mut buffer = Vec::new();
        if f.read_to_end(&mut buffer).is_ok() {
            // Preserve animated GIFs
            let is_gif_header = buffer.len() >= 6
                && (buffer.starts_with(b"GIF87a") || buffer.starts_with(b"GIF89a"));
            let is_gif_ext = file_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("gif"))
                .unwrap_or(false);

            if is_gif_header || is_gif_ext {
                let b64 = general_purpose::STANDARD.encode(&buffer);
                *new_content = format!("data:image/gif;base64,{}", b64);
                return true;
            }

            if let Ok(img) = image::load_from_memory(&buffer) {
                use std::io::Cursor;
                let mut bytes: Vec<u8> = Vec::new();
                if img
                    .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
                    .is_ok()
                {
                    let b64 = general_purpose::STANDARD.encode(&bytes);
                    *new_content = format!("data:image/png;base64,{}", b64);
                    return true;
                }
            }
        }
    }
    false
}

fn read_text_file(file_path: &std::path::Path, new_content: &mut String) -> bool {
    if let Ok(mut f) = std::fs::File::open(file_path) {
        f.read_to_string(new_content).is_ok()
    } else {
        false
    }
}

fn generate_preview(content: &str) -> String {
    if content.starts_with("data:image") {
        "[New Image]".to_string()
    } else if content.chars().count() > 500 {
        let preview_text: String = content.chars().take(497).collect();
        format!("{}...", preview_text.replace('\n', " "))
    } else {
        content.replace('\n', " ")
    }
}

fn update_database_with_changes(
    watch: &WatchedEntryInfo,
    new_content: String,
    preview: &str,
) -> Result<Option<ClipboardEntry>, String> {
    let mut pending = crate::database::PendingAttachments::default();
    let id = watch.version.id;
    let new_content = if watch.content_type == "image" {
        store_edited_image(&watch.app, id, &new_content, &mut pending)?
    } else {
        new_content
    };
    let watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
    if !watch_is_current(&watched, watch) {
        return Ok(None);
    }
    if id > 0 {
        let state = watch.app.state::<DbState>();
        let conn = state.conn.lock().unwrap();
        let Some(entry) = state.repo.get_entry_by_id_with_conn(&conn, id)? else {
            return Ok(None);
        };
        if !watch.version.matches(&entry) {
            return Ok(None);
        }
        state
            .repo
            .update_entry_content_with_conn(&conn, id, &new_content, preview)?;
        state.repo.get_entry_by_id_with_conn(&conn, id)
    } else {
        let session = watch.app.state::<crate::app_state::SessionHistory>();
        let mut history = session.0.lock().unwrap();
        let Some(entry) = history
            .iter_mut()
            .find(|entry| watch.version.matches(entry))
        else {
            return Ok(None);
        };
        if entry.content != new_content && entry.content_type == "image" {
            entry.ocr_text = None;
            entry.is_external = true;
        }
        entry.content = new_content;
        entry.preview = preview.to_string();
        entry.html_content = None;
        if entry.content_type == "rich_text" {
            entry.content_type = "text".into();
        }
        Ok(Some(entry.clone()))
    }
}

fn watch_is_current(watched: &HashMap<i64, WatchedEntryInfo>, watch: &WatchedEntryInfo) -> bool {
    watched
        .get(&watch.version.id)
        .is_some_and(|current| current.generation == watch.generation)
}

pub(crate) fn app_data_dir(app_handle: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app_handle
        .try_state::<crate::app_state::AppDataDir>()
        .map(|dir| dir.0.lock().unwrap().clone())
}

/// An existing file an entry names, as it opens.
struct FileToOpen {
    path: std::path::PathBuf,
    /// The entry's own file: outside edits refresh its preview and the entry
    /// keeps pointing at it. A copy's saved edits come back as a new image.
    is_original: bool,
}

/// The app's own image files (in its data folder: attachments, session images)
/// are named by their content and may be shared by several entries: edit a
/// copy. Editing them in place changed what other entries (and later copies
/// of the original image) show.
fn file_to_open(
    content_type: &str,
    path: std::path::PathBuf,
    data_dir: Option<&std::path::Path>,
) -> Result<FileToOpen, AppError> {
    if content_type == "image" && data_dir.is_some_and(|dir| path.starts_with(dir)) {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("png");
        let copy = std::env::temp_dir().join(format!(
            "CopyPaste_Clip_edit_{}.{extension}",
            uuid::Uuid::new_v4()
        ));
        std::fs::copy(&path, &copy)?;
        return Ok(FileToOpen {
            path: copy,
            is_original: false,
        });
    }
    Ok(FileToOpen {
        path,
        is_original: true,
    })
}

fn store_edited_image(
    app: &tauri::AppHandle,
    id: i64,
    content: &str,
    pending: &mut crate::database::PendingAttachments,
) -> Result<String, String> {
    let (_, encoded) = content.split_once(',').ok_or("invalid edited image")?;
    let bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| err.to_string())?;
    let data_dir = app_data_dir(app).ok_or("data directory unavailable")?;
    // Session entries keep files in the marked session folder, which is
    // swept once no session entry shows them; database entries use attachments/.
    let root = if id < 0 {
        crate::app::data_directory::session_storage_root(&data_dir)
    } else {
        data_dir
    };
    let ext = crate::database::attachment_extension(&bytes, "png");
    let hash = crate::database::stable_bytes_hash(&bytes);
    let path = pending
        .write(&root.join("attachments"), &bytes, ext, hash)
        .ok_or("could not save edited image")?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{existing_file_paths, file_to_open, names_missing_file};

    #[test]
    fn existing_file_paths_keeps_every_file_still_on_disk() {
        let dir = std::env::temp_dir().join(format!("cp-open-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.html");
        let b = dir.join("b.html");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let missing = dir.join("gone.html");

        let content = format!("{}\n\n  {}  \n{}", a.display(), missing.display(), b.display());
        assert_eq!(existing_file_paths(&content), vec![a, b]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_image_file_is_not_read_as_image_data() {
        let path = "/Users/someone/Library/Application Support/com.copypaste/attachments/img_1.png";
        assert!(names_missing_file("image", path));
        assert!(names_missing_file("file", path));
        assert!(names_missing_file("video", path));
        // Images saved as bare base64 (a JPEG's starts with a slash) are data.
        assert!(!names_missing_file("image", "/9j/4AAQSkZJRgABAQ=="));
        assert!(!names_missing_file("image", "iVBORw0KGgo="));
    }

    #[test]
    fn an_own_image_opens_as_a_copy_of_its_file() {
        // The path has a space, as "Application Support" does on macOS.
        let root = std::env::temp_dir().join(format!("cp open {}", uuid::Uuid::new_v4()));
        let data_dir = root.join("com.copypaste");
        std::fs::create_dir_all(data_dir.join("attachments")).unwrap();
        let saved = data_dir.join("attachments").join("img_open_copy.png");
        std::fs::write(&saved, b"png bytes").unwrap();
        let outside = root.join("photo.png");
        std::fs::write(&outside, b"photo").unwrap();

        let copy = file_to_open("image", saved.clone(), Some(&data_dir)).unwrap();
        let second_copy = file_to_open("image", saved.clone(), Some(&data_dir)).unwrap();
        assert!(!copy.is_original);
        assert_ne!(copy.path, saved);
        assert_ne!(copy.path, second_copy.path);
        assert_eq!(std::fs::read(&copy.path).unwrap(), b"png bytes");
        std::fs::write(&copy.path, b"edited bytes").unwrap();
        assert_eq!(std::fs::read(&second_copy.path).unwrap(), b"png bytes");
        std::fs::remove_file(second_copy.path).unwrap();

        let original = file_to_open("image", outside.clone(), Some(&data_dir)).unwrap();
        assert!(original.is_original);
        assert_eq!(original.path, outside);
        let file = file_to_open("file", saved.clone(), Some(&data_dir)).unwrap();
        assert!(file.is_original);
        assert_eq!(file.path, saved);

        std::fs::remove_file(&copy.path).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
