// Content handler module for opening various content types.
use crate::database::DbState;
use crate::error::AppError;
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;
use std::io::Read;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use tauri::{Emitter, Manager, State};

struct WatchedEntryInfo {
    path: std::path::PathBuf,
    cancel: Arc<AtomicBool>,
    last_activity: Arc<Mutex<Instant>>,
}

static WATCHED_PREVIEW_ENTRIES: LazyLock<Mutex<HashMap<i64, WatchedEntryInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[tauri::command]
pub async fn open_content(
    app_handle: tauri::AppHandle,
    state: State<'_, DbState>,
    id: i64,
    mut content: String,
    content_type: String,
) -> Result<(), AppError> {
    let mut html_content = None;
    // 0. Resolve full content if ID is provided and content is placeholder/truncated
    if id != 0 {
        if id > 0 {
            // Fetch from Database
            if let Ok(Some((full_content, _, full_html))) =
                state.repo.get_entry_content_with_html(id)
            {
                content = full_content;
                html_content = full_html;
            }
        } else {
            // Fetch from Session
            let session = app_handle.state::<crate::app_state::SessionHistory>();
            let session_items = session.0.lock().unwrap();
            if let Some(item) = session_items.iter().find(|i| i.id == id) {
                content = item.content.clone();
                html_content = item.html_content.clone();
            }
        }
    }

    // Increment use count if ID is provided
    if id > 0 {
        let _ = state.repo.increment_use_count(id);
    }

    let app_path = get_app_path_for_content_type(&state, &content_type)?;
    let filename = if id != 0 {
        if id > 0 {
            format!("CopyPaste_Clip_entry_{}", id)
        } else {
            format!("CopyPaste_Clip_session_{}", id.abs())
        }
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
            entry_file = Some(file_to_open(&content_type, path, data_dir.as_deref()));
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
    if id != 0 && content_type != "rich_text" {
        register_file_preview_watch(
            app_handle,
            id,
            file_path_clone,
            content_type,
            use_direct_path,
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

/// Watch an on-disk file and refresh list previews when it changes.
pub fn register_file_preview_watch(
    app_handle: tauri::AppHandle,
    id: i64,
    file_path: std::path::PathBuf,
    content_type: String,
    keep_path_on_change: bool,
) {
    if id == 0 {
        return;
    }

    let (cancel_token, last_activity) = {
        let mut watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
        if let Some(existing) = watched.get_mut(&id) {
            if existing.path == file_path {
                // Stable reuse of the same entry file: refresh activity timer and continue existing watch
                *existing.last_activity.lock().unwrap() = Instant::now();
                return;
            } else {
                // Path changed: signal cancellation to previous watcher and spawn new watcher
                existing.cancel.store(true, Ordering::Relaxed);
                let cancel = Arc::new(AtomicBool::new(false));
                let last_act = Arc::new(Mutex::new(Instant::now()));
                *existing = WatchedEntryInfo {
                    path: file_path.clone(),
                    cancel: cancel.clone(),
                    last_activity: last_act.clone(),
                };
                (cancel, last_act)
            }
        } else {
            let cancel = Arc::new(AtomicBool::new(false));
            let last_act = Arc::new(Mutex::new(Instant::now()));
            watched.insert(
                id,
                WatchedEntryInfo {
                    path: file_path.clone(),
                    cancel: cancel.clone(),
                    last_activity: last_act.clone(),
                },
            );
            (cancel, last_act)
        }
    };

    std::thread::spawn(move || {
        let mut last_mtime = std::fs::metadata(&file_path)
            .ok()
            .and_then(|m| m.modified().ok());

        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));

            if cancel_token.load(Ordering::Relaxed) {
                break;
            }

            let elapsed = last_activity.lock().unwrap().elapsed().as_secs();
            if elapsed >= 3600 {
                break;
            }

            if !file_path.exists() {
                break;
            }

            let current_mtime = std::fs::metadata(&file_path)
                .ok()
                .and_then(|m| m.modified().ok());

            if current_mtime != last_mtime {
                last_mtime = current_mtime;
                println!("File changed detected: {:?}", file_path);

                if keep_path_on_change {
                    touch_external_file_preview(&app_handle, id);
                } else if let Some((new_content, preview)) =
                    read_changed_file(&file_path, &content_type)
                {
                    let new_content = store_edited_image(&app_handle, id, &content_type, new_content);
                    update_database_with_changes(&app_handle, id, &new_content, &preview);
                }
            }
        }

        let mut watched = WATCHED_PREVIEW_ENTRIES.lock().unwrap();
        if let Some(existing) = watched.get(&id) {
            if existing.cancel.load(Ordering::Relaxed)
                || existing.last_activity.lock().unwrap().elapsed().as_secs() >= 3600
                || !file_path.exists()
            {
                watched.remove(&id);
            }
        }
    });
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

fn touch_external_file_preview(app_handle: &tauri::AppHandle, id: i64) {
    let state = app_handle.state::<DbState>();
    let now = chrono::Utc::now().timestamp_millis();
    let mut updated_entry = None;

    if id > 0 {
        if state.repo.touch_entry(id, now).is_err() {
            return;
        }
        if let Ok(Some(entry)) = state.repo.get_entry_by_id(id) {
            updated_entry = Some(entry);
        }
    } else {
        use crate::app_state::SessionHistory;
        if let Some(session) = app_handle.try_state::<SessionHistory>() {
            let mut history = session.0.lock().unwrap();
            if let Some(item) = history.iter_mut().find(|i| i.id == id) {
                item.timestamp = now;
                updated_entry = Some(item.clone());
            }
        }
    }

    if let Some(entry) = updated_entry {
        let _ = app_handle.emit("clipboard-updated", entry);
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
    app_handle: &tauri::AppHandle,
    id: i64,
    new_content: &str,
    preview: &str,
) {
    let state = app_handle.state::<DbState>();
    let mut updated_entry = None;

    if id > 0 {
        if state
            .repo
            .update_entry_content(id, new_content, preview)
            .is_err()
        {
            return;
        }

        if let Ok(Some(entry)) = state.repo.get_entry_by_id(id) {
            updated_entry = Some(entry);
        }
    } else {
        use crate::app_state::SessionHistory;
        if let Some(session) = app_handle.try_state::<SessionHistory>() {
            let mut history = session.0.lock().unwrap();
            if let Some(item) = history.iter_mut().find(|i| i.id == id) {
                item.content = new_content.to_string();
                item.preview = preview.to_string();
                item.html_content = None;
                if item.content_type == "rich_text" {
                    item.content_type = "text".to_string();
                }
                updated_entry = Some(item.clone());
            }
        }
    }

    if let Some(entry) = updated_entry {
        let _ = app_handle.emit("clipboard-updated", entry);
        println!(
            "Clipboard entry updated and clipboard-updated event emitted for id: {}",
            id
        );
    } else {
        let _ = app_handle.emit("clipboard-changed", id);
    }
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
) -> FileToOpen {
    if content_type == "image" && data_dir.is_some_and(|dir| path.starts_with(dir)) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image.png");
        let copy = std::env::temp_dir().join(format!("CopyPaste_Clip_edit_{}", name));
        if std::fs::copy(&path, &copy).is_ok() {
            return FileToOpen { path: copy, is_original: false };
        }
    }
    FileToOpen { path, is_original: true }
}

/// An edited image arrives as a data URL; keep it as an attachment file
/// rather than a large data URL in the database.
fn store_edited_image(
    app_handle: &tauri::AppHandle,
    id: i64,
    content_type: &str,
    content: String,
) -> String {
    if content_type != "image" || !content.starts_with("data:") {
        return content;
    }
    let Some(data_dir) = app_data_dir(app_handle) else {
        return content;
    };
    // Database entries (id > 0, e.g. pinned while persistence is off) keep
    // their files in attachments/; only session entries use session/, which
    // is swept once no session entry shows the file.
    let root = if id > 0 { data_dir.clone() } else { data_dir.join("session") };
    crate::database::save_image_to_file(&content, &root).unwrap_or(content)
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

        let copy = file_to_open("image", saved.clone(), Some(&data_dir));
        assert!(!copy.is_original);
        assert_ne!(copy.path, saved);
        assert_eq!(std::fs::read(&copy.path).unwrap(), b"png bytes");

        let original = file_to_open("image", outside.clone(), Some(&data_dir));
        assert!(original.is_original);
        assert_eq!(original.path, outside);
        let file = file_to_open("file", saved.clone(), Some(&data_dir));
        assert!(file.is_original);
        assert_eq!(file.path, saved);

        std::fs::remove_file(&copy.path).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
