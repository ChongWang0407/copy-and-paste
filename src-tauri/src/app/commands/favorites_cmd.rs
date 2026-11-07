//! 常用: 常用语 / 常用图 / 常用视频 / 常用文件. Media is copied to
//! `<data>/favorites`, outside the attachments the history cleans up, so a
//! favorite outlives its history entry and its original file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::{AppDataDir, SessionHistory};
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::favorites_repo::{self as repo, Favorite, NewFavorite};
use crate::services::clipboard::{build_entry_preview, detect_content_type};

/// A larger file would silently fill the disk with a copy.
const MAX_FAVORITE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const FAVORITES_DIR: &str = "favorites";

/// `Favorite` as the window sees it: media paths are absolute.
#[derive(Debug, Serialize)]
pub struct FavoriteView {
    #[serde(flatten)]
    pub favorite: Favorite,
    pub paths: Vec<String>,
}

fn is_media(content_type: &str) -> bool {
    matches!(content_type, "image" | "video" | "file")
}

fn favorites_dir(app: &AppHandle) -> PathBuf {
    app.state::<AppDataDir>().0.lock().unwrap().join(FAVORITES_DIR)
}

fn absolute_paths(dir: &Path, favorite: &Favorite) -> Vec<String> {
    if !is_media(&favorite.content_type) {
        return Vec::new();
    }
    favorite
        .content
        .lines()
        .map(|name| dir.join(name).to_string_lossy().to_string())
        .collect()
}

fn view(app: &AppHandle, favorite: Favorite) -> FavoriteView {
    let paths = absolute_paths(&favorites_dir(app), &favorite);
    FavoriteView { favorite, paths }
}

fn tree_size(path: &Path) -> std::io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        total += tree_size(&entry?.path())?;
    }
    Ok(total)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Copies the files into their own folder, keeping their names (pasting a
/// favorite pastes a file called what the original was called). Returns the
/// paths relative to the favorites folder, one per line.
fn store_files(dir: &Path, sources: &[PathBuf]) -> AppResult<String> {
    let mut total = 0;
    for source in sources {
        total += tree_size(source).map_err(|_| AppError::Validation("file_missing".to_string()))?;
    }
    if total > MAX_FAVORITE_BYTES {
        return Err(AppError::Validation("favorite_too_large".to_string()));
    }

    let folder = uuid::Uuid::new_v4().to_string();
    let target = dir.join(&folder);
    std::fs::create_dir_all(&target).map_err(AppError::from)?;
    let mut names = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .ok_or_else(|| AppError::Validation("file_missing".to_string()))?;
        if let Err(error) = copy_tree(source, &target.join(name)) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(AppError::from(error));
        }
        names.push(format!("{folder}/{}", name.to_string_lossy()));
    }
    Ok(names.join("\n"))
}

fn remove_files(dir: &Path, favorite: &Favorite) {
    if !is_media(&favorite.content_type) {
        return;
    }
    let folders: std::collections::HashSet<&str> = favorite
        .content
        .lines()
        .filter_map(|name| name.split('/').next())
        .filter(|folder| !folder.is_empty() && !folder.contains(".."))
        .collect();
    for folder in folders {
        let _ = std::fs::remove_dir_all(dir.join(folder));
    }
}

fn notify(app: &AppHandle) {
    let _ = app.emit("favorites-changed", ());
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tauri::command]
pub fn list_favorites(app_handle: AppHandle, state: State<'_, DbState>) -> AppResult<Vec<FavoriteView>> {
    let favorites = {
        let conn = state.conn.lock().unwrap();
        repo::list(&conn).map_err(AppError::from)?
    };
    let dir = favorites_dir(&app_handle);
    Ok(favorites
        .into_iter()
        .map(|favorite| {
            let paths = absolute_paths(&dir, &favorite);
            FavoriteView { favorite, paths }
        })
        .collect())
}

