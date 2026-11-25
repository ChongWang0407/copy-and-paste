use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::database::DbState;
use crate::infrastructure::repository::{
    clipboard_repo::SqliteClipboardRepository, favorites_repo,
};

pub fn spawn_sensitive_alignment(app_handle: AppHandle) {
    if cfg!(feature = "portable") {
        return;
    }
    std::thread::spawn(move || {
        let state = app_handle.state::<DbState>();
        let conn = state.conn.lock().unwrap();
        if let Err(error) = favorites_repo::repair_sensitive_favorites(&conn) {
            crate::error!(">>> [ENCRYPTION] Favorite repair: {error}");
            return;
        }
        if let Err(error) = align(&conn, &state.repo) {
            crate::error!(">>> [ENCRYPTION] History alignment: {error}");
        }
    });
}

fn align(conn: &Connection, repo: &SqliteClipboardRepository) -> Result<(), String> {
    const MIGRATION: &str = "sensitive-alignment-v2";
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let complete = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM fork_migrations WHERE name = ?)",
            [MIGRATION],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|e| e.to_string())?;
    if complete {
        return Ok(());
    }
    let pending = {
        let mut stmt = tx.prepare(
            "SELECT id, sensitive FROM (
                SELECT ch.id, content, preview, html_content,
                       EXISTS(SELECT 1 FROM entry_tags WHERE entry_id = ch.id
                              AND tag COLLATE NOCASE IN ('sensitive', '密码', 'password')) AS sensitive
                FROM clipboard_history ch
             )
             WHERE (sensitive AND (content NOT GLOB 'enc1:*' OR preview NOT GLOB 'enc1:*'
                   OR (html_content IS NOT NULL AND html_content NOT GLOB 'enc1:*')))
                OR (NOT sensitive AND (content GLOB 'enc1:*' OR preview GLOB 'enc1:*' OR html_content GLOB 'enc1:*'
                   OR content GLOB 'plain:*' OR preview GLOB 'plain:*' OR html_content GLOB 'plain:*'
                   OR content GLOB 'dpapi:*' OR preview GLOB 'dpapi:*' OR html_content GLOB 'dpapi:*'))",
        ).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    for (id, sensitive) in pending {
        if sensitive {
            repo.encrypt_entry_with_conn(&tx, id)?;
        } else {
            repo.decrypt_entry_with_conn(&tx, id)?;
        }
    }
    tx.execute("INSERT INTO fork_migrations (name) VALUES (?)", [MIGRATION])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(all(test, not(feature = "portable")))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn alignment_rolls_back_every_update_and_leaves_the_marker_unset_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview)
             VALUES (1, 'text', 'sensitive text', 'Notes', 1, 'sensitive preview'),
                    (2, 'text', 'dpapi:unreadable', 'Notes', 2, 'unreadable preview');
             INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'sensitive'), (2, 'sensitive');",
        ).unwrap();
        let conn = Arc::new(Mutex::new(conn));
        let repo = SqliteClipboardRepository::new(conn.clone());
        let conn = conn.lock().unwrap();
        assert!(align(&conn, &repo).is_err());
        let content = conn.query_row("SELECT content FROM clipboard_history WHERE id = 1", [], |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(content, "sensitive text");
        assert_eq!(conn.query_row("SELECT count(*) FROM fork_migrations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
}
