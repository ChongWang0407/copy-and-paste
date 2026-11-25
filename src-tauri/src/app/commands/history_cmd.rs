use crate::app_state::{AppDataDir, PasteQueue, SessionHistory};
use crate::database::DbState;
use crate::domain::models::{ClipboardEntry, EntryIdentity};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::{
    matches_type, ClipboardRepository, ClipboardSortMode, ConfirmedEntry, SqliteClipboardRepository,
    SELECTION_CHANGED,
};
use crate::infrastructure::repository::tag_repo::TagRepository;
use crate::services::clipboard::entry_matches_search;
use std::collections::{HashSet, VecDeque};
use tauri::{AppHandle, Emitter, State};

fn validate_and_repair_external_media(item: &mut ClipboardEntry, data_dir: &std::path::Path) {
    if item.content_type == "image"
        || item.content_type == "video"
        || item.content_type == "file"
        || item.is_external
    {
        if item.content.starts_with("data:") {
            item.file_preview_exists = true;
            return;
        }

        let raw_path = item.content.trim();
        if raw_path.is_empty() {
            item.file_preview_exists = false;
            return;
        }

        // Multi-file entries count as present while any file is: one moved
        // file used to show the whole entry as deleted although the rest still
        // paste. Only the first files are checked, so a huge list or a network
        // share doesn't make every page load stat thousands of paths.
        if item.content_type == "file" && raw_path.contains('\n') {
            const MAX_CHECKED_FILES: usize = 50;
            let mut paths = raw_path.lines().map(str::trim).filter(|line| !line.is_empty());
            let any_exist = paths
                .by_ref()
                .take(MAX_CHECKED_FILES)
                .any(|p| std::path::Path::new(p).exists())
                || paths.next().is_some();
            item.file_preview_exists = any_exist;
            return;
        }

        match crate::database::resolve_entry_file_path(raw_path, data_dir) {
            Some(path) => {
                if path.as_os_str() != raw_path {
                    item.content = path.to_string_lossy().to_string();
                }
                item.file_preview_exists = true;
            }
            None => item.file_preview_exists = false,
        }
    }
}

// async: off the main thread. Checking that files exist can be slow (many
// files, a disconnected network share) and froze the UI on every list load.
#[tauri::command(async)]
pub fn get_clipboard_history(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    limit: i32,
    offset: i32,
    content_type: Option<String>,
    sort_mode: Option<String>,
) -> AppResult<Vec<ClipboardEntry>> {
    let sort_mode = ClipboardSortMode::from_setting(sort_mode.as_deref());
    history_page(&state, &session, &app_data, limit, offset, content_type, sort_mode)
}

/// One page of history (database and session entries merged) in `sort_mode`.
pub fn history_page(
    state: &DbState,
    session: &SessionHistory,
    app_data: &AppDataDir,
    limit: i32,
    offset: i32,
    content_type: Option<String>,
    sort_mode: ClipboardSortMode,
) -> AppResult<Vec<ClipboardEntry>> {
    let data_dir = app_data.0.lock().unwrap().clone();
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let mut history = {
        let items = session.0.lock().unwrap();
        state.repo.history_with_session(
            items.iter().collect(),
            limit,
            offset.max(0),
            content_type.as_deref(),
            sort_mode,
        )?
    };
    for item in &mut history {
        prepare_list_item(item, &data_dir);
    }
    Ok(history)
}

fn prepare_list_item(item: &mut ClipboardEntry, data_dir: &std::path::Path) {
    validate_and_repair_external_media(item, data_dir);
    item.html_content = None;
    item.ocr_text = None;
    if crate::database::is_text_type(&item.content_type) {
        if let Some((end, _)) = item.content.char_indices().nth(2000) {
            item.content.truncate(end);
            item.content.push_str("... [Truncated for speed]");
        }
        if item.preview.is_empty() {
            item.preview = crate::services::clipboard::build_entry_preview(
                &item.content_type,
                &item.content,
                None,
            );
        }
    }
}

#[cfg(test)]
fn sorted_page<T: Clone>(items: &mut Vec<&T>, offset: usize, limit: usize,
    compare: impl Fn(&T, &T) -> std::cmp::Ordering) -> Vec<T> {
    items.sort_by(|a, b| compare(a, b));
    items.iter().skip(offset).take(limit).map(|item| (*item).clone()).collect()
}