/// 右键 › 添加为常用语 / 常用图 / 常用视频 / 常用文件.
#[tauri::command]
pub async fn add_favorite_from_entry(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
) -> AppResult<FavoriteView> {
    let (content, content_type, html, source_app, preview) = if id > 0 {
        let entry = state
            .repo
            .get_entry_by_id(id)
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
        let (content, content_type, html) = state
            .repo
            .get_entry_content_with_html(id)
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
        (content, content_type, html, entry.source_app, entry.preview)
    } else {
        let items = session.0.lock().unwrap();
        let item = items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
        (
            item.content.clone(),
            item.content_type.clone(),
            item.html_content.clone(),
            item.source_app.clone(),
            item.preview.clone(),
        )
    };

    if !is_media(&content_type) {
        let conn = state.conn.lock().unwrap();
        if let Some(existing) = repo::find_text(&conn, &content_type, &content).map_err(AppError::from)? {
            return Ok(view(&app_handle, existing));
        }
        let favorite = repo::insert(
            &conn,
            &NewFavorite {
                content_type: &content_type,
                content: &content,
                html_content: html.as_deref(),
                preview: &preview,
                source_app: &source_app,
            },
            now_ms(),
        )
        .map_err(AppError::from)?;
        drop(conn);
        notify(&app_handle);
        return Ok(view(&app_handle, favorite));
    }

    let data_dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    let content_for_copy = content.clone();
    let target = data_dir.join(FAVORITES_DIR);
    let stored = tauri::async_runtime::spawn_blocking(move || {
        let sources: Vec<PathBuf> = content_for_copy
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| crate::database::resolve_entry_file_path(line, &data_dir))
            .collect();
        if sources.is_empty() {
            return Err(AppError::Validation("file_missing".to_string()));
        }
        store_files(&target, &sources)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    let favorite = {
        let conn = state.conn.lock().unwrap();
        repo::insert(
            &conn,
            &NewFavorite {
                content_type: &content_type,
                content: &stored,
                html_content: None,
                preview: &preview,
                source_app: &source_app,
            },
            now_ms(),
        )
        .map_err(AppError::from)?
    };
    notify(&app_handle);
    Ok(view(&app_handle, favorite))
}

fn text_preview(content_type: &str, text: &str) -> String {
    build_entry_preview(content_type, text, None)
}

/// 常用 panel › 手动添加. Only text can be typed in.
#[tauri::command]
pub fn add_favorite_text(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    text: String,
) -> AppResult<FavoriteView> {
    if text.trim().is_empty() {
        return Err(AppError::Validation("empty".to_string()));
    }
    let content_type = detect_content_type(&text);
    let conn = state.conn.lock().unwrap();
    if let Some(existing) = repo::find_text(&conn, &content_type, &text).map_err(AppError::from)? {
        drop(conn);
        return Ok(view(&app_handle, existing));
    }
    let favorite = repo::insert(
        &conn,
        &NewFavorite {
            content_type: &content_type,
            content: &text,
            html_content: None,
            preview: &text_preview(&content_type, &text),
            source_app: "",
        },
        now_ms(),
    )
    .map_err(AppError::from)?;
    drop(conn);
    notify(&app_handle);
    Ok(view(&app_handle, favorite))
}

#[tauri::command]
pub fn update_favorite_text(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    text: String,
) -> AppResult<()> {
    if text.trim().is_empty() {
        return Err(AppError::Validation("empty".to_string()));
    }
    let content_type = detect_content_type(&text);
    let conn = state.conn.lock().unwrap();
    repo::update_text(
        &conn,
        id,
        &content_type,
        &text,
        &text_preview(&content_type, &text),
        now_ms(),
    )
    .map_err(AppError::from)?;
    drop(conn);
    notify(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn delete_favorite(app_handle: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    let favorite = {
        let conn = state.conn.lock().unwrap();
        let favorite = repo::get(&conn, id).map_err(AppError::from)?;
        repo::delete(&conn, id).map_err(AppError::from)?;
        favorite
    };
    if let Some(favorite) = favorite {
        remove_files(&favorites_dir(&app_handle), &favorite);
    }
    notify(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn reorder_favorites(state: State<'_, DbState>, ids: Vec<i64>) -> AppResult<()> {
    let mut conn = state.conn.lock().unwrap();
    repo::reorder(&mut conn, &ids).map_err(AppError::from)
}

/// Click on a favorite: pastes it at once. Favorites are never deleted by it.
#[tauri::command]
pub async fn paste_favorite(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    id: i64,
) -> AppResult<()> {
    let favorite = {
        let conn = state.conn.lock().unwrap();
        repo::get(&conn, id).map_err(AppError::from)?
    }
    .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;

    let content = if is_media(&favorite.content_type) {
        absolute_paths(&favorites_dir(&app_handle), &favorite).join("\n")
    } else {
        favorite.content.clone()
    };
    crate::services::clipboard_ops::paste_stored_content(
        &app_handle,
        &state,
        &content,
        &favorite.content_type,
        favorite.html_content.as_deref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{remove_files, store_files};
    use crate::infrastructure::repository::favorites_repo::Favorite;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("copy-paste-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stored_files_outlive_the_original_and_go_with_the_favorite() {
        let root = temp_dir("favorites");
        let source = root.join("clip.png");
        std::fs::write(&source, b"image").unwrap();
        let dir = root.join("favorites");

        let stored = store_files(&dir, &[source.clone()]).unwrap();
        std::fs::remove_file(&source).unwrap();
        assert!(stored.ends_with("/clip.png"));
        assert_eq!(std::fs::read(dir.join(&stored)).unwrap(), b"image");

        let favorite = Favorite {
            id: 1,
            content_type: "image".to_string(),
            content: stored.clone(),
            html_content: None,
            preview: String::new(),
            source_app: String::new(),
            sort_order: 1,
            created_at: 0,
        };
        remove_files(&dir, &favorite);
        assert!(!dir.join(&stored).exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
