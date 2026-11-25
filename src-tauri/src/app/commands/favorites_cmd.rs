//! 常用: 常用语 / 常用图 / 常用视频 / 常用文件. Media is copied to
//! `<data>/favorites`, outside the attachments the history cleans up, so a
//! favorite outlives its history entry and its original file.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
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

fn add_text(conn: &Connection, text: &str, title: &str, tags: &[String], now: i64) -> AppResult<Favorite> {
    if text.trim().is_empty() {
        return Err(AppError::Validation("empty".to_string()));
    }
    let content_type = detect_content_type(text);
    if repo::find_text(conn, &content_type, text)?.is_some() {
        return Err(AppError::Validation("favorite_duplicate".to_string()));
    }
    let tx = conn.unchecked_transaction()?;
    let favorite = repo::insert(
        &tx,
        &NewFavorite {
            content_type: &content_type,
            content: text,
            html_content: None,
            preview: &text_preview(&content_type, text),
            source_app: "",
        },
        now,
    )?;
    repo::set_meta(&tx, &favorite, title, tags, now)?;
    let favorite = repo::get(&tx, favorite.id)?.expect("just inserted");
    tx.commit()?;
    Ok(favorite)
}

fn existing(conn: &Connection, id: i64) -> AppResult<Favorite> {
    repo::get(conn, id)?.ok_or_else(|| AppError::Validation("Item not found".to_string()))
}

/// The text is rewritten only when it changed, so a title or tag edit keeps
/// a rich-text favorite's format. `dropping_html` runs before a rewrite.
fn edit_text(
    conn: &Connection,
    id: i64,
    text: &str,
    title: &str,
    tags: &[String],
    now: i64,
    dropping_html: impl FnOnce(&Connection, &Favorite) -> AppResult<()>,
) -> AppResult<()> {
    if text.trim().is_empty() {
        return Err(AppError::Validation("empty".to_string()));
    }
    let tx = conn.unchecked_transaction()?;
    let favorite = existing(&tx, id)?;
    if is_media(&favorite.content_type) {
        return Err(AppError::Validation("favorite_not_text".to_string()));
    }
    if favorite.content != text {
        dropping_html(&tx, &favorite)?;
        let content_type = detect_content_type(text);
        repo::update_text(&tx, id, &content_type, text, &text_preview(&content_type, text), now)?;
    }
    repo::set_meta(&tx, &favorite, title, tags, now)?;
    tx.commit()?;
    Ok(())
}

fn edit_meta(conn: &Connection, id: i64, title: &str, tags: &[String], now: i64) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    let favorite = existing(&tx, id)?;
    repo::set_meta(&tx, &favorite, title, tags, now)?;
    tx.commit()?;
    Ok(())
}

/// 常用 panel › 手动添加. Only text can be typed in; the same text twice is refused.
#[tauri::command]
pub fn add_favorite_text(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    text: String,
    title: Option<String>,
    tags: Option<Vec<String>>,
) -> AppResult<FavoriteView> {
    let favorite = {
        let conn = state.conn.lock().unwrap();
        add_text(
            &conn,
            &text,
            title.as_deref().unwrap_or_default(),
            tags.as_deref().unwrap_or_default(),
            now_ms(),
        )?
    };
    notify(&app_handle);
    Ok(view(&app_handle, favorite))
}

#[tauri::command]
pub fn update_favorite_text(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    text: String,
    title: String,
    tags: Vec<String>,
) -> AppResult<()> {
    let data_dir = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    {
        let conn = state.conn.lock().unwrap();
        edit_text(&conn, id, &text, &title, &tags, now_ms(), |tx, favorite| {
            state
                .repo
                .queue_favorite_attachment_deletions(tx, &data_dir, favorite.html_content.as_deref())
                .map_err(AppError::Internal)
        })?;
    }
    notify(&app_handle);
    Ok(())
}

