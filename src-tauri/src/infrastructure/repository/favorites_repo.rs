//! 常用: kept apart from `clipboard_history`, so no automatic cleanup reaches it.

use rusqlite::{params, Connection, OptionalExtension, Result, Row};
use serde::Serialize;
use crate::infrastructure::encryption;

#[derive(Debug, Clone, Serialize)]
pub struct Favorite {
    pub id: i64,
    /// text / rich_text / code / url / image / video / file
    pub content_type: String,
    /// The text, or for media files paths relative to `<data>/favorites`.
    pub content: String,
    pub html_content: Option<String>,
    pub preview: String,
    pub source_app: String,
    pub sort_order: i64,
    pub created_at: i64,
    #[serde(skip)]
    pub is_encrypted: bool,
    /// For lookup only, never pasted. Empty when unset.
    pub title: String,
    pub tags: Vec<String>,
}

pub struct NewFavorite<'a> {
    pub content_type: &'a str,
    pub content: &'a str,
    pub html_content: Option<&'a str>,
    pub preview: &'a str,
    pub source_app: &'a str,
}

const COLUMNS: &str = "f.id, f.content_type, f.content, f.html_content, f.preview, f.source_app, f.sort_order,
    f.created_at, f.is_encrypted, COALESCE(m.title, ''), COALESCE(m.tags, '[]')";
/// A meta row counts only while its favorite's creation time matches: another
/// build may have deleted the favorite and SQLite may reuse the id.
const FROM: &str = "favorites f LEFT JOIN cp_fork_favorite_meta m
    ON m.favorite_id = f.id AND m.favorite_created_at = f.created_at";

pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_TAG_CHARS: usize = 24;
pub const MAX_TAGS: usize = 8;

/// Trim, then cut to `max` code points.
fn clip(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect::<String>().trim_end().to_string()
}

pub fn normalize_title(title: &str) -> String {
    clip(title, MAX_TITLE_CHARS)
}

/// No empties; the first spelling of a case-insensitive repeat wins.
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    tags.iter()
        .map(|tag| clip(tag, MAX_TAG_CHARS))
        .filter(|tag| !tag.is_empty() && seen.insert(tag.to_lowercase()))
        .take(MAX_TAGS)
        .collect()
}

fn crypto_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
}

fn from_row(row: &Row<'_>) -> Result<Favorite> {
    let mut favorite = Favorite {
        id: row.get(0)?,
        content_type: row.get(1)?,
        content: row.get(2)?,
        html_content: row.get(3)?,
        preview: row.get(4)?,
        source_app: row.get(5)?,
        sort_order: row.get(6)?,
        created_at: row.get(7)?,
        is_encrypted: row.get(8)?,
        title: row.get(9)?,
        tags: Vec::new(),
    };
    let mut tags: String = row.get(10)?;
    if favorite.is_encrypted {
        let decode = |value: &str| encryption::decrypt_stored_value(value).map_err(crypto_error);
        favorite.content = decode(&favorite.content)?;
        favorite.preview = decode(&favorite.preview)?;
        favorite.html_content = favorite.html_content.as_deref().map(decode).transpose()?;
        favorite.title = decode(&favorite.title)?;
        tags = decode(&tags)?;
    }
    // Unreadable tags are dropped rather than hiding the favorite.
    favorite.tags = serde_json::from_str(&tags).unwrap_or_default();
    Ok(favorite)
}

pub fn list(conn: &Connection) -> Result<Vec<Favorite>> {
    conn.execute(
        "DELETE FROM cp_fork_favorite_meta WHERE NOT EXISTS (SELECT 1 FROM favorites f
         WHERE f.id = favorite_id AND f.created_at = favorite_created_at)",
        [],
    )?;
    let sql = format!("SELECT {COLUMNS} FROM {FROM} ORDER BY f.sort_order ASC, f.id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Favorite>> {
    let sql = format!("SELECT {COLUMNS} FROM {FROM} WHERE f.id = ?1");
    conn.query_row(&sql, params![id], from_row).optional()
}

/// Title and tags, normalized and encrypted like the favorite's content.
/// Nothing left to keep removes the row.
pub fn set_meta(conn: &Connection, favorite: &Favorite, title: &str, tags: &[String], now: i64) -> Result<()> {
    let title = normalize_title(title);
    let tags = normalize_tags(tags);
    if title.is_empty() && tags.is_empty() {
        conn.execute("DELETE FROM cp_fork_favorite_meta WHERE favorite_id = ?1", [favorite.id])?;
        return Ok(());
    }
    let json = serde_json::to_string(&tags).map_err(|error| crypto_error(error.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO cp_fork_favorite_meta
            (favorite_id, favorite_created_at, title, tags, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            favorite.id,
            favorite.created_at,
            storage_value(&title, favorite.is_encrypted)?,
            storage_value(&json, favorite.is_encrypted)?,
            now
        ],
    )?;
    Ok(())
}

/// Text already kept: adding it again keeps the one entry.
pub fn find_text(conn: &Connection, content_type: &str, content: &str) -> Result<Option<Favorite>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM {FROM} WHERE f.content_type = ?1
        AND (f.content = ?2 OR f.is_encrypted = 1) ORDER BY f.is_encrypted DESC, f.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![content_type, content], from_row)?;
    for favorite in rows {
        let favorite = favorite?;
        if favorite.content == content {
            return Ok(Some(favorite));
        }
    }
    Ok(None)
}