#[cfg(test)]
fn merged_history_page<'a>(
    session_items: impl Iterator<Item = &'a ClipboardEntry>,
    db_items: &'a [ClipboardEntry],
    offset: usize,
    limit: usize,
    sort_mode: ClipboardSortMode,
) -> Vec<ClipboardEntry> {
    let mut seen = HashSet::new();
    // Prefer database fields over any legacy in-memory copy. Deduplicate BEFORE
    // slicing so duplicate IDs cannot consume page slots or shift later pages.
    let mut candidates = db_items.iter().chain(session_items)
        .filter(|item| item.id == 0 || seen.insert(item.id))
        .collect();
    sorted_page(&mut candidates, offset, limit, |a, b| sort_mode.compare(a, b))
}

#[cfg(test)]
mod merged_page_tests {
    use super::{merged_history_page, sorted_page, ClipboardEntry, ClipboardSortMode};

    fn entry(id: i64, sort_at: i64, tags: &[&str]) -> ClipboardEntry {
        serde_json::from_value(serde_json::json!({
            "id": id, "content_type": "text", "content": "same content",
            "source_app": "Test", "timestamp": sort_at, "sort_at": sort_at,
            "preview": "same content", "is_pinned": false, "tags": tags,
        })).unwrap()
    }

    #[test]
    fn promoted_entry_appears_once_and_uses_database_tags() {
        let session = [entry(12, 30, &["old"]), entry(-1, 20, &[])];
        let database = [entry(12, 30, &["new"]), entry(13, 10, &[])];
        let first = merged_history_page(session.iter(), &database, 0, 2, ClipboardSortMode::Manual);
        assert_eq!(first.iter().map(|item| item.id).collect::<Vec<_>>(), vec![12, -1]);
        assert_eq!(first[0].tags, vec!["new"]);
        let next = merged_history_page(session.iter(), &database, 2, 2, ClipboardSortMode::Manual);
        assert_eq!(next.iter().map(|item| item.id).collect::<Vec<_>>(), vec![13]);
    }

    #[test]
    fn equal_content_with_different_ids_is_not_removed() {
        let session = [entry(-1, 20, &[])];
        let database = [entry(12, 30, &["tag"]), entry(13, 10, &[])];
        assert_eq!(merged_history_page(session.iter(), &database, 0, 10, ClipboardSortMode::Manual).len(), 3);
    }

    #[test]
    fn recent_picks_the_newest_copy_even_below_pins() {
        let mut pin = entry(5, 10, &[]);
        pin.is_pinned = true;
        pin.pinned_order = 1;
        let database = [pin.clone(), entry(6, 20, &[])];
        let session = [entry(-1, 15, &[])];
        let top = |db: &[ClipboardEntry]| {
            merged_history_page(session.iter(), db, 0, 1, ClipboardSortMode::Recent)[0].id
        };
        assert_eq!(top(&database), 6);
        // A pin copied again is the newest copy, so it is the one pasted.
        pin.sort_at = 30;
        assert_eq!(top(&[pin, entry(6, 20, &[])]), 5);
    }

    #[test]
    fn mixed_sources_are_sorted_before_pagination() {
        let session = [8, 3, 1];
        let database = [10, 9, 7, 6, 5, 4, 2];
        let mut candidates = session.iter().chain(database.iter()).collect();
        assert_eq!(sorted_page(&mut candidates, 0, 4, |a, b| b.cmp(a)), vec![10, 9, 8, 7]);
        assert_eq!(sorted_page(&mut candidates, 4, 4, |a, b| b.cmp(a)), vec![6, 5, 4, 3]);
        assert_eq!(sorted_page(&mut candidates, 8, 4, |a, b| b.cmp(a)), vec![2, 1]);
        assert!(sorted_page(&mut candidates, 10, 4, |a, b| b.cmp(a)).is_empty());
    }
}

