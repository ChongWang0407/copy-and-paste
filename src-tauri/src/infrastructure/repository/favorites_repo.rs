//! 常用: kept apart from `clipboard_history`, so no automatic cleanup reaches it.

use rusqlite::{params, Connection, OptionalExtension, Result, Row};
use serde::Serialize;

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
}

pub struct NewFavorite<'a> {
    pub content_type: &'a str,
    pub content: &'a str,
    pub html_content: Option<&'a str>,
    pub preview: &'a str,
    pub source_app: &'a str,
}

const COLUMNS: &str = "id, content_type, content, html_content, preview, source_app, sort_order, created_at";

fn from_row(row: &Row<'_>) -> Result<Favorite> {
    Ok(Favorite {
        id: row.get(0)?,
        content_type: row.get(1)?,
        content: row.get(2)?,
        html_content: row.get(3)?,
        preview: row.get(4)?,
        source_app: row.get(5)?,
        sort_order: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Favorite>> {
    let sql = format!("SELECT {COLUMNS} FROM favorites ORDER BY sort_order ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Favorite>> {
    let sql = format!("SELECT {COLUMNS} FROM favorites WHERE id = ?1");
    conn.query_row(&sql, params![id], from_row).optional()
}

/// Text already kept: adding it again keeps the one entry.
pub fn find_text(conn: &Connection, content_type: &str, content: &str) -> Result<Option<Favorite>> {
    let sql = format!("SELECT {COLUMNS} FROM favorites WHERE content_type = ?1 AND content = ?2");
    conn.query_row(&sql, params![content_type, content], from_row).optional()
}

/// New favorites go to the end of the list.
pub fn insert(conn: &Connection, new: &NewFavorite<'_>, now: i64) -> Result<Favorite> {
    let order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM favorites",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO favorites (content_type, content, html_content, preview, source_app, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![new.content_type, new.content, new.html_content, new.preview, new.source_app, order, now],
    )?;
    Ok(get(conn, conn.last_insert_rowid())?.expect("just inserted"))
}

/// Editing makes it plain text, like editing a history entry.
pub fn update_text(conn: &Connection, id: i64, content_type: &str, content: &str, preview: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE favorites SET content_type = ?1, content = ?2, html_content = NULL, preview = ?3, updated_at = ?4 WHERE id = ?5",
        params![content_type, content, preview, now, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM favorites WHERE id = ?1", params![id])?;
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
}
