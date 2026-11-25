//! 常用: 常用语 / 常用图 / 常用视频 / 常用文件. Media is copied to
//! `<data>/favorites`, outside the attachments the history cleans up, so a
//! favorite outlives its history entry and its original file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::{AppDataDir, SessionHistory};
use crate::database::DbState;
use crate::error::{AppError, AppResult};
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

fn copy_tree(
    from: &Path,
    to: &Path,
    ancestors: &mut std::collections::HashSet<PathBuf>,
    remaining: &mut u64,
) -> AppResult<()> {
    use std::io::Read;
    let source = from
        .canonicalize()
        .map_err(|_| AppError::Validation("file_missing".into()))?;
    let metadata = std::fs::metadata(&source)?;
    if metadata.is_dir() {
        if to
            .parent()
            .expect("favorite copy has a parent")
            .canonicalize()?
            .starts_with(&source)
            || !ancestors.insert(source.clone())
        {
            return Err(AppError::Validation("favorite_directory_cycle".into()));
        }
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(&source)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &to.join(entry.file_name()),
                ancestors,
                remaining,
            )?;
        }
        ancestors.remove(&source);
    } else if metadata.is_file() {
        if metadata.len() > *remaining {
            return Err(AppError::Validation("favorite_too_large".into()));
        }
        let mut input = std::fs::File::open(&source)?.take(*remaining + 1);
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(to)?;
        let copied = std::io::copy(&mut input, &mut output)?;
        if copied > *remaining {
            return Err(AppError::Validation("favorite_too_large".into()));
        }
        output.set_permissions(metadata.permissions())?;
        *remaining -= copied;
    } else {
        return Err(AppError::Validation("file_missing".into()));
    }
    Ok(())
}