#[cfg(test)]
mod delete_selection_tests {
    use super::{delete_selection, ClipboardEntry, ConfirmedEntry, SELECTION_CHANGED};
    use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
    use crate::infrastructure::repository::migrations::run_migrations;
    use rusqlite::Connection;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    struct Fixture {
        root: PathBuf,
        conn: Arc<Mutex<Connection>>,
        repo: SqliteClipboardRepository,
        session: VecDeque<ClipboardEntry>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("copypaste-batch-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("attachments")).unwrap();
            let conn = Connection::open_in_memory().unwrap();
            run_migrations(&conn).unwrap();
            let conn = Arc::new(Mutex::new(conn));
            Self {
                root,
                repo: SqliteClipboardRepository::new(conn.clone()),
                conn,
                session: VecDeque::new(),
            }
        }

        fn attachment(&self, name: &str) -> PathBuf {
            let path = self.root.join("attachments").join(name);
            std::fs::write(&path, b"attachment").unwrap();
            path
        }

        fn row(&self, id: Option<i64>, created_at: i64, image: Option<&Path>) -> i64 {
            let conn = self.conn.lock().unwrap();
            let (kind, content, external) = match image {
                Some(path) => ("image", path.to_string_lossy().into_owned(), 1),
                None => ("text", format!("text {created_at}"), 0),
            };
            conn.execute(
                "INSERT INTO clipboard_history
                     (id, content_type, content, source_app, timestamp, sort_at, created_at, preview, is_external)
                 VALUES (?1, ?2, ?3, 'Test', ?4, ?4, ?4, 'preview', ?5)",
                rusqlite::params![id, kind, content, created_at, external],
            )
            .unwrap();
            conn.last_insert_rowid()
        }

        fn session_entry(&mut self, id: i64, created_at: i64) {
            self.session.push_back(
                serde_json::from_value(serde_json::json!({
                    "id": id, "content_type": "text", "content": "session", "source_app": "Test",
                    "timestamp": created_at, "created_at": created_at, "sort_at": created_at,
                    "preview": "session", "is_pinned": false,
                }))
                .unwrap(),
            );
        }

        fn ids(&self) -> Vec<i64> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM clipboard_history ORDER BY id").unwrap();
            let ids = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
            ids
        }

        fn delete(&mut self, items: &[ConfirmedEntry]) -> crate::error::AppResult<usize> {
            delete_selection(&self.repo, &mut self.session, items, &self.root)
        }

        fn sweep(&self) {
            let conn = self.conn.lock().unwrap();
            self.repo
                .sweep_attachment_deletions(&conn, &self.root, Vec::new(), self.session.iter())
                .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn confirmed(id: i64, created_at: i64) -> ConfirmedEntry {
        ConfirmedEntry { id, created_at, is_pinned: false }
    }

    #[test]
    fn a_failing_row_keeps_the_whole_batch() {
        let mut fixture = Fixture::new();
        let image = fixture.attachment("img_1.png");
        fixture.row(Some(1), 10, Some(&image));
        fixture.row(Some(2), 20, None);
        fixture.row(Some(3), 30, None);
        fixture.session_entry(-5, 40);
        // Row 3 fails mid-transaction, after rows 1 and 2 were deleted.
        fixture.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_delete BEFORE DELETE ON clipboard_history WHEN old.id = 3
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
        ).unwrap();

        let result = fixture.delete(&[confirmed(-5, 40), confirmed(1, 10), confirmed(2, 20), confirmed(3, 30)]);
        assert!(result.is_err());
        assert_eq!(fixture.ids(), vec![1, 2, 3]);
        assert_eq!(fixture.session.len(), 1);
        fixture.sweep();
        assert!(image.exists());
    }

    #[test]
    fn a_reused_id_fails_the_batch_and_the_new_entry_survives() {
        let mut fixture = Fixture::new();
        fixture.row(Some(1), 10, None);
        fixture.row(Some(2), 20, None);
        fixture.row(Some(3), 30, None);
        // Confirmed: entries 2 and 3 (the highest ID). Meanwhile 3 is deleted
        // and the next copy gets its ID back.
        let selection = [confirmed(2, 20), confirmed(3, 30)];
        fixture.conn.lock().unwrap().execute("DELETE FROM clipboard_history WHERE id = 3", []).unwrap();
        assert_eq!(fixture.row(None, 50, None), 3);

        let error = fixture.delete(&selection).unwrap_err().to_string();
        assert!(error.contains(SELECTION_CHANGED), "{error}");
        assert_eq!(fixture.ids(), vec![1, 2, 3]);
    }