/// New favorites go to the end of the list.
pub fn insert(conn: &Connection, new: &NewFavorite<'_>, now: i64) -> Result<Favorite> {
    insert_with_protection(conn, new, now, false)
}

fn storage_value(value: &str, protected: bool) -> Result<String> {
    if protected {
        return encryption::encrypt_value(value)
            .ok_or_else(|| crypto_error("favorite encryption unavailable".into()));
    }
    Ok(value.into())
}

pub fn insert_with_protection(
    conn: &Connection,
    new: &NewFavorite<'_>,
    now: i64,
    protected: bool,
) -> Result<Favorite> {
    let encode = |value: &str| storage_value(value, protected);
    let content = encode(new.content)?;
    let preview = encode(new.preview)?;
    let html = new.html_content.map(encode).transpose()?;
    let order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM favorites",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO favorites (content_type, content, html_content, preview, source_app, sort_order, created_at, updated_at, is_encrypted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        params![new.content_type, content, html, preview, new.source_app, order, now, protected],
    )?;
    let id = conn.last_insert_rowid();
    // A reused id must not inherit a deleted favorite's title and tags.
    conn.execute("DELETE FROM cp_fork_favorite_meta WHERE favorite_id = ?1", [id])?;
    Ok(get(conn, id)?.expect("just inserted"))
}

pub fn protect(conn: &Connection, favorite: &mut Favorite) -> Result<()> {
    if !favorite.is_encrypted {
        let html = favorite
            .html_content
            .as_deref()
            .map(|value| storage_value(value, true))
            .transpose()?;
        conn.execute(
            "UPDATE favorites SET content = ?1, preview = ?2, html_content = ?3, is_encrypted = 1 WHERE id = ?4",
            params![storage_value(&favorite.content, true)?, storage_value(&favorite.preview, true)?, html, favorite.id],
        )?;
        favorite.is_encrypted = true;
        if !favorite.title.is_empty() || !favorite.tags.is_empty() {
            let (title, tags) = (favorite.title.clone(), favorite.tags.clone());
            set_meta(conn, favorite, &title, &tags, chrono::Utc::now().timestamp_millis())?;
        }
    }
    Ok(())
}

const PROTECTED_SOURCE: &str =
    "(content GLOB 'enc1:*' OR preview GLOB 'enc1:*' OR html_content GLOB 'enc1:*'
      OR content GLOB 'plain:*' OR preview GLOB 'plain:*' OR html_content GLOB 'plain:*'
      OR content GLOB 'dpapi:*' OR preview GLOB 'dpapi:*' OR html_content GLOB 'dpapi:*'
      OR EXISTS(SELECT 1 FROM entry_tags WHERE entry_id = ch.id
                AND tag COLLATE NOCASE IN ('sensitive', '密码', 'password')))";

pub fn is_sensitive_source(conn: &Connection, id: i64) -> Result<bool> {
    conn.query_row(
        &format!("SELECT COALESCE({PROTECTED_SOURCE}, 0) FROM clipboard_history ch WHERE id = ?1"),
        [id],
        |row| row.get(0),
    )
}

/// Old favorites stored no source ID. Repair rows whose original source
/// fields match a still-present sensitive entry, including the full format.
pub(crate) fn repair_sensitive_favorites(conn: &Connection) -> Result<()> {
    if cfg!(feature = "portable") {
        return Ok(());
    }
    const MIGRATION: &str = "sensitive-favorites-v1";
    let tx = conn.unchecked_transaction()?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM fork_migrations WHERE name = ?)",
        [MIGRATION],
        |row| row.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let favorites = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {COLUMNS} FROM {FROM} WHERE f.is_encrypted = 0
             AND f.content_type IN ('text', 'rich_text', 'code', 'url')"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for mut favorite in favorites {
        let protected = {
            let sql = format!(
                "SELECT content, preview, html_content FROM clipboard_history ch
                 WHERE content_type = ?1 AND source_app = ?2 AND created_at <= ?3 AND {PROTECTED_SOURCE}"
            );
            let mut stmt = tx.prepare(&sql)?;
            let mut rows = stmt.query(params![
                favorite.content_type,
                favorite.source_app,
                favorite.created_at
            ])?;
            let mut matched = false;
            while let Some(row) = rows.next()? {
                let decode =
                    |value: &str| encryption::decrypt_stored_value(value).map_err(crypto_error);
                let content: String = row.get(0)?;
                if decode(&content)? != favorite.content {
                    continue;
                }
                let preview: String = row.get(1)?;
                let html: Option<String> = row.get(2)?;
                if decode(&preview)? == favorite.preview
                    && html.as_deref().map(decode).transpose()? == favorite.html_content
                {
                    matched = true;
                    break;
                }
            }
            matched
        };
        if protected {
            protect(&tx, &mut favorite)?;
        }
    }
    tx.execute("INSERT INTO fork_migrations (name) VALUES (?)", [MIGRATION])?;
    tx.commit()
}