/// Keep the original names even when files from different folders share one.
fn store_files(dir: &Path, sources: &[PathBuf]) -> AppResult<String> {
    let folder = uuid::Uuid::new_v4().to_string();
    let target = dir.join(&folder);
    std::fs::create_dir_all(dir)?;
    if !dir.symlink_metadata()?.is_dir() {
        return Err(AppError::Validation("file_missing".into()));
    }
    std::fs::create_dir(&target)?;
    let result = (|| {
        let mut remaining = MAX_FAVORITE_BYTES;
        let mut names = Vec::new();
        let mut ancestors = std::collections::HashSet::new();
        for source in sources {
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.contains(['\r', '\n']))
                .ok_or_else(|| AppError::Validation("file_missing".into()))?;
            let path = if target.join(name).exists() {
                let duplicate = uuid::Uuid::new_v4().to_string();
                std::fs::create_dir(target.join(&duplicate))?;
                format!("{duplicate}/{name}")
            } else {
                name.to_owned()
            };
            copy_tree(source, &target.join(&path), &mut ancestors, &mut remaining)?;
            names.push(format!("{folder}/{path}"));
        }
        Ok(names.join("\n"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&target);
    }
    result
}

fn remove_files(dir: &Path, favorite: &Favorite) {
    if !is_media(&favorite.content_type) {
        return;
    }
    remove_stored_files(dir, &favorite.content);
}

fn remove_stored_files(dir: &Path, stored: &str) {
    if !dir.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        return;
    }
    let folders: std::collections::HashSet<&str> = stored
        .lines()
        .filter_map(|name| name.split('/').next())
        .filter(|folder| uuid::Uuid::parse_str(folder).is_ok())
        .collect();
    for path in stored.lines().map(|name| Path::new(name)) {
        if path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
            && path
                .components()
                .next()
                .is_some_and(|part| folders.contains(part.as_os_str().to_str().unwrap_or_default()))
        {
            crate::app::commands::thumbnail_cmd::remove_cached_thumbnails(&dir.join(path));
        }
    }
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
    let (content, content_type, source_app, preview, is_external, protected) = {
        // Keep a rich-text attachment alive until its favorite row is visible.
        let _attachments = crate::database::ATTACHMENT_WRITERS.read().unwrap();
        let (entry, protected) = if id > 0 {
            let conn = state.conn.lock().unwrap();
            let entry = state
                .repo
                .get_entry_by_id_with_conn(&conn, id)
                .map_err(AppError::from)?
                .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
            let protected = repo::is_sensitive_source(&conn, id)?
                || crate::database::has_sensitive_tag(&entry.tags);
            (entry, protected)
        } else {
            let items = session.0.lock().unwrap();
            let item = items
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(|| AppError::Validation("Item not found".to_string()))?;
            (item.clone(), crate::database::has_sensitive_tag(&item.tags))
        };
        let crate::database::ClipboardEntry {
            content,
            content_type,
            html_content: html,
            source_app,
            preview,
            is_external,
            ..
        } = entry;

        if !is_media(&content_type) {
            let conn = state.conn.lock().unwrap();
            if let Some(mut existing) =
                repo::find_text(&conn, &content_type, &content).map_err(AppError::from)?
            {
                if protected {
                    repo::protect(&conn, &mut existing)?;
                }
                drop(conn);
                return Ok(view(&app_handle, existing));
            }
            let favorite = repo::insert_with_protection(
                &conn,
                &NewFavorite {
                    content_type: &content_type,
                    content: &content,
                    html_content: html.as_deref(),
                    preview: &preview,
                    source_app: &source_app,
                },
                now_ms(),
                protected,
            )
            .map_err(AppError::from)?;
            drop(conn);
            notify(&app_handle);
            return Ok(view(&app_handle, favorite));
        }
        (
            content,
            content_type,
            source_app,
            preview,
            is_external,
            protected,
        )
    };

    let data_dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    let target = data_dir.join(FAVORITES_DIR);
    let copy_target = target.clone();
    let stored = tauri::async_runtime::spawn_blocking(move || {
        let _attachments = crate::database::ATTACHMENT_WRITERS.read().unwrap();
        let sources: Vec<PathBuf> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let path = Path::new(line.trim());
                if path.exists() {
                    Some(path.to_path_buf())
                } else if is_external
                    && path
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == "attachments")
                {
                    crate::database::resolve_entry_file_path(line, &data_dir)
                } else {
                    None
                }
            })
            .collect::<Option<_>>()
            .ok_or_else(|| AppError::Validation("file_missing".into()))?;
        if sources.is_empty() {
            return Err(AppError::Validation("file_missing".to_string()));
        }
        store_files(&copy_target, &sources)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    let favorite = {
        let conn = state.conn.lock().unwrap();
        repo::insert_with_protection(
            &conn,
            &NewFavorite {
                content_type: &content_type,
                content: &stored,
                html_content: None,
                preview: &preview,
                source_app: &source_app,
            },
            now_ms(),
            protected,
        )
    };
    let favorite = match favorite {
        Ok(favorite) => favorite,
        Err(error) => {
            remove_stored_files(&target, &stored);
            return Err(AppError::from(error));
        }
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
    let data_dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    let conn = state.conn.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    if let Some(favorite) = repo::get(&tx, id)? {
        state.repo.queue_favorite_attachment_deletions(&tx, &data_dir, favorite.html_content.as_deref())?;
    }
    repo::update_text(
        &tx,
        id,
        &content_type,
        &text,
        &text_preview(&content_type, &text),
        now_ms(),
    )
    .map_err(AppError::from)?;
    tx.commit()?;
    drop(conn);
    notify(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn delete_favorite(app_handle: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    let dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    let favorite = {
        let conn = state.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let favorite = repo::get(&tx, id)?;
        if let Some(favorite) = &favorite {
            state.repo.queue_favorite_attachment_deletions(&tx, &dir, favorite.html_content.as_deref())?;
        }
        repo::delete(&tx, id)?;
        tx.commit()?;
        favorite
    };
    if let Some(favorite) = favorite {
        remove_files(&dir.join(FAVORITES_DIR), &favorite);
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
    fn same_named_sources_keep_both_files() {
        let root = temp_dir("favorite-names");
        let first = root.join("a/file.txt");
        let second = root.join("b/file.txt");
        for (path, content) in [(&first, "first"), (&second, "second")] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let dir = root.join("favorites");
        let stored = store_files(&dir, &[first, second]).unwrap();
        let copies: Vec<_> = stored.lines().map(|name| std::fs::read_to_string(dir.join(name)).unwrap()).collect();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(copies, ["first", "second"]);
    }

    #[test]
    #[cfg(unix)]
    fn following_symlinks_enforces_the_size_limit_and_rejects_cycles() {
        let root = temp_dir("favorite-symlinks");
        let favorites = root.join("favorites");
        let file = root.join("large.bin");
        std::fs::File::create(&file).unwrap().set_len(super::MAX_FAVORITE_BYTES + 1).unwrap();
        let link = root.join("alias");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(store_files(&favorites, &[link]).unwrap_err().to_string().contains("favorite_too_large"));

        let directory = root.join("directory");
        std::fs::create_dir(&directory).unwrap();
        std::os::unix::fs::symlink(&directory, directory.join("cycle")).unwrap();
        assert!(store_files(&favorites, &[directory]).is_err());
        assert_eq!(std::fs::read_dir(&favorites).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_source_rolls_back_completed_copies() {
        let root = temp_dir("favorite-incomplete");
        let source = root.join("first.txt");
        std::fs::write(&source, b"original").unwrap();
        let dir = root.join("favorites");
        assert!(store_files(&dir, &[source.clone(), root.join("missing.txt")]).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(std::fs::read(&source).unwrap(), b"original");
        std::fs::remove_dir_all(root).unwrap();
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
            is_encrypted: false,
        };
        remove_files(&dir, &favorite);
        assert!(!dir.join(&stored).exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