    #[test]
    fn a_changed_pin_or_missing_entry_fails_the_batch() {
        let mut fixture = Fixture::new();
        fixture.row(Some(1), 10, None);
        fixture.row(Some(2), 20, None);
        fixture.session_entry(-7, 70);
        fixture.conn.lock().unwrap().execute("UPDATE clipboard_history SET is_pinned = 1 WHERE id = 2", []).unwrap();

        let pinned_changed = fixture.delete(&[confirmed(1, 10), confirmed(2, 20)]).unwrap_err();
        assert!(pinned_changed.to_string().contains(SELECTION_CHANGED));
        let missing = fixture.delete(&[confirmed(1, 10), confirmed(9, 90)]).unwrap_err();
        assert!(missing.to_string().contains(SELECTION_CHANGED));
        let session_gone = fixture.delete(&[confirmed(1, 10), confirmed(-8, 70)]).unwrap_err();
        assert!(session_gone.to_string().contains(SELECTION_CHANGED));
        assert_eq!(fixture.ids(), vec![1, 2]);
        assert_eq!(fixture.session.len(), 1);

        let pinned = ConfirmedEntry { id: 2, created_at: 20, is_pinned: true };
        assert_eq!(fixture.delete(&[confirmed(1, 10), pinned, confirmed(-7, 70)]).unwrap(), 3);
        assert!(fixture.ids().is_empty());
        assert!(fixture.session.is_empty());
    }

    #[test]
    fn attachments_go_only_when_nothing_else_references_them() {
        let mut fixture = Fixture::new();
        let alone = fixture.attachment("img_1.png");
        let kept_by_history = fixture.attachment("img_2.png");
        let kept_by_favorite = fixture.attachment("img_3.png");
        fixture.row(Some(1), 10, Some(&alone));
        fixture.row(Some(2), 20, Some(&kept_by_history));
        fixture.row(Some(3), 30, Some(&kept_by_history));
        fixture.row(Some(4), 40, Some(&kept_by_favorite));
        fixture.conn.lock().unwrap().execute(
            "INSERT INTO favorites (content_type, content, html_content, sort_order, created_at, updated_at)
             VALUES ('rich_text', 'body', ?1, 1, 1, 1)",
            [format!("<img src=\"file://{}\">", kept_by_favorite.display())],
        ).unwrap();

        // The same entry twice counts once; the count is what was deleted.
        let deleted = fixture
            .delete(&[confirmed(1, 10), confirmed(2, 20), confirmed(4, 40), confirmed(4, 40)])
            .unwrap();
        assert_eq!(deleted, 3);
        fixture.sweep();
        assert!(!alone.exists());
        assert!(kept_by_history.exists());
        assert!(kept_by_favorite.exists());
        assert_eq!(fixture.ids(), vec![3]);
    }
}

// async: off the main thread. A search reads every row of a large history
// (0.1-0.2 s at 100k entries) and froze typing and scrolling meanwhile.
#[tauri::command(async)]
pub fn search_clipboard_history(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    search_term: String,
    limit: i32,
    tag_only: Option<bool>,
    note_only: Option<bool>,
    content_type: Option<String>,
) -> AppResult<Vec<ClipboardEntry>> {
    let tag_only = tag_only.unwrap_or(false);
    let note_only = note_only.unwrap_or(false);
    let limit = limit.max(0);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let data_dir = app_data.0.lock().unwrap().clone();
    let mut history = state.repo.search_filtered(
        &search_term,
        limit,
        tag_only,
        note_only,
        content_type.as_deref(),
    )?;

    {
        let items = session.0.lock().unwrap();
        let needle = search_term.trim().to_lowercase();
        let mut ids: HashSet<i64> = history.iter().map(|item| item.id).collect();
        let mut matches: Vec<_> = items
            .iter()
            .filter(|item| matches_type(item, content_type.as_deref()))
            .filter(|item| entry_matches_search(item, &search_term, tag_only, note_only))
            .collect();
        matches.sort_by(|a, b| ClipboardSortMode::Recent.compare(a, b));
        history.extend(
            matches
                .into_iter()
                .filter(|item| item.id == 0 || ids.insert(item.id))
                .take(limit as usize)
                .map(|item| {
                    let mut result = item.clone();
                    result.ocr_match = item.content_type == "image"
                        && !needle.is_empty()
                        && !tag_only
                        && !note_only
                        && item
                            .ocr_text
                            .as_deref()
                            .is_some_and(|text| text.to_lowercase().contains(&needle));
                    result
                }),
        );
    }
    history.sort_by(|a, b| ClipboardSortMode::Recent.compare(a, b));
    history.truncate(limit as usize);
    for item in &mut history {
        prepare_list_item(item, &data_dir);
    }
    Ok(history)
}

