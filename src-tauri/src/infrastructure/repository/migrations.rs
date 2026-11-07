use rusqlite::{params, Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    let current_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Migration 1: Initial Baseline
    if current_version < 1 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                source_app TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                preview TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (1)", [])?;
    }

    // Migration 2: Add core feature columns
    if current_version < 2 {
        let columns = [
            ("is_pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("tags", "TEXT NOT NULL DEFAULT '[]'"),
            ("use_count", "INTEGER NOT NULL DEFAULT 0"),
            ("pinned_order", "INTEGER NOT NULL DEFAULT 0"),
            ("content_hash", "INTEGER NOT NULL DEFAULT 0"),
            ("html_content", "TEXT"),
        ];

        for (name, def) in columns {
            if !has_column(conn, "clipboard_history", name)? {
                conn.execute(
                    &format!("ALTER TABLE clipboard_history ADD COLUMN {} {}", name, def),
                    [],
                )?;
            }
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (2)", [])?;
    }

    // Migration 3: Add is_external
    if current_version < 3 {
        if !has_column(conn, "clipboard_history", "is_external")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN is_external INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (3)", [])?;
    }

    // Migration 4: Tag management
    if current_version < 4 {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS saved_tags (
                name TEXT PRIMARY KEY,
                color TEXT
            )",
            [],
        )?;

        // Insert default tags
        let _ = conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name) VALUES ('sensitive')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR IGNORE INTO saved_tags (name) VALUES ('密码')",
            [],
        );

        conn.execute("INSERT INTO schema_migrations (version) VALUES (4)", [])?;
    }

    // Migration 5: Performance indexes
    if current_version < 5 {
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_pinned_order_time
                ON clipboard_history (is_pinned, pinned_order, timestamp);
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_type_hash
                ON clipboard_history (content_type, content_hash);
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_timestamp
                ON clipboard_history (timestamp);
        ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (5)", [])?;
    }

    // Migration 6: Normalize tags into entry_tags
    if current_version < 6 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS entry_tags (
                entry_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags (tag);
            CREATE INDEX IF NOT EXISTS idx_entry_tags_entry ON entry_tags (entry_id);
        ",
        )?;

        // Backfill entry_tags from clipboard_history.tags JSON
        conn.execute("BEGIN", [])?;
        let backfill = (|| -> Result<()> {
            let mut stmt = conn.prepare("SELECT id, tags FROM clipboard_history")?;
            let rows = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let tags: Option<String> = row.get(1)?;
                Ok((id, tags.unwrap_or_else(|| "[]".to_string())))
            })?;

            for row in rows {
                let (id, tags_json) = row?;
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                for tag in tags {
                    if tag.trim().is_empty() {
                        continue;
                    }
                    conn.execute(
                        "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                        params![id, tag],
                    )?;
                }
            }
            Ok(())
        })();

        if let Err(err) = backfill {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
        conn.execute("COMMIT", [])?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (6)", [])?;
    }

    // Migration 7: Cloud sync tombstones for deletion propagation
    if current_version < 7 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cloud_sync_tombstones (
                content_type TEXT NOT NULL,
                content_hash INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL,
                PRIMARY KEY (content_type, content_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_cloud_sync_tombstones_deleted_at
                ON cloud_sync_tombstones (deleted_at);
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (7)", [])?;
    }

    // Migration 8: Local incremental sync index (for delta diff against last uploaded state)
    if current_version < 8 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cloud_sync_local_index (
                sync_key TEXT PRIMARY KEY,
                digest TEXT NOT NULL
            );
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (8)", [])?;
    }

    // Migration 9: Persist source app path for source icon rendering
    if current_version < 9 {
        if !has_column(conn, "clipboard_history", "source_app_path")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN source_app_path TEXT",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (9)", [])?;
    }

    // Migration 10: Cloud sync content type preferences (per-device; not synced in settings snapshot)
    if current_version < 10 {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('cloud_sync_content_prefs', '{\"text\":true,\"image\":true,\"file_path\":true,\"emoji\":true}')",
            [],
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (10)", [])?;
    }

    // Migration 11: Per-entry free-text note (separate from category tags)
    if current_version < 11 {
        if !has_column(conn, "clipboard_history", "note")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN note TEXT NOT NULL DEFAULT ''",
                [],
            )?;
            eprintln!("[migrations] added clipboard_history.note column");
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (11)", [])?;
    }

    // Safety net: older builds may have queried `note` before migration 11 ran,
    // or schema_migrations can drift. Always ensure the column exists.
    if !has_column(conn, "clipboard_history", "note")? {
        conn.execute(
            "ALTER TABLE clipboard_history ADD COLUMN note TEXT NOT NULL DEFAULT ''",
            [],
        )?;
        eprintln!("[migrations] repaired missing clipboard_history.note column");
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version) VALUES (11)",
            [],
        )?;
    }

    // Migration 12: Cover the exact list ordering used by paginated history.
    // Including `id` also makes entries with identical timestamps deterministic.
    if current_version < 12 {
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_list_order
                ON clipboard_history (
                    is_pinned DESC,
                    pinned_order DESC,
                    timestamp DESC,
                    id DESC
                );
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_type_list_order
                ON clipboard_history (
                    content_type,
                    is_pinned DESC,
                    pinned_order DESC,
                    timestamp DESC,
                    id DESC
                );
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (12)", [])?;
    }

    // Migration 13: Structured per-entry shortcut and usage metadata.
    // These fields are intentionally separate from the user-facing note.
    if current_version < 13 {
        let columns = [
            ("item_hotkey", "TEXT NOT NULL DEFAULT ''"),
            ("item_hotkey_global", "INTEGER NOT NULL DEFAULT 0"),
            ("last_used_at", "INTEGER NOT NULL DEFAULT 0"),
            ("move_to_group_hotkey", "TEXT NOT NULL DEFAULT ''"),
            ("move_to_group_hotkey_global", "INTEGER NOT NULL DEFAULT 0"),
        ];
        for (name, definition) in columns {
            if !has_column(conn, "clipboard_history", name)? {
                conn.execute(
                    &format!("ALTER TABLE clipboard_history ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_last_used_at
                ON clipboard_history (last_used_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_item_hotkey
                ON clipboard_history (item_hotkey)
                WHERE item_hotkey <> '';
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (13)", [])?;
    }

    // Migration 14: Split immutable creation time from mutable list ordering.
    // Existing `timestamp` values are the best available historical value, so
    // use them for both fields to preserve the exact pre-upgrade list order.
    if current_version < 14 {
        if !has_column(conn, "clipboard_history", "created_at")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !has_column(conn, "clipboard_history", "sort_at")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN sort_at INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.execute(
            "UPDATE clipboard_history SET
                created_at = CASE WHEN created_at <= 0 THEN timestamp ELSE created_at END,
                sort_at = CASE WHEN sort_at <= 0 THEN timestamp ELSE sort_at END",
            [],
        )?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_sort_order
                ON clipboard_history (
                    is_pinned DESC,
                    pinned_order DESC,
                    sort_at DESC,
                    id DESC
                );
            CREATE INDEX IF NOT EXISTS idx_clipboard_history_type_sort_order
                ON clipboard_history (
                    content_type,
                    is_pinned DESC,
                    pinned_order DESC,
                    sort_at DESC,
                    id DESC
                );
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (14)", [])?;
    }

    // Migration 15: when an entry's tags were last edited, so cloud sync can
    // keep the newer edit instead of letting any remote copy overwrite it.
    if current_version < 15 {
        if !has_column(conn, "clipboard_history", "tags_updated_at")? {
            conn.execute(
                "ALTER TABLE clipboard_history ADD COLUMN tags_updated_at INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (15)", [])?;
    }

    // Migration 16: Windows 0.3.x stored the sound volume as 0-100; it is 0-1
    // now. Left alone, 70 shows as 7000% and plays at full volume.
    if current_version < 16 {
        let has_settings: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
            [],
            |row| row.get(0),
        )?;
        if has_settings {
            conn.execute(
                "UPDATE settings SET value = CAST(MIN(CAST(value AS REAL), 100.0) / 100.0 AS TEXT)
                 WHERE key = 'app.sound_volume' AND CAST(value AS REAL) > 1.0",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (16)", [])?;
    }

    // Migration 17: rich text previews show the formatted text itself now; the
    // snapshot image (default on until now, marked experimental) shrank long
    // content until it was unreadable. Everyone starts with it off once; it
    // was on by default, so a deliberate choice can't be told apart.
    if current_version < 17 {
        let has_settings: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
            [],
            |row| row.get(0),
        )?;
        if has_settings {
            conn.execute(
                "UPDATE settings SET value = 'false' WHERE key = 'app.rich_text_snapshot_preview'",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (17)", [])?;
    }

    // Migration 18: the default paste method "shift_insert" already switched
    // keys per target app on Windows; that is now the explicit "auto", and
    // "shift_insert" always sends Shift+Insert. It was the default, so a
    // deliberate choice can't be told apart: everyone on it moves to auto and
    // keeps the behaviour they had.
    if current_version < 18 {
        let has_settings: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
            [],
            |row| row.get(0),
        )?;
        if has_settings {
            conn.execute(
                "UPDATE settings SET value = 'auto' WHERE key = 'app.paste_method' AND value = 'shift_insert'",
                [],
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (18)", [])?;
    }

    // Migration 19: a click now pastes with the format, so the "paste with
    // format" hotkey is gone and the plain text one takes its keys. A plain
    // hotkey the user set stays; otherwise it gets the rich one's (even an
    // empty one, so the new default doesn't bind keys the user cleared).
    if current_version < 19 {
        let has_settings: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
            [],
            |row| row.get(0),
        )?;
        if has_settings {
            conn.execute(
                "INSERT INTO settings (key, value)
                 SELECT 'app.plain_paste_hotkey', value FROM settings WHERE key = 'app.rich_paste_hotkey'
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value WHERE trim(settings.value) = ''",
                [],
            )?;
            conn.execute("DELETE FROM settings WHERE key = 'app.rich_paste_hotkey'", [])?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (19)", [])?;
    }

    // Migration 20: search, tag and note views and the encrypted-entry scan
    // list newest first by (sort_at, id). Without an index on that order
    // SQLite sorted every match before taking the first page, and each batch
    // of the encrypted scan re-read the whole table. With it, a search stops
    // once the page is full.
    if current_version < 20 {
        if has_column(conn, "clipboard_history", "sort_at")? {
            conn.execute_batch(
                "
                CREATE INDEX IF NOT EXISTS idx_clipboard_history_recent
                    ON clipboard_history (sort_at DESC, id DESC);
                ",
            )?;
        }
        conn.execute("INSERT INTO schema_migrations (version) VALUES (20)", [])?;
    }

    // Migration 21: OCR text of images, and 常用 (kept apart from the history so
    // automatic cleanup can never reach it).
    if current_version < 21 {
        if !has_column(conn, "clipboard_history", "ocr_text")? {
            conn.execute_batch("ALTER TABLE clipboard_history ADD COLUMN ocr_text TEXT;")?;
        }
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS favorites (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                html_content TEXT,
                preview TEXT NOT NULL DEFAULT '',
                source_app TEXT NOT NULL DEFAULT '',
                sort_order INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_favorites_order ON favorites (sort_order);
            ",
        )?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (21)", [])?;
    }

    Ok(())
}

fn has_column(conn: &Connection, table_name: &str, column_name: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table_name))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_14_preserves_existing_order_and_backfills_creation_time() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(
            "
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO schema_migrations(version) VALUES (12);
            CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY,
                content_type TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                pinned_order INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO clipboard_history(id, content_type, timestamp)
                VALUES
                    (1, 'text', 1700000000123),
                    (2, 'text', 1700000000456);
            ",
        )
        .expect("create version 12 fixture");

        run_migrations(&conn).expect("run update migrations");

        let values: (i64, i64, i64) = conn
            .query_row(
                "SELECT created_at, sort_at, last_used_at
                 FROM clipboard_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read migrated fields");
        assert_eq!(values, (1_700_000_000_123, 1_700_000_000_123, 0));
        let migrated_order: Vec<i64> = conn
            .prepare("SELECT id FROM clipboard_history ORDER BY sort_at DESC, id DESC")
            .expect("prepare migrated order query")
            .query_map([], |row| row.get(0))
            .expect("query migrated order")
            .collect::<Result<Vec<_>>>()
            .expect("collect migrated order");
        assert_eq!(migrated_order, vec![2, 1]);
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read schema version");
        assert_eq!(version, 21);
    }

    #[test]
    fn newest_first_queries_walk_the_recent_index_instead_of_sorting() {
        let conn = Connection::open_in_memory().expect("open database");
        run_migrations(&conn).expect("run migrations");
        let plan = |sql: &str| -> String {
            conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .expect("prepare plan")
                .query_map([], |row| row.get::<_, String>(3))
                .expect("query plan")
                .collect::<Result<Vec<_>>>()
                .expect("collect plan")
                .join(" | ")
        };

        // Shaped like the search query: a LIKE nothing can index, newest first.
        let search = plan(
            "SELECT ch.id FROM clipboard_history ch
             WHERE ch.content LIKE '%' || 'x' || '%' OR ch.note LIKE '%' || 'x' || '%'
             ORDER BY ch.sort_at DESC, ch.id DESC LIMIT 200",
        );
        assert!(search.contains("idx_clipboard_history_recent"), "{search}");
        assert!(!search.contains("TEMP B-TREE"), "{search}");

        // The encrypted-entry scan pages with a (sort_at, id) cursor.
        let batch = plan(
            "SELECT ch.id FROM clipboard_history ch
             WHERE ch.content LIKE 'enc1:%'
               AND ((ch.sort_at < 5) OR (ch.sort_at = 5 AND ch.id < 9))
             ORDER BY ch.sort_at DESC, ch.id DESC LIMIT 500",
        );
        assert!(batch.contains("idx_clipboard_history_recent (sort_at<?)"), "{batch}");
        assert!(!batch.contains("TEMP B-TREE"), "{batch}");
    }

    #[test]
    fn migration_16_rescales_percent_sound_volume() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(
            "
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO schema_migrations(version) VALUES (15);
            CREATE TABLE clipboard_history (id INTEGER PRIMARY KEY, note TEXT NOT NULL DEFAULT '');
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO settings(key, value) VALUES ('app.sound_volume', '70');
            ",
        )
        .expect("create version 15 fixture");

        run_migrations(&conn).expect("run update migrations");

        let volume = |conn: &Connection| -> f64 {
            conn.query_row(
                "SELECT value FROM settings WHERE key = 'app.sound_volume'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read volume")
            .parse()
            .expect("volume is a number")
        };
        assert!((volume(&conn) - 0.7).abs() < 1e-9);

        // Already on the 0-1 scale: untouched.
        conn.execute("UPDATE settings SET value = '0.5' WHERE key = 'app.sound_volume'", [])
            .unwrap();
        conn.execute("DELETE FROM schema_migrations WHERE version >= 16", []).unwrap();
        run_migrations(&conn).expect("rerun migration 16");
        assert!((volume(&conn) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn migration_18_moves_the_old_default_paste_method_to_auto() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(
            "
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO schema_migrations(version) VALUES (17);
            CREATE TABLE clipboard_history (id INTEGER PRIMARY KEY, note TEXT NOT NULL DEFAULT '');
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO settings(key, value) VALUES ('app.paste_method', 'shift_insert');
            ",
        )
        .expect("create version 17 fixture");
        let method = |conn: &Connection| -> String {
            conn.query_row("SELECT value FROM settings WHERE key = 'app.paste_method'", [], |row| row.get(0))
                .expect("read paste method")
        };

        run_migrations(&conn).expect("run update migrations");
        assert_eq!(method(&conn), "auto");

        // Other choices are kept.
        for kept in ["ctrl_v", "game_mode"] {
            conn.execute("UPDATE settings SET value = ?1 WHERE key = 'app.paste_method'", [kept]).unwrap();
            conn.execute("DELETE FROM schema_migrations WHERE version >= 18", []).unwrap();
            run_migrations(&conn).expect("rerun migration 18");
            assert_eq!(method(&conn), kept);
        }
    }

    #[test]
    fn migration_19_gives_the_rich_paste_hotkey_keys_to_plain_paste() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(
            "
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO schema_migrations(version) VALUES (18);
            CREATE TABLE clipboard_history (id INTEGER PRIMARY KEY, note TEXT NOT NULL DEFAULT '');
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            ",
        )
        .expect("create version 18 fixture");
        let setting = |conn: &Connection, key: &str| -> Option<String> {
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0)).ok()
        };
        let rerun = |conn: &Connection, rich: Option<&str>, plain: Option<&str>| {
            conn.execute("DELETE FROM settings", []).unwrap();
            for (key, value) in [("app.rich_paste_hotkey", rich), ("app.plain_paste_hotkey", plain)] {
                if let Some(value) = value {
                    conn.execute("INSERT INTO settings(key, value) VALUES (?1, ?2)", [key, value]).unwrap();
                }
            }
            conn.execute("DELETE FROM schema_migrations WHERE version >= 19", []).unwrap();
            run_migrations(conn).expect("run migration 19");
            assert_eq!(setting(conn, "app.rich_paste_hotkey"), None);
            setting(conn, "app.plain_paste_hotkey")
        };

        // Plain hotkey never set (no row) or cleared: it takes the rich keys.
        assert_eq!(rerun(&conn, Some("Alt+Shift+V"), None).as_deref(), Some("Alt+Shift+V"));
        assert_eq!(rerun(&conn, Some("Ctrl+Alt+V"), Some("")).as_deref(), Some("Ctrl+Alt+V"));
        // A cleared rich hotkey stays cleared instead of getting the new default.
        assert_eq!(rerun(&conn, Some(""), None).as_deref(), Some(""));
        // A plain hotkey the user set wins.
        assert_eq!(rerun(&conn, Some("Alt+Shift+V"), Some("Ctrl+Shift+B")).as_deref(), Some("Ctrl+Shift+B"));
        // Nothing to move on a fresh database: the seed adds the default.
        assert_eq!(rerun(&conn, None, None), None);
    }
}