/// Title and tags only: the content, format and files stay as they are.
#[tauri::command]
pub fn update_favorite_meta(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    title: String,
    tags: Vec<String>,
) -> AppResult<()> {
    {
        let conn = state.conn.lock().unwrap();
        edit_meta(&conn, id, &title, &tags, now_ms())?;
    }
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
mod meta_tests {
    use super::{add_text, edit_meta, edit_text};
    use crate::infrastructure::repository::favorites_repo::{self as repo, NewFavorite};
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        conn
    }

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn meta_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM cp_fork_favorite_meta", [], |row| row.get(0)).unwrap()
    }

    fn rich(conn: &Connection) -> repo::Favorite {
        repo::insert(
            conn,
            &NewFavorite {
                content_type: "rich_text",
                content: "bold",
                html_content: Some("<b>bold</b>"),
                preview: "bold",
                source_app: "Pages",
            },
            10,
        )
        .unwrap()
    }

    #[test]
    fn adding_keeps_title_and_tags_and_refuses_the_same_text() {
        let conn = db();
        let saved = add_text(&conn, "hello", " Greeting ", &tags(&["work"]), 1).unwrap();
        assert_eq!(saved.title, "Greeting");
        assert_eq!(saved.tags, ["work"]);
        let error = add_text(&conn, "hello", "", &[], 2).unwrap_err().to_string();
        assert!(error.contains("favorite_duplicate"), "{error}");
        assert_eq!(repo::list(&conn).unwrap().len(), 1);
        assert!(add_text(&conn, "  ", "", &[], 3).is_err());
    }

    #[test]
    fn title_and_tag_edits_keep_the_rich_format() {
        let conn = db();
        let favorite = rich(&conn);
        let mut dropped = false;
        edit_text(&conn, favorite.id, "bold", "Title", &tags(&["a"]), 11, |_, _| {
            dropped = true;
            Ok(())
        })
        .unwrap();
        assert!(!dropped);
        let edited = repo::get(&conn, favorite.id).unwrap().unwrap();
        assert_eq!(edited.content_type, "rich_text");
        assert_eq!(edited.html_content.as_deref(), Some("<b>bold</b>"));
        assert_eq!((edited.title.as_str(), edited.tags.clone()), ("Title", tags(&["a"])));
        let updated_at: i64 = conn
            .query_row("SELECT updated_at FROM favorites WHERE id = ?1", [favorite.id], |row| row.get(0))
            .unwrap();
        assert_eq!(updated_at, 10, "the favorites row is not touched");

        edit_text(&conn, favorite.id, "plain now", "Title", &tags(&["a"]), 12, |_, old| {
            dropped = old.html_content.is_some();
            Ok(())
        })
        .unwrap();
        assert!(dropped);
        let edited = repo::get(&conn, favorite.id).unwrap().unwrap();
        assert_eq!((edited.content_type.as_str(), edited.html_content), ("text", None));
        assert_eq!(edited.title, "Title");
    }

    #[test]
    fn media_takes_titles_and_tags_but_never_text() {
        let conn = db();
        let image = repo::insert(
            &conn,
            &NewFavorite { content_type: "image", content: "f/a.png", html_content: None, preview: "Image", source_app: "" },
            5,
        )
        .unwrap();
        let error = edit_text(&conn, image.id, "text", "", &[], 6, |_, _| Ok(())).unwrap_err();
        assert!(error.to_string().contains("favorite_not_text"));
        edit_meta(&conn, image.id, "Logo", &tags(&["brand"]), 7).unwrap();
        let edited = repo::get(&conn, image.id).unwrap().unwrap();
        assert_eq!((edited.content.as_str(), edited.title.as_str()), ("f/a.png", "Logo"));
        // Clearing both leaves no row behind.
        edit_meta(&conn, image.id, " ", &tags(&[" "]), 8).unwrap();
        assert_eq!(meta_rows(&conn), 0);
        assert!(edit_meta(&conn, 999, "x", &[], 9).is_err());
        assert_eq!(meta_rows(&conn), 0, "no row for a missing favorite");
    }

    #[test]
    fn titles_and_tags_are_normalized() {
        let conn = db();
        let long_tag = "标".repeat(30);
        let mut many = tags(&[" Prompt ", "", "prompt", "PROMPT", "  ", "😀😀"]);
        many.push(long_tag);
        many.extend((0..10).map(|n| format!("t{n}")));
        let saved = add_text(&conn, "x", &format!("  {}  ", "长".repeat(100)), &many, 1).unwrap();
        assert_eq!(saved.title.chars().count(), 80);
        assert_eq!(saved.tags.len(), 8);
        assert_eq!(&saved.tags[..3], ["Prompt", "😀😀", &"标".repeat(24)]);
        assert_eq!(saved.tags[3], "t0");
        assert_eq!(repo::normalize_title(&format!("{}  b", "a".repeat(79))), "a".repeat(79));
    }

    #[test]
    fn stale_rows_are_ignored_and_deleted_and_deletes_cascade() {
        let conn = db();
        let favorite = rich(&conn);
        conn.execute(
            "INSERT INTO cp_fork_favorite_meta (favorite_id, favorite_created_at, title, tags, updated_at)
             VALUES (?1, 999, 'old', '[\"old\"]', 1)",
            [favorite.id],
        )
        .unwrap();
        let listed = repo::list(&conn).unwrap();
        assert_eq!((listed[0].title.as_str(), listed[0].tags.len()), ("", 0));
        assert_eq!(meta_rows(&conn), 0);

        edit_meta(&conn, favorite.id, "Kept", &tags(&["a"]), 2).unwrap();
        assert_eq!(meta_rows(&conn), 1);
        let tx = conn.unchecked_transaction().unwrap();
        repo::delete(&tx, favorite.id).unwrap();
        tx.commit().unwrap();
        assert_eq!(meta_rows(&conn), 0);
        // The next favorite may reuse the id; it starts with no title.
        let next = add_text(&conn, "new", "", &[], 3).unwrap();
        assert_eq!(next.title, "");
    }

    #[cfg(not(feature = "portable"))]
    #[test]
    fn titles_and_tags_of_an_encrypted_favorite_are_encrypted() {
        use crate::infrastructure::encryption;
        let conn = db();
        let secret = repo::insert_with_protection(
            &conn,
            &NewFavorite { content_type: "text", content: "secret", html_content: None, preview: "secret", source_app: "" },
            1,
            true,
        )
        .unwrap();
        edit_text(&conn, secret.id, "secret", "Bank", &tags(&["pin"]), 2, |_, _| Ok(())).unwrap();
        let (title, stored_tags): (String, String) = conn
            .query_row("SELECT title, tags FROM cp_fork_favorite_meta", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert!(encryption::is_modern_ciphertext(&title));
        assert!(encryption::is_modern_ciphertext(&stored_tags));
        let read = repo::get(&conn, secret.id).unwrap().unwrap();
        assert_eq!((read.title.as_str(), read.tags.clone()), ("Bank", tags(&["pin"])));

        // A plain favorite that later turns sensitive takes its title along.
        let plain = add_text(&conn, "later", "Visible", &tags(&["x"]), 3).unwrap();
        let mut found = repo::find_text(&conn, "text", "later").unwrap().unwrap();
        assert_eq!(found.title, "Visible");
        repo::protect(&conn, &mut found).unwrap();
        let title: String = conn
            .query_row("SELECT title FROM cp_fork_favorite_meta WHERE favorite_id = ?1", [plain.id], |row| row.get(0))
            .unwrap();
        assert!(encryption::is_modern_ciphertext(&title));
        assert_eq!(repo::get(&conn, plain.id).unwrap().unwrap().title, "Visible");
    }
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
            title: String::new(),
            tags: Vec::new(),
        };
        remove_files(&dir, &favorite);
        assert!(!dir.join(&stored).exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