// async for the same reason: deleting an image looks for other references
// to its file across the whole history.
#[tauri::command(async)]
pub fn delete_clipboard_entry(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    paste_queue: State<'_, PasteQueue>,
    id: i64,
    created_at: i64,
) -> AppResult<()> {
    let identity = EntryIdentity { id, created_at };
    if id > 0 {
        let data_dir = app_data.0.lock().unwrap().clone();
        if !state.repo.delete_if_current(identity, Some(&data_dir))? {
            return Err(AppError::Validation(SELECTION_CHANGED.into()));
        }
    }
    session.0.lock().unwrap().retain(|item| !identity.matches(item));
    paste_queue.0.lock().unwrap().items.retain(|item| *item != identity);
    crate::services::clipboard::sweep_session_files(&app_handle);
    let _ = app_handle.emit("clipboard-changed", ());
    Ok(())
}

/// Deletes a confirmed selection as one change. Database rows go in one
/// transaction; session entries are removed only after it committed, so a
/// failure leaves everything (rows, session entries, files) in place.
pub(crate) fn delete_selection(
    repo: &SqliteClipboardRepository,
    session: &mut VecDeque<ClipboardEntry>,
    items: &[ConfirmedEntry],
    data_dir: &std::path::Path,
) -> AppResult<usize> {
    let mut seen = HashSet::new();
    let items: Vec<ConfirmedEntry> = items
        .iter()
        .filter(|item| item.id != 0 && seen.insert(item.id))
        .copied()
        .collect();
    let changed = || AppError::Validation(SELECTION_CHANGED.to_string());
    for item in items.iter().filter(|item| item.id < 0) {
        let current = session.iter().find(|entry| entry.id == item.id).ok_or_else(changed)?;
        if current.created_at != item.created_at || current.is_pinned != item.is_pinned {
            return Err(changed());
        }
    }
    let stored: Vec<ConfirmedEntry> = items.iter().filter(|item| item.id > 0).copied().collect();
    let mut deleted = if stored.is_empty() {
        0
    } else {
        repo.delete_confirmed(&stored, Some(data_dir)).map_err(|error| {
            if error == SELECTION_CHANGED {
                changed()
            } else {
                AppError::Database(error)
            }
        })?
    };
    let session_entries = |items: &VecDeque<ClipboardEntry>| items.iter().filter(|entry| entry.id < 0).count();
    let before = session_entries(session);
    // Positive IDs too: a legacy in-memory copy of a stored entry goes with it.
    let identities: HashSet<_> = items.iter().map(ConfirmedEntry::identity).collect();
    session.retain(|entry| !identities.contains(&EntryIdentity::from_entry(entry)));
    deleted += before - session_entries(session);
    Ok(deleted)
}

/// 批量删除. `items` is the selection frozen when the user confirmed; any entry
/// that changed since (deleted, its ID reused, pinned or unpinned) fails the
/// whole call with `selection_changed`. Returns how many entries were deleted.
#[tauri::command(async)]
pub fn delete_clipboard_entries(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    paste_queue: State<'_, PasteQueue>,
    items: Vec<ConfirmedEntry>,
) -> AppResult<usize> {
    if items.is_empty() {
        return Ok(0);
    }
    let data_dir = app_data.0.lock().unwrap().clone();
    let deleted = {
        let mut session_items = session.0.lock().unwrap();
        delete_selection(&state.repo, &mut session_items, &items, &data_dir)?
    };
    {
        let identities: HashSet<_> = items.iter().map(ConfirmedEntry::identity).collect();
        paste_queue.0.lock().unwrap().items.retain(|item| !identities.contains(item));
    }
    crate::services::clipboard::sweep_session_files(&app_handle);
    let _ = app_handle.emit("clipboard-changed", ());
    Ok(deleted)
}