/// Editing makes it plain text, like editing a history entry.
pub fn update_text(
    conn: &Connection,
    id: i64,
    content_type: &str,
    content: &str,
    preview: &str,
    now: i64,
) -> Result<()> {
    let Some(protected) = conn
        .query_row(
            "SELECT is_encrypted FROM favorites WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    conn.execute(
        "UPDATE favorites SET content_type = ?1, content = ?2, html_content = NULL, preview = ?3, updated_at = ?4 WHERE id = ?5",
        params![content_type, storage_value(content, protected)?, storage_value(preview, protected)?, now, id],
    )?;
    Ok(())
}

/// Run it in a transaction: the favorite and its title and tags go together.
pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM favorites WHERE id = ?1", params![id])?;
    conn.execute("DELETE FROM cp_fork_favorite_meta WHERE favorite_id = ?1", params![id])?;
    Ok(())
}

pub fn reorder(conn: &mut Connection, ids: &[i64]) -> Result<()> {
    let tx = conn.transaction()?;
    for (index, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE favorites SET sort_order = ?1 WHERE id = ?2",
            params![index as i64 + 1, id],
        )?;
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        conn
    }

    fn text<'a>(content: &'a str) -> NewFavorite<'a> {
        NewFavorite {
            content_type: "text",
            content,
            html_content: None,
            preview: content,
            source_app: "",
        }
    }

    #[test]
    fn keeps_manual_order() {
        let mut conn = db();
        let a = insert(&conn, &text("a"), 1).unwrap();
        let b = insert(&conn, &text("b"), 2).unwrap();
        let c = insert(&conn, &text("c"), 3).unwrap();
        assert_eq!(list(&conn).unwrap().iter().map(|f| f.id).collect::<Vec<_>>(), vec![a.id, b.id, c.id]);

        reorder(&mut conn, &[c.id, a.id, b.id]).unwrap();
        assert_eq!(list(&conn).unwrap().iter().map(|f| f.id).collect::<Vec<_>>(), vec![c.id, a.id, b.id]);

        let d = insert(&conn, &text("d"), 4).unwrap();
        assert_eq!(list(&conn).unwrap().last().unwrap().id, d.id);
    }

    #[test]
    fn editing_drops_the_rich_format() {
        let conn = db();
        let rich = NewFavorite { content_type: "rich_text", content: "a", html_content: Some("<b>a</b>"), preview: "a", source_app: "" };
        let favorite = insert(&conn, &rich, 1).unwrap();
        update_text(&conn, favorite.id, "text", "b", "b", 2).unwrap();
        let edited = get(&conn, favorite.id).unwrap().unwrap();
        assert_eq!(edited.content, "b");
        assert_eq!(edited.content_type, "text");
        assert!(edited.html_content.is_none());
    }

    #[cfg(not(feature = "portable"))]
    #[test]
    fn sensitive_favorites_stay_encrypted_when_read_deduplicated_and_edited() {
        let conn = db();
        let rich = NewFavorite {
            content_type: "rich_text", content: "secret", html_content: Some("<b>secret</b>"),
            preview: "secret preview", source_app: "Notes",
        };
        let saved = insert_with_protection(&conn, &rich, 20, true).unwrap();
        let raw = |column: &str| conn.query_row(
            &format!("SELECT {column} FROM favorites WHERE id = ?"), [saved.id], |row| row.get::<_, String>(0),
        ).unwrap();
        for (column, value) in [("content", "secret"), ("preview", "secret preview"), ("html_content", "<b>secret</b>")] {
            let cipher = raw(column);
            assert!(encryption::is_modern_ciphertext(&cipher));
            assert_eq!(encryption::decrypt_stored_value(&cipher).unwrap(), value);
        }
        assert_eq!(find_text(&conn, "rich_text", "secret").unwrap().unwrap().id, saved.id);
        update_text(&conn, saved.id, "text", "new secret", "new preview", 21).unwrap();
        assert!(encryption::is_modern_ciphertext(&raw("content")));
        assert_eq!(get(&conn, saved.id).unwrap().unwrap().content, "new secret");
    }

    #[cfg(not(feature = "portable"))]
    #[test]
    fn repair_encrypts_only_favorites_with_matching_sensitive_sources() {
        let conn = db();
        conn.execute_batch(
            "INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, created_at, preview)
             VALUES (1, 'text', 'secret', 'Notes', 10, 10, 'secret');
             INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'sensitive');",
        ).unwrap();
        let saved = insert(&conn, &NewFavorite { source_app: "Notes", ..text("secret") }, 20).unwrap();
        let manual = insert(&conn, &text("secret"), 20).unwrap();
        repair_sensitive_favorites(&conn).unwrap();
        repair_sensitive_favorites(&conn).unwrap();
        assert!(get(&conn, saved.id).unwrap().unwrap().is_encrypted);
        assert!(!get(&conn, manual.id).unwrap().unwrap().is_encrypted);
    }
}