/// The current ID, capture time and pin state of entries that still exist,
/// so a selection that changed can be brought up to date.
#[tauri::command(async)]
pub fn get_clipboard_entry_states(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    ids: Vec<i64>,
) -> AppResult<Vec<ConfirmedEntry>> {
    let mut states: Vec<ConfirmedEntry> = {
        let session_items = session.0.lock().unwrap();
        ids.iter()
            .filter(|id| **id < 0)
            .filter_map(|id| session_items.iter().find(|entry| entry.id == *id))
            .map(|entry| ConfirmedEntry {
                id: entry.id,
                created_at: entry.created_at,
                is_pinned: entry.is_pinned,
            })
            .collect()
    };
    let stored: Vec<i64> = ids.into_iter().filter(|id| *id > 0).collect();
    states.extend(state.repo.entry_states(&stored).map_err(AppError::Database)?);
    Ok(states)
}

// async: clearing a large history takes seconds.
#[tauri::command(async)]
pub fn clear_clipboard_history(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
) -> AppResult<()> {
    let data_dir = app_data.0.lock().unwrap().clone();
    state.repo.clear(Some(&data_dir))?;
    {
        let mut session_items = session.inner().0.lock().unwrap();
        session_items.retain(|item| item.is_pinned || !item.tags.is_empty());
    }
    crate::services::clipboard::sweep_session_files(&app_handle);
    let _ = app_handle.emit("clipboard-changed", ());
    Ok(())
}

#[tauri::command(async)]
pub fn get_tag_items(state: State<'_, DbState>, tag: String) -> AppResult<Vec<ClipboardEntry>> {
    let mut history = state
        .tag_repo
        .get_entries_by_tag(&tag)
        .map_err(AppError::from)?;

    for item in &mut history {
        if (item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text")
            && item.content.chars().count() > 50000
        {
            item.content = format!(
                "{}... [Content Truncated]",
                item.content.chars().take(50000).collect::<String>()
            );
        }
    }

    Ok(history)
}

#[tauri::command]
pub fn get_all_tags_info(
    state: State<'_, DbState>,
) -> AppResult<std::collections::HashMap<String, i32>> {
    state.tag_repo.get_all_with_counts().map_err(AppError::from)
}

#[tauri::command]
pub fn rename_tag_globally(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    old_name: String,
    new_name: String,
) -> AppResult<()> {
    {
        let mut session_items = session.inner().0.lock().unwrap();
        for item in session_items.iter_mut() {
            for tag in item.tags.iter_mut() {
                if *tag == old_name {
                    *tag = new_name.clone();
                }
            }
            item.tags.sort();
            item.tags.dedup();
        }
    }

    state
        .tag_repo
        .rename(&old_name, &new_name)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn delete_tag_from_all(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    tag_name: String,
) -> AppResult<()> {
    // Deleting a tag removes it from entries; the entries themselves stay.
    {
        let mut session_items = session.inner().0.lock().unwrap();
        for item in session_items.iter_mut() {
            item.tags.retain(|tag| tag != &tag_name);
        }
    }

    state
        .tag_repo
        .delete_globally(&tag_name)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn create_new_tag(state: State<'_, DbState>, tag_name: String) -> AppResult<()> {
    state.tag_repo.create(&tag_name).map_err(AppError::from)
}

#[tauri::command]
pub fn get_clipboard_content(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
    created_at: Option<i64>,
) -> AppResult<String> {
    if id <= 0 {
        let session_items = session.inner().0.lock().unwrap();
        if let Some(item) = session_items.iter().find(|item| {
            item.id == id && created_at.is_none_or(|expected| item.created_at == expected)
        }) {
            return Ok(item.content.clone());
        }
    }

    state
        .repo
        .get_entry_content_at(id, created_at)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::Validation("Entry not found".to_string()))
}



