use crate::database::{
    calc_image_hash, calc_image_hash_from_bytes, calc_text_hash,
    has_sensitive_tag, is_text_type, save_image_bytes_to_file_with_extension, save_image_to_file,
};
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::encryption;
use crate::infrastructure::repository::settings_repo::SqliteSettingsRepository;
use rusqlite::{params, OptionalExtension};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use urlencoding::decode;

const RICH_IMAGE_FALLBACK_PREFIX: &str = "<!--CP_RICH_IMAGE:";
const RICH_IMAGE_FALLBACK_SUFFIX: &str = "-->";

fn decode_for_cleanup(value: &str) -> Result<std::borrow::Cow<'_, str>, String> {
    if encryption::is_encrypted_value(value) {
        return encryption::decrypt_stored_value(value).map(std::borrow::Cow::Owned);
    }
    Ok(std::borrow::Cow::Borrowed(value))
}

pub struct AttachmentNames(regex::RegexSet);

impl AttachmentNames {
    pub fn new(paths: &[PathBuf]) -> Option<Self> {
        let patterns = paths.iter().map(|path| {
            let name = path.file_name()?.to_str()?;
            let encoded = urlencoding::encode(name);
            let escaped = name.replace('&', "&amp;");
            Some(format!("{}|{}|{}", regex::escape(name), regex::escape(&encoded), regex::escape(&escaped)))
        }).collect::<Option<Vec<_>>>()?;
        regex::RegexSet::new(patterns).ok().map(Self)
    }

    pub fn references(&self, value: &str, found: &mut HashSet<usize>) {
        found.extend(self.0.matches(value));
        if value.contains('%') {
            if let Ok(decoded) = decode(value) {
                if let std::borrow::Cow::Owned(decoded) = decoded {
                    found.extend(self.0.matches(&decoded));
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClipboardSortMode {
    #[default]
    Activity,
    Created,
    LastUsed,
    Usage,
    Manual,
    /// Newest copy first, pinned or not. Not a list setting: the plain text
    /// paste hotkey uses it so a pin at the top of the list never wins.
    Recent,
}

impl ClipboardSortMode {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("created") => Self::Created,
            Some("last_used") => Self::LastUsed,
            Some("usage") => Self::Usage,
            Some("manual") => Self::Manual,
            Some(_) => Self::Activity,
            None => Self::Manual,
        }
    }

    fn sort_expression(self) -> &'static str {
        match self {
            Self::Activity | Self::Manual => "CASE WHEN is_pinned = 1 THEN pinned_order ELSE 0 END",
            Self::Created => "CASE WHEN is_pinned = 1 THEN pinned_order WHEN created_at > 0 THEN created_at ELSE sort_at END",
            Self::LastUsed => "CASE WHEN is_pinned = 1 THEN pinned_order ELSE last_used_at END",
            Self::Usage => "CASE WHEN is_pinned = 1 THEN pinned_order ELSE use_count END",
            Self::Recent => "0",
        }
    }

    fn order_clause(self) -> String {
        if self == Self::Recent {
            "sort_at DESC, id DESC".to_owned()
        } else {
            format!("is_pinned DESC, {} DESC, sort_at DESC, id DESC", self.sort_expression())
        }
    }

    fn key(self, entry: &ClipboardEntry) -> (bool, i64, i64, i64) {
        let pinned = self != Self::Recent && entry.is_pinned;
        let primary = if pinned {
            entry.pinned_order
        } else {
            match self {
                Self::Created if entry.created_at > 0 => entry.created_at,
                Self::Created => entry.timestamp,
                Self::LastUsed => entry.last_used_at,
                Self::Usage => i64::from(entry.use_count),
                _ => 0,
            }
        };
        (pinned, primary, entry.sort_at, entry.id)
    }

    pub fn compare(self, a: &ClipboardEntry, b: &ClipboardEntry) -> std::cmp::Ordering {
        self.key(b).cmp(&self.key(a))
    }
}

pub fn matches_type(entry: &ClipboardEntry, filter: Option<&str>) -> bool {
    filter.is_none_or(|kind| entry.content_type == kind || (kind == "text" && entry.content_type == "rich_text"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

const TYPE_FILTER_SQL: &str =
    "(:kind IS NULL OR content_type = :kind OR (:kind = 'text' AND content_type = 'rich_text'))";

const HISTORY_READABLE: &str =
    "(CASE WHEN content GLOB 'enc1:*' OR content GLOB 'dpapi:*' THEN cp_decodable(content) ELSE 1 END)
     AND (CASE WHEN preview GLOB 'enc1:*' OR preview GLOB 'dpapi:*' THEN cp_decodable(preview) ELSE 1 END)";

const DECODED_CONTENT: &str =
    "CASE WHEN content GLOB 'enc1:*' OR content GLOB 'plain:*' OR content GLOB 'dpapi:*'
          THEN cp_decode(content) ELSE content END";
const DECODED_PREVIEW: &str =
    "CASE WHEN preview GLOB 'enc1:*' OR preview GLOB 'plain:*' OR preview GLOB 'dpapi:*'
          THEN cp_decode(preview) ELSE preview END";

fn history_columns() -> String {
    format!(
        "id, content_type,
         CASE WHEN content_type IN ('text','code','url','rich_text') AND instr(content, char(0)) = 0
              THEN substr({DECODED_CONTENT}, 1, 2001) ELSE {DECODED_CONTENT} END,
         source_app, sort_at, {DECODED_PREVIEW}, is_pinned, tags, use_count,
         is_external, pinned_order, source_app_path, note, created_at, last_used_at"
    )
}

fn register_history_functions(conn: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    conn.create_scalar_function("cp_decode", 1, flags, |ctx| {
        Ok(encryption::decrypt_stored_value(ctx.get_raw(0).as_str()?).ok())
    })?;
    conn.create_scalar_function("cp_decodable", 1, flags, |ctx| {
        let value = ctx.get_raw(0).as_str()?;
        Ok(!encryption::is_encrypted_value(value) || encryption::decrypt_stored_value(value).is_ok())
    })
}

pub trait ClipboardRepository {
    fn save(
        &self,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
    ) -> Result<i64, String>;
    fn get_history(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String>;
    fn get_history_sorted(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
        sort_mode: ClipboardSortMode,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let _ = sort_mode;
        self.get_history(limit, offset, content_type)
    }
    fn get_history_since(
        &self,
        since_timestamp: i64,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String>;
    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
        note_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String>;
    fn delete(&self, id: i64, data_dir: Option<&std::path::Path>) -> Result<(), String>;
    fn clear(&self, data_dir: Option<&std::path::Path>) -> Result<(), String>;
    fn get_count(&self) -> Result<i64, String>;
    fn increment_use_count(&self, id: i64) -> Result<(), String>;
    fn touch_entry(&self, id: i64, timestamp: i64) -> Result<(), String>;
    fn toggle_pin(&self, id: i64, is_pinned: bool) -> Result<(), String>;
    fn update_pinned_order(&self, orders: Vec<(i64, i64)>) -> Result<(), String>;
    fn get_entry_by_id(&self, id: i64) -> Result<Option<ClipboardEntry>, String>;
    fn get_entry_by_content(
        &self,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String>;
    fn update_entry_content(&self, id: i64, content: &str, preview: &str) -> Result<(), String>;
    fn update_entry_note(&self, id: i64, note: &str) -> Result<(), String>;
    fn get_entry_content(&self, id: i64) -> Result<Option<String>, String>;
    fn get_entry_content_full(&self, id: i64) -> Result<Option<(String, String)>, String>;
    fn get_ocr_text(&self, id: i64) -> Result<Option<String>, String>;
    fn set_ocr_text(&self, id: i64, text: &str) -> Result<(), String>;
    fn get_entry_content_with_html(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String>;
}

pub struct SqliteClipboardRepository {
    conn: Arc<Mutex<Connection>>,
    history_functions_registered: std::sync::OnceLock<()>,
}



impl SqliteClipboardRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn, history_functions_registered: std::sync::OnceLock::new() }
    }

    pub fn encrypt_entry_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        let (content_raw, preview_raw, html_raw, content_type, content_hash): (String, String, Option<String>, String, i64) =
            conn.query_row(
                "SELECT content, preview, html_content, content_type, content_hash FROM clipboard_history WHERE id = ?",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2).ok(), row.get(3)?, row.get(4)?)),
            ).map_err(|e| e.to_string())?;

        let content_plain = self.maybe_decrypt_text(&content_raw)?;
        let preview_plain = self.maybe_decrypt_text(&preview_raw)?;
        let html_plain = html_raw.map(|h| self.maybe_decrypt_text(&h)).transpose()?;

        let content_enc = self.maybe_encrypt_text(&content_plain)?;
        let preview_enc = self.maybe_encrypt_text(&preview_plain)?;
        let html_enc = html_plain.as_ref().map(|h| self.maybe_encrypt_text(h)).transpose()?;
        let new_hash = if is_text_type(&content_type) {
            calc_text_hash(&content_plain) as i64
        } else {
            content_hash
        };

        conn.execute(
            "UPDATE clipboard_history SET content = ?, preview = ?, html_content = ?, content_hash = ? WHERE id = ?",
            params![content_enc, preview_enc, html_enc, new_hash, id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn decrypt_entry_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        let (content_raw, preview_raw, html_raw, content_type, content_hash): (String, String, Option<String>, String, i64) =
            conn.query_row(
                "SELECT content, preview, html_content, content_type, content_hash FROM clipboard_history WHERE id = ?",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2).ok(), row.get(3)?, row.get(4)?)),
            ).map_err(|e| e.to_string())?;

        let any_encrypted = encryption::is_encrypted_value(&content_raw)
            || encryption::is_encrypted_value(&preview_raw)
            || html_raw
                .as_ref()
                .map(|h| encryption::is_encrypted_value(h))
                .unwrap_or(false);
        if !any_encrypted {
            return Ok(());
        }

        let content_plain = self.maybe_decrypt_text(&content_raw)?;
        let preview_plain = self.maybe_decrypt_text(&preview_raw)?;
        let html_plain = html_raw.map(|h| self.maybe_decrypt_text(&h)).transpose()?;
        let new_hash = if is_text_type(&content_type) {
            calc_text_hash(&content_plain) as i64
        } else {
            content_hash
        };

        conn.execute(
            "UPDATE clipboard_history SET content = ?, preview = ?, html_content = ?, content_hash = ? WHERE id = ?",
            params![content_plain, preview_plain, html_plain, new_hash, id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn sync_entry_tags_with_conn(
        &self,
        conn: &Connection,
        entry_id: i64,
        tags: &[String],
    ) -> Result<(), String> {
        conn.execute(
            "DELETE FROM entry_tags WHERE entry_id = ?",
            params![entry_id],
        )
        .map_err(|e| e.to_string())?;
        for tag in tags {
            let clean = tag.trim();
            if clean.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                params![entry_id, clean],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn maybe_encrypt_text(&self, value: &str) -> Result<String, String> {
        encryption::encrypt_for_storage(value)
    }

    fn maybe_decrypt_text(&self, value: &str) -> Result<String, String> {
        encryption::decrypt_stored_value(value)
    }

    pub(crate) fn validate_entry_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        let (content, preview, html): (String, String, Option<String>) = conn.query_row(
            "SELECT content, preview, html_content FROM clipboard_history WHERE id = ?",
            [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|e| e.to_string())?;
        self.maybe_decrypt_text(&content)?;
        self.maybe_decrypt_text(&preview)?;
        html.as_deref().map(|v| self.maybe_decrypt_text(v)).transpose()?;
        Ok(())
    }

    fn extract_rich_image_fallback_payload(html: &str) -> Option<String> {
        if let Some(start) = html.rfind(RICH_IMAGE_FALLBACK_PREFIX) {
            let marker_start = start + RICH_IMAGE_FALLBACK_PREFIX.len();
            if let Some(end_rel) = html[marker_start..].find(RICH_IMAGE_FALLBACK_SUFFIX) {
                let marker_end = marker_start + end_rel;
                let payload = html[marker_start..marker_end].trim();
                if !payload.is_empty() {
                    return Some(payload.to_string());
                }
            }
        }
        None
    }

    fn fallback_payload_to_path(payload: &str) -> Option<PathBuf> {
        let value = payload.trim();
        if value.is_empty() || value.starts_with("data:image/") {
            return None;
        }

        let path_raw = if value.starts_with("file://") {
            value.trim_start_matches("file://")
        } else {
            value
        };

        let path_without_drive_prefix =
            if path_raw.starts_with('/') && path_raw.chars().nth(2) == Some(':') {
                &path_raw[1..]
            } else {
                path_raw
            };

        let decoded_path = decode(path_without_drive_prefix)
            .map(|p| p.into_owned())
            .unwrap_or_else(|_| path_without_drive_prefix.to_string());

        if decoded_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(decoded_path))
        }
    }

    fn collect_attachment_paths_for_cleanup(
        &self,
        content_raw: &str,
        html_raw: Option<&str>,
        is_external: bool,
        attachments_dir: &std::path::Path,
    ) -> Result<Vec<PathBuf>, String> {
        let mut paths = HashSet::new();

        let content = decode_for_cleanup(content_raw)?;
        if is_external {
            for line in content.lines() {
                let path = PathBuf::from(line.trim());
                if path.parent() == Some(attachments_dir) {
                    paths.insert(path);
                }
            }
        }

        if let Some(html_raw_value) = html_raw {
            let html = decode_for_cleanup(html_raw_value)?;
            if let Some(payload) = Self::extract_rich_image_fallback_payload(&html) {
                if let Some(path) = Self::fallback_payload_to_path(&payload) {
                    if path.parent() == Some(attachments_dir) {
                        paths.insert(path);
                    }
                }
            }
            if html.contains("attachments") {
                let attributes = Arc::new(Mutex::new(Vec::new()));
                let sources = attributes.clone();
                htmd::HtmlToMarkdown::builder()
                    .add_handler(vec!["img"], move |_: &dyn htmd::element_handler::Handlers, element: htmd::Element<'_>| {
                        for attr in element.attrs {
                            if matches!(attr.name.local.as_ref(), "src" | "data-src" | "data-original" | "data-actualsrc") {
                                sources.lock().unwrap().push(attr.value.to_string());
                            }
                        }
                        None
                    })
                    .build().convert(&html).map_err(|e| e.to_string())?;
                for source in attributes.lock().unwrap().iter() {
                    if let Some(path) = Self::fallback_payload_to_path(source) {
                        if path.parent() == Some(attachments_dir) {
                            paths.insert(path);
                        }
                    }
                }
            }
        }

        Ok(paths.into_iter().collect())
    }

    fn list_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardEntry> {
        let tags: String = row.get(7)?;
        Ok(ClipboardEntry {
            id: row.get(0)?,
            content_type: row.get(1)?,
            content: row.get(2)?,
            html_content: None,
            source_app: row.get(3)?,
            timestamp: row.get(4)?,
            sort_at: row.get(4)?,
            preview: row.get(5)?,
            is_pinned: row.get::<_, i32>(6)? == 1,
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            use_count: row.get(8)?,
            is_external: row.get::<_, i32>(9)? == 1,
            pinned_order: row.get(10)?,
            source_app_path: row.get(11)?,
            note: row.get(12)?,
            created_at: row.get(13)?,
            last_used_at: row.get(14)?,
            ocr_text: None,
            ocr_match: row.get(15)?,
            file_preview_exists: true,
        })
    }

    fn fetch_history(
        conn: &Connection,
        since: Option<i64>,
        limit: i32,
        offset: i32,
        kind: Option<&str>,
        mode: ClipboardSortMode,
    ) -> rusqlite::Result<Vec<ClipboardEntry>> {
        let order = mode.order_clause();
        let columns = history_columns();
        let sql = format!(
            "SELECT {columns}, 0 FROM clipboard_history
             WHERE {TYPE_FILTER_SQL} AND (:since IS NULL OR sort_at > :since) AND {HISTORY_READABLE}
             ORDER BY {order} LIMIT :limit OFFSET :offset"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            rusqlite::named_params! { ":kind": kind, ":since": since, ":limit": limit, ":offset": offset },
            Self::list_row,
        )?;
        rows.collect()
    }

    fn history_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if self.history_functions_registered.get().is_none() {
            register_history_functions(&conn).map_err(|e| e.to_string())?;
            self.history_functions_registered
                .set(())
                .expect("registration holds the connection lock");
        }
        Ok(conn)
    }

    fn load_history(
        &self,
        since: Option<i64>,
        limit: i32,
        offset: i32,
        kind: Option<&str>,
        mode: ClipboardSortMode,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let conn = self.history_connection()?;
        Self::fetch_history(&conn, since, limit.max(0), offset.max(0), kind, mode)
            .map_err(|e| e.to_string())
    }

    pub fn history_with_session(
        &self,
        mut session: Vec<&ClipboardEntry>,
        limit: i32,
        offset: i32,
        kind: Option<&str>,
        mode: ClipboardSortMode,
    ) -> Result<Vec<ClipboardEntry>, String> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let offset = offset.max(0);
        let conn = self.history_connection()?;
        let result = (|| -> rusqlite::Result<Vec<ClipboardEntry>> {
            session.retain(|entry| matches_type(entry, kind));
            if !session.is_empty() {
                // A legacy positive ID can also be present in memory. The
                // database wins before ranking so that pages contain unique IDs.
                let mut exists = conn.prepare_cached(
                    "SELECT EXISTS(SELECT 1 FROM clipboard_history WHERE id = ?)",
                )?;
                let mut unique = HashSet::new();
                let mut keep = Vec::with_capacity(session.len());
                for entry in session {
                    if entry.id == 0
                        || (unique.insert(entry.id)
                            && (entry.id < 0
                                || !exists.query_row([entry.id], |row| row.get::<_, bool>(0))?))
                    {
                        keep.push(entry);
                    }
                }
                session = keep;
                session.sort_by(|a, b| mode.compare(a, b));
            }

            // Rank the session rows without returning any earlier database
            // payloads. Both queries use the same readability filter.
            let mut left = 0;
            let mut right = session.len().min(offset as usize);
            if right > 0 {
                let key_filter = if mode == ClipboardSortMode::Recent {
                    "(sort_at, id) > (:sort_at, :id)".to_owned()
                } else {
                    format!(
                        "(is_pinned, {}, sort_at, id) > (:pinned, :primary, :sort_at, :id)",
                        mode.sort_expression()
                    )
                };
                let sql = format!(
                    "SELECT count(*) FROM clipboard_history
                     WHERE {TYPE_FILTER_SQL} AND {key_filter} AND {HISTORY_READABLE}"
                );
                let mut rank = conn.prepare_cached(&sql)?;
                while left < right {
                    let mid = left + (right - left) / 2;
                    let (pinned, primary, sort_at, id) = mode.key(session[mid]);
                    let db_before: usize = if mode == ClipboardSortMode::Recent {
                        rank.query_row(rusqlite::named_params! { ":kind": kind, ":sort_at": sort_at, ":id": id }, |row| row.get(0))?
                    } else {
                        rank.query_row(rusqlite::named_params! {
                            ":kind": kind, ":pinned": pinned, ":primary": primary, ":sort_at": sort_at, ":id": id,
                        }, |row| row.get(0))?
                    };
                    if mid + db_before < offset as usize {
                        left = mid + 1;
                    } else {
                        right = mid;
                    }
                }
            }
            let database =
                Self::fetch_history(&conn, None, limit, offset - left as i32, kind, mode)?;
            let mut database = database.into_iter().peekable();
            let mut session = session.into_iter().skip(left).peekable();
            let mut page = Vec::new();
            while page.len() < limit as usize {
                match (database.peek(), session.peek()) {
                    (Some(a), Some(b)) if mode.compare(a, b).is_le() => {
                        page.extend(database.next())
                    }
                    (_, Some(_)) => page.extend(session.next().cloned()),
                    (Some(_), None) => page.extend(database.next()),
                    (None, None) => break,
                }
            }
            Ok(page)
        })();
        result.map_err(|e| e.to_string())
    }

    pub fn search_filtered(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
        note_only: bool,
        kind: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let term = query.trim().to_lowercase();
        if limit <= 0 || (term.is_empty() && (tag_only || !note_only)) {
            return Ok(Vec::new());
        }
        let escaped = term
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let term_pattern = format!("%{escaped}%");
        let ocr_match =
            "content_type = 'image' AND COALESCE(ocr_text LIKE :pattern ESCAPE '\\', 0)";
        let condition = if tag_only {
            "EXISTS(SELECT 1 FROM entry_tags WHERE entry_id = ch.id AND tag COLLATE NOCASE = :term)"
                .to_owned()
        } else if note_only {
            "TRIM(note) != '' AND note LIKE :pattern ESCAPE '\\'".to_owned()
        } else {
            format!(
                "((content_type IN ('text','code','url','rich_text')
                    AND (({DECODED_CONTENT}) LIKE :pattern ESCAPE '\\'
                         OR ({DECODED_PREVIEW}) LIKE :pattern ESCAPE '\\'))
                  OR ({ocr_match}) OR source_app LIKE :pattern ESCAPE '\\'
                  OR note LIKE :pattern ESCAPE '\\'
                  OR EXISTS(SELECT 1 FROM entry_tags WHERE entry_id = ch.id AND tag LIKE :pattern ESCAPE '\\'))"
            )
        };
        let ocr_match = if tag_only || note_only {
            "0"
        } else {
            ocr_match
        };
        let columns = history_columns();
        let sql = format!(
            "SELECT {columns}, ({ocr_match}) FROM clipboard_history ch
             WHERE {TYPE_FILTER_SQL} AND {condition} AND {HISTORY_READABLE}
             ORDER BY sort_at DESC, id DESC LIMIT :limit"
        );
        let conn = self.history_connection()?;
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
        let rows = if tag_only {
            stmt.query_map(rusqlite::named_params! { ":kind": kind, ":term": term, ":limit": limit }, Self::list_row)
        } else {
            stmt.query_map(rusqlite::named_params! { ":kind": kind, ":pattern": term_pattern, ":limit": limit }, Self::list_row)
        }.map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())
    }

    pub fn save_with_conn(
        &self,
        conn: &Connection,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
        precomputed_content_hash: Option<i64>,
    ) -> Result<i64, String> {
        if entry.id > 0 { self.validate_entry_with_conn(conn, entry.id)?; }
        let mut entry = entry.clone();
        entry.content = self.maybe_decrypt_text(&entry.content)?;
        entry.preview = self.maybe_decrypt_text(&entry.preview)?;
        entry.html_content = entry.html_content.as_deref().map(|v| self.maybe_decrypt_text(v)).transpose()?;
        // Encrypt only when explicitly marked as sensitive
        let should_encrypt = has_sensitive_tag(&entry.tags);

        let mut final_content = entry.content.clone();
        let mut final_is_external = entry.is_external;

        // Externalize image if possible
        if entry.content_type == "image" && entry.content.starts_with("data:image/") {
            if let Some(dir) = data_dir {
                if let Some(path) = save_image_to_file(&entry.content, dir) {
                    final_content = path;
                    final_is_external = true;
                }
            }
        }

        // A session-only image (persistence off) sits under session/, which is
        // swept once no session entry shows it. Pinning or tagging moves the
        // entry here, so give the database its own copy in attachments/.
        if entry.content_type == "image" && !final_content.starts_with("data:") {
            if let Some(dir) = data_dir {
                let path = std::path::Path::new(&final_content);
                if path.starts_with(crate::app::data_directory::session_storage_root(dir))
                    || path.starts_with(dir.join("session"))
                {
                    let extension = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("png");
                    if let Some(copied) = std::fs::read(path).ok().and_then(|bytes| {
                        save_image_bytes_to_file_with_extension(&bytes, dir, extension)
                    }) {
                        final_content = copied;
                        final_is_external = true;
                    }
                }
            }
        }

        let calculated_hash = precomputed_content_hash.unwrap_or_else(|| {
            if entry.content_type == "image" {
                if entry.content.starts_with("data:") {
                    calc_image_hash(&entry.content).unwrap_or(0)
                } else if let Ok(bytes) = std::fs::read(&entry.content) {
                    calc_image_hash_from_bytes(&bytes).unwrap_or(0)
                } else {
                    0
                }
            } else {
                calc_text_hash(&final_content) as i64
            }
        });

        let (content, preview, content_hash, html_content) = if should_encrypt {
            let encrypted_content = self.maybe_encrypt_text(&final_content)?;
            let encrypted_preview = self.maybe_encrypt_text(&entry.preview)?;
            let encrypted_html = entry
                .html_content
                .as_ref()
                .map(|html| self.maybe_encrypt_text(html)).transpose()?;
            (
                encrypted_content,
                encrypted_preview,
                calculated_hash,
                encrypted_html,
            )
        } else {
            (
                final_content.clone(),
                entry.preview.clone(),
                calculated_hash,
                entry.html_content.clone(),
            )
        };

        let mut seen: HashSet<String> = HashSet::new();
        let mut cleaned_tags: Vec<String> = Vec::new();
        for tag in &entry.tags {
            let t = tag.trim();
            if t.is_empty() {
                continue;
            }
            let t_owned = t.to_string();
            if seen.insert(t_owned.clone()) {
                cleaned_tags.push(t_owned);
            }
        }
        let created_at = if entry.created_at > 0 {
            entry.created_at
        } else {
            entry.timestamp
        };
        let sort_at = if entry.sort_at > 0 {
            entry.sort_at
        } else {
            entry.timestamp
        };

        if entry.id > 0 {
            // Update existing entry (Move to top logic)
            conn.execute(
                "UPDATE clipboard_history SET 
                    content_type = ?1, 
                    content = ?2, 
                    html_content = ?3, 
                    source_app = ?4, 
                    timestamp = ?5, 
                    sort_at = ?5,
                    preview = ?6, 
                    content_hash = ?7, 
                    tags = ?8, 
                    is_external = ?9,
                    source_app_path = ?10,
                    use_count = use_count + 1
                 WHERE id = ?11",
                params![
                    entry.content_type,
                    content,
                    html_content,
                    entry.source_app,
                    sort_at,
                    preview,
                    content_hash,
                    serde_json::to_string(&cleaned_tags).unwrap_or_else(|_| "[]".to_string()),
                    if final_is_external { 1 } else { 0 },
                    entry.source_app_path.as_deref(),
                    entry.id
                ],
            )
            .map_err(|e| e.to_string())?;
            self.sync_entry_tags_with_conn(conn, entry.id, &cleaned_tags)?;
            Ok(entry.id)
        } else {
            // Insert new entry
            conn.execute(
                "INSERT INTO clipboard_history (content_type, content, html_content, source_app, timestamp, created_at, last_used_at, sort_at, preview, is_pinned, content_hash, tags, is_external, pinned_order, source_app_path, note, use_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    entry.content_type,
                    content,
                    html_content,
                    entry.source_app,
                    sort_at,
                    created_at,
                    entry.last_used_at,
                    sort_at,
                    preview,
                    if entry.is_pinned { 1 } else { 0 },
                    content_hash,
                    serde_json::to_string(&cleaned_tags).unwrap_or_else(|_| "[]".to_string()),
                    if final_is_external { 1 } else { 0 },
                    entry.pinned_order,
                    entry.source_app_path.as_deref(),
                    entry.note,
                    entry.use_count
                ],
            ).map_err(|e| e.to_string())?;

            let new_id = conn.last_insert_rowid();
            self.sync_entry_tags_with_conn(conn, new_id, &cleaned_tags)?;
            Ok(new_id)
        }
    }

    pub fn delete_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        data_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        self.delete_entries(conn, &[id], data_dir, now_ms())
    }

    pub fn delete_after_paste(
        &self,
        id: i64,
        data_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.delete_entries(&conn, &[id], data_dir, now_ms() + 120_000)
    }

    fn delete_entries(
        &self,
        conn: &Connection,
        ids: &[i64],
        data_dir: Option<&std::path::Path>,
        deadline: i64,
    ) -> Result<(), String> {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        let mut all_readable = true;
        let attachments = data_dir.map(|dir| dir.join("attachments"));
        for id in ids {
            if let Some(dir) = &attachments {
                let raw = tx
                    .query_row(
                        "SELECT content, html_content,
                            is_external = 1 OR content_type IN ('image', 'video', 'file', 'audio'),
                            preview FROM clipboard_history WHERE id = ?",
                        [id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, bool>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some((content, html, external, preview)) = raw {
                    match decode_for_cleanup(&preview).and_then(|_| {
                        self.collect_attachment_paths_for_cleanup(
                            &content,
                            html.as_deref(),
                            external,
                            dir,
                        )
                    }) {
                        Ok(paths) => files.extend(paths),
                        Err(_) => all_readable = false,
                    }
                }
            }
            tx.execute("DELETE FROM clipboard_history WHERE id = ?", [id])
                .map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM entry_tags WHERE entry_id = ?", [id])
                .map_err(|e| e.to_string())?;
        }
        if all_readable {
            Self::queue_attachment_deletions(&tx, &files, deadline)?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    fn queue_attachment_deletions(
        conn: &Connection,
        files: &[PathBuf],
        deadline: i64,
    ) -> Result<(), String> {
        let mut stmt = conn.prepare(
            "INSERT INTO pending_attachment_deletions (path, delete_after) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET delete_after = MAX(delete_after, excluded.delete_after)",
        ).map_err(|e| e.to_string())?;
        for path in files {
            stmt.execute(params![
                path.to_str().ok_or("invalid attachment path")?,
                deadline
            ])
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn queue_favorite_attachment_deletions(
        &self,
        conn: &Connection,
        data_dir: &std::path::Path,
        html: Option<&str>,
    ) -> Result<(), String> {
        if html.is_some() {
            let paths = self.collect_attachment_paths_for_cleanup(
                "",
                html,
                false,
                &data_dir.join("attachments"),
            )?;
            Self::queue_attachment_deletions(conn, &paths, now_ms())?;
        }
        Ok(())
    }

    pub fn sweep_attachment_deletions<'a>(
        &self,
        conn: &Connection,
        data_dir: &std::path::Path,
        mut session_files: Vec<PathBuf>,
        session: impl Iterator<Item = &'a ClipboardEntry>,
    ) -> Result<(), String> {
        let folder = data_dir.join("attachments");
        let mut stmt = conn.prepare(
            "SELECT path FROM pending_attachment_deletions WHERE delete_after <= ? ORDER BY delete_after",
        ).map_err(|e| e.to_string())?;
        let paths = stmt
            .query_map([now_ms()], |row| row.get::<_, String>(0).map(PathBuf::from))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let pending: HashSet<_> = if folder.symlink_metadata().is_ok_and(|meta| !meta.is_dir()) {
            HashSet::new()
        } else {
            paths
                .into_iter()
                .filter(|path| path.parent() == Some(&folder))
                .collect()
        };
        session_files.extend(pending.iter().cloned());
        let Some((unused, session_references)) =
            self.unreferenced_files(conn, &session_files, session)
        else {
            return Ok(());
        };
        for path in session_files {
            if unused.contains(&path) {
                crate::app::commands::thumbnail_cmd::remove_cached_thumbnails(&path);
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            // A remaining persistent owner will enqueue this file when it
            // leaves, while a session reference may disappear from memory.
            if pending.contains(&path) && !session_references.contains(&path) {
                conn.execute(
                    "DELETE FROM pending_attachment_deletions WHERE path = ?",
                    [path.to_string_lossy()],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn unreferenced_files<'a>(
        &self,
        conn: &Connection,
        candidates: &[PathBuf],
        session: impl Iterator<Item = &'a ClipboardEntry>,
    ) -> Option<(HashSet<PathBuf>, HashSet<PathBuf>)> {
        let mut referenced = HashSet::new();
        let mut referenced_by_session = HashSet::new();
        if !candidates.is_empty() {
            let patterns = AttachmentNames::new(candidates)?;
            for entry in session {
                patterns.references(&entry.content, &mut referenced_by_session);
                if let Some(html) = &entry.html_content {
                    patterns.references(html, &mut referenced_by_session);
                }
            }
            referenced.extend(&referenced_by_session);
            if referenced.len() == candidates.len() {
                return Some((HashSet::new(), candidates.iter().cloned().collect()));
            }
            let mut stmt = conn
                .prepare(
                    "SELECT content, html_content, preview, 1, 0 FROM clipboard_history
                 UNION ALL SELECT content, html_content, preview, is_encrypted,
                    content_type IN ('image', 'video', 'file') FROM favorites",
                )
                .ok()?;
            let mut rows = stmt.query([]).ok()?;
            while let Some(row) = rows.next().ok()? {
                let content: &str = row.get_ref(0).ok()?.as_str().ok()?;
                let html: Option<&str> = row.get_ref(1).ok()?.as_str_or_null().ok()?;
                let preview: &str = row.get_ref(2).ok()?.as_str().ok()?;
                let has_envelope = row.get::<_, bool>(3).ok()?;
                let (content, html) = if has_envelope {
                    decode_for_cleanup(preview).ok()?;
                    (
                        decode_for_cleanup(content).ok()?,
                        html.map(decode_for_cleanup).transpose().ok()?,
                    )
                } else {
                    (content.into(), html.map(Into::into))
                };
                if row.get::<_, bool>(4).ok()? {
                    // Copied favorite media uses relative paths under favorites/.
                    for path in content.lines().filter(|path| {
                        path.starts_with("file://") || std::path::Path::new(path).is_absolute()
                    }) {
                        patterns.references(path, &mut referenced);
                    }
                } else {
                    patterns.references(&content, &mut referenced);
                }
                if let Some(html) = html {
                    patterns.references(&html, &mut referenced);
                }
                if referenced.len() == candidates.len() {
                    break;
                }
            }
        }
        let unused = candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| !referenced.contains(i))
            .map(|(_, path)| path.clone())
            .collect();
        let session_paths = referenced_by_session
            .into_iter()
            .map(|index| candidates[index].clone())
            .collect();
        Some((unused, session_paths))
    }

    pub fn delete_metadata_with_conn(&self, conn: &Connection, id: i64) -> Result<(), String> {
        self.validate_entry_with_conn(conn, id)?;
        conn.execute("DELETE FROM clipboard_history WHERE id = ?", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM entry_tags WHERE entry_id = ?", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Return every textual candidate so a different rich-text format cannot
    /// hide a later match. Callers verify decrypted content and HTML.
    pub fn find_text_candidates_with_conn(&self, conn: &Connection, content: &str) -> Result<Vec<i64>, String> {
        let hash = calc_text_hash(content) as i64;
        let mut stmt = conn.prepare(
            "SELECT id FROM clipboard_history WHERE content_type IN ('text','code','url','rich_text') AND (content_hash = ?1 OR content = ?2) ORDER BY sort_at DESC, id DESC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![hash, content], |row| row.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(|e| e.to_string())
    }

    pub fn find_by_content_with_conn(
        &self,
        conn: &Connection,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String> {
        if content_type == Some("image") {
            if let Some(hash) = calc_image_hash(content) {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM clipboard_history \
                     WHERE (content_type = 'image' AND content_hash = ?) OR content = ?",
                    )
                    .map_err(|e| e.to_string())?;
                let mut rows = stmt
                    .query(params![hash, content])
                    .map_err(|e| e.to_string())?;
                if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    return Ok(Some(row.get(0).map_err(|e| e.to_string())?));
                }
                return Ok(None);
            }
        }

        let hash = calc_text_hash(content) as i64;

        if let Some(ct) = content_type {
            let mut stmt = conn.prepare(
                "SELECT id FROM clipboard_history \
                 WHERE (content_type = ? AND content_hash = ?) OR (content_type = ? AND content = ?)",
            ).map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![ct, hash, ct, content])
                .map_err(|e| e.to_string())?;
            if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                Ok(Some(row.get(0).map_err(|e| e.to_string())?))
            } else {
                Ok(None)
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id FROM clipboard_history \
                 WHERE ((content_type IN ('text', 'rich_text', 'code', 'url')) AND content_hash = ?) OR content = ?",
            ).map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![hash, content])
                .map_err(|e| e.to_string())?;
            if let Some(row) = rows.next().map_err(|e| e.to_string())? {
                Ok(Some(row.get(0).map_err(|e| e.to_string())?))
            } else {
                Ok(None)
            }
        }
    }

    pub fn enforce_limit_with_conn(
        &self,
        conn: &Connection,
        data_dir: Option<&std::path::Path>,
    ) -> Result<Vec<i64>, String> {
        // Check if storage limit is enabled
        if let Ok(Some(limit_enabled_str)) =
            SqliteSettingsRepository::get_raw(conn, "app.persistent_limit_enabled")
        {
            if limit_enabled_str == "false" {
                return Ok(Vec::new());
            }
        }

        // Get the storage limit
        if let Ok(Some(limit_str)) = SqliteSettingsRepository::get_raw(conn, "app.persistent_limit")
        {
            if let Ok(limit) = limit_str.parse::<i32>() {
                // Count non-pinned entries
                let count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM clipboard_history WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)",
                    [],
                    |row| row.get(0)
                ).map_err(|e| e.to_string())?;

                if count > limit {
                    // First, get the IDs that will be deleted
                    let to_delete = count - limit;
                    let deleted_ids: Vec<i64> = {
                        let mut stmt = conn
                            .prepare(
                                "SELECT id FROM clipboard_history 
                             WHERE is_pinned = 0 AND (tags = '[]' OR tags IS NULL)
                             ORDER BY sort_at ASC
                             LIMIT ?",
                            )
                            .map_err(|e| e.to_string())?;

                        let rows = stmt
                            .query_map([to_delete], |row| row.get(0))
                            .map_err(|e| e.to_string())?;
                        rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?
                    };
                    // Actually delete records (and files if needed)
                    self.delete_entries(conn, &deleted_ids, data_dir, now_ms())?;
                    return Ok(deleted_ids);
                }
            }
        }

        Ok(Vec::new())
    }
    pub fn toggle_pin_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        is_pinned: bool,
    ) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        if is_pinned {
            // Set pinned_order to max + 1 so it appears at top
            conn.execute(
                "UPDATE clipboard_history 
                 SET is_pinned = 1, 
                     pinned_order = (SELECT COALESCE(MAX(pinned_order), 0) + 1 FROM clipboard_history WHERE is_pinned = 1),
                     timestamp = ?2,
                     sort_at = ?2
                 WHERE id = ?1",
                params![id, now],
            ).map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE clipboard_history SET is_pinned = 0, pinned_order = 0, timestamp = ?2, sort_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn update_pinned_order_with_conn(
        &self,
        conn: &Connection,
        orders: Vec<(i64, i64)>,
    ) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        for (id, order) in orders {
            conn.execute(
                "UPDATE clipboard_history SET pinned_order = ?, timestamp = ?, sort_at = ? WHERE id = ?",
                params![order, now, now, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn get_entry_by_id_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<ClipboardEntry>, String> {
        let mut stmt = conn.prepare(
            "SELECT id, content_type, content, html_content, source_app, sort_at, preview, is_pinned, tags, use_count, is_external, pinned_order, source_app_path, note, created_at, last_used_at, sort_at
             FROM clipboard_history 
             WHERE id = ? 
             LIMIT 1",
        ).map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let tags_str: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();

            let content_raw: String = row.get(2).map_err(|e| e.to_string())?;
            let html_raw: Option<String> = row.get(3).map_err(|e| e.to_string())?;
            let preview_raw: String = row.get(6).map_err(|e| e.to_string())?;
            let content = self.maybe_decrypt_text(&content_raw)?;
            let preview = self.maybe_decrypt_text(&preview_raw)?;
            let html_content = html_raw.map(|v| self.maybe_decrypt_text(&v)).transpose()?;

            Ok(Some(ClipboardEntry {
                id: row.get(0).map_err(|e| e.to_string())?,
                content_type: row.get(1).map_err(|e| e.to_string())?,
                content,
                html_content,
                source_app: row.get(4).map_err(|e| e.to_string())?,
                timestamp: row.get(5).map_err(|e| e.to_string())?,
                created_at: row.get(14).unwrap_or_else(|_| row.get(5).unwrap_or(0)),
                last_used_at: row.get(15).unwrap_or(0),
                sort_at: row.get(16).unwrap_or_else(|_| row.get(5).unwrap_or(0)),
                preview,
                is_pinned: row.get::<_, i32>(7).map_err(|e| e.to_string())? == 1,
                tags,
                use_count: row.get(9).unwrap_or(0),
                is_external: row.get::<_, i32>(10).unwrap_or(0) == 1,
                pinned_order: row.get(11).unwrap_or(0),
                source_app_path: row.get(12).unwrap_or(None),
                note: row.get::<_, String>(13).unwrap_or_default(),
                ocr_text: None,
                ocr_match: false,
                file_preview_exists: true,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn update_entry_content_with_conn(
        &self,
        conn: &Connection,
        id: i64,
        content: &str,
        preview: &str,
    ) -> Result<(), String> {
        self.validate_entry_with_conn(conn, id)?;
        let content = self.maybe_decrypt_text(content)?;
        let preview = self.maybe_decrypt_text(preview)?;
        let content = content.as_str();
        let preview = preview.as_str();
        let (old_content_raw, content_type, tags_json, has_html) = conn
            .query_row(
                "SELECT content, content_type, tags, (html_content IS NOT NULL) FROM clipboard_history WHERE id = ?",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let old_content = self.maybe_decrypt_text(&old_content_raw)?;
        // Procceed if content changed, OR if content is same but we need to transition away from rich text/clear HTML
        if old_content == content && content_type != "rich_text" && !has_html {
            return Ok(());
        }

        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let should_encrypt = has_sensitive_tag(&tags);

        if is_text_type(&content_type) {
            let hash = calc_text_hash(content) as i64;
            let new_type = if content_type == "rich_text" {
                "text"
            } else {
                &content_type
            };
            if should_encrypt {
                let encrypted_content = self.maybe_encrypt_text(content)?;
                let encrypted_preview = self.maybe_encrypt_text(preview)?;
                conn.execute(
                    "UPDATE clipboard_history SET content = ?, preview = ?, content_hash = ?, html_content = NULL, content_type = ? WHERE id = ?",
                    params![encrypted_content, encrypted_preview, hash, new_type, id],
                ).map_err(|e| e.to_string())?;
            } else {
                conn.execute(
                    "UPDATE clipboard_history SET content = ?, preview = ?, content_hash = ?, html_content = NULL, content_type = ? WHERE id = ?",
                    params![content, preview, hash, new_type, id],
                ).map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        let image_hash = if content_type == "image" {
            calc_image_hash(content)
        } else {
            None
        };
        let encode = |value: &str| {
            if should_encrypt { self.maybe_encrypt_text(value) } else { Ok(value.into()) }
        };
        conn.execute(
            "UPDATE clipboard_history SET content = ?1, preview = ?2, html_content = NULL,
             content_hash = COALESCE(?3, content_hash),
             ocr_text = CASE WHEN content_type = 'image' THEN NULL ELSE ocr_text END WHERE id = ?4",
            params![encode(content)?, encode(preview)?, image_hash, id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_entry_content_if_current(
        &self, id: i64, created_at: i64, content: &str, preview: &str,
    ) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM clipboard_history WHERE id = ? AND created_at = ?)",
            params![id, created_at], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if current {
            self.update_entry_content_with_conn(&tx, id, content, preview)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(current)
    }

    pub fn set_ocr_text_if_current(&self, id: i64, created_at: i64, text: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE clipboard_history SET ocr_text = ?1 WHERE id = ?2 AND created_at = ?3 AND content_type = 'image'",
            params![text, id, created_at],
        ).map(|changed| changed > 0).map_err(|e| e.to_string())
    }

    pub fn get_entry_content_full_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<(String, String)>, String> {
        let mut stmt = conn
            .prepare("SELECT content, content_type FROM clipboard_history WHERE id = ?")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            let content_type: String = row.get(1).map_err(|e| e.to_string())?;
            Ok(Some((self.maybe_decrypt_text(&content)?, content_type)))
        } else {
            Ok(None)
        }
    }

    pub fn get_entry_content_with_html_with_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT content, content_type, html_content FROM clipboard_history WHERE id = ?",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            let content_type: String = row.get(1).map_err(|e| e.to_string())?;
            let html_raw: Option<String> = row.get(2).map_err(|e| e.to_string())?;
            let html_content = html_raw.map(|v| self.maybe_decrypt_text(&v)).transpose()?;
            Ok(Some((
                self.maybe_decrypt_text(&content)?,
                content_type,
                html_content,
            )))
        } else {
            Ok(None)
        }
    }
}

impl ClipboardRepository for SqliteClipboardRepository {
    fn save(
        &self,
        entry: &ClipboardEntry,
        data_dir: Option<&std::path::Path>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.save_with_conn(&conn, entry, data_dir, None)
    }

    fn get_history(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        self.load_history(None, limit, offset, content_type, ClipboardSortMode::Manual)
    }

    fn get_history_sorted(
        &self,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
        sort_mode: ClipboardSortMode,
    ) -> Result<Vec<ClipboardEntry>, String> {
        self.load_history(None, limit, offset, content_type, sort_mode)
    }

    fn get_history_since(
        &self,
        since_timestamp: i64,
        limit: i32,
        offset: i32,
        content_type: Option<&str>,
    ) -> Result<Vec<ClipboardEntry>, String> {
        self.load_history(
            Some(since_timestamp),
            limit,
            offset,
            content_type,
            ClipboardSortMode::Manual,
        )
    }

    fn search(
        &self,
        query: &str,
        limit: i32,
        tag_only: bool,
        note_only: bool,
    ) -> Result<Vec<ClipboardEntry>, String> {
        self.search_filtered(query, limit, tag_only, note_only, None)
    }

    fn delete(&self, id: i64, data_dir: Option<&std::path::Path>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.delete_with_conn(&conn, id, data_dir)
    }

    fn clear(&self, data_dir: Option<&std::path::Path>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        // Get IDs of unpinned items without tags.
        let ids = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM clipboard_history
             WHERE is_pinned = 0
               AND NOT EXISTS (SELECT 1 FROM entry_tags WHERE entry_id = clipboard_history.id)",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<i64>>>().map_err(|e| e.to_string())?
        };

        // Keep per-entry cleanup, but commit it as one transaction.
        // The previous implementation performed one durable SQLite commit per
        // entry and then VACUUMed synchronously, freezing the app on large lists.
        self.delete_entries(&conn, &ids, data_dir, now_ms())
    }

    fn get_count(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM clipboard_history")
            .map_err(|e| e.to_string())?;
        let count: i64 = stmt
            .query_row([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    fn increment_use_count(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE clipboard_history
             SET use_count = use_count + 1, last_used_at = ?1
             WHERE id = ?2",
            params![now_ms(), id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn touch_entry(&self, id: i64, timestamp: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let updated = conn.execute(
            "UPDATE clipboard_history SET timestamp = ?1, sort_at = ?1 WHERE id = ?2",
            params![timestamp, id],
        )
        .map_err(|e| e.to_string())?;
        if updated == 0 { Err("Entry not found".into()) } else { Ok(()) }
    }

    fn toggle_pin(&self, id: i64, is_pinned: bool) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.toggle_pin_with_conn(&conn, id, is_pinned)
    }

    fn update_pinned_order(&self, orders: Vec<(i64, i64)>) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        self.update_pinned_order_with_conn(&tx, orders)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_entry_by_id(&self, id: i64) -> Result<Option<ClipboardEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_by_id_with_conn(&conn, id)
    }

    fn get_entry_by_content(
        &self,
        content: &str,
        content_type: Option<&str>,
    ) -> Result<Option<i64>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.find_by_content_with_conn(&conn, content, content_type)
    }

    fn update_entry_content(&self, id: i64, content: &str, preview: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.update_entry_content_with_conn(&conn, id, content, preview)
    }

    fn update_entry_note(&self, id: i64, note: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.validate_entry_with_conn(&conn, id)?;
        let cleaned = note.trim();
        conn.execute(
            "UPDATE clipboard_history SET note = ?1 WHERE id = ?2",
            params![cleaned, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_entry_content(&self, id: i64) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT content FROM clipboard_history WHERE id = ?")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let content: String = row.get(0).map_err(|e| e.to_string())?;
            Ok(Some(self.maybe_decrypt_text(&content)?))
        } else {
            Ok(None)
        }
    }

    fn get_entry_content_full(&self, id: i64) -> Result<Option<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_content_full_with_conn(&conn, id)
    }

    fn get_ocr_text(&self, id: i64) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT ocr_text FROM clipboard_history WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| e.to_string())
    }

    fn set_ocr_text(&self, id: i64, text: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE clipboard_history SET ocr_text = ?1 WHERE id = ?2",
            params![text, id],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    fn get_entry_content_with_html(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, Option<String>)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_entry_content_with_html_with_conn(&conn, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ENCRYPT_PREFIX;
    use crate::infrastructure::repository::migrations::run_migrations;

    fn search_repository(fixture: &str) -> SqliteClipboardRepository {
        let conn = Connection::open_in_memory().expect("open database");
        run_migrations(&conn).expect("migrate database");
        conn.execute_batch(fixture).expect("insert search fixture");
        let repo = SqliteClipboardRepository::new(Arc::new(Mutex::new(conn)));
        {
            let conn = repo.conn.lock().unwrap();
            let tags = conn
                .prepare("SELECT id, tags FROM clipboard_history")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for (id, json) in tags {
                repo.sync_entry_tags_with_conn(
                    &conn,
                    id,
                    &serde_json::from_str::<Vec<String>>(&json).unwrap(),
                )
                .unwrap();
            }
        }
        repo
    }

    #[test]
    fn text_dedup_candidates_include_other_types_and_all_rich_variants() {
        let repo = search_repository("INSERT INTO clipboard_history (id, content_type, content, timestamp, sort_at, source_app, preview) VALUES (1,'rich_text','same',1,1,'test',''), (2,'rich_text','same',2,2,'test',''), (3,'code','same',3,3,'test',''), (4,'file','same',4,4,'test','');");
        let conn = repo.conn.lock().unwrap();
        conn.execute("UPDATE clipboard_history SET content_hash = ?1", params![calc_text_hash("same") as i64]).unwrap();
        assert_eq!(repo.find_text_candidates_with_conn(&conn, "same").unwrap(), vec![3,2,1]);
        assert_eq!(repo.find_text_candidates_with_conn(&conn, "different").unwrap(), Vec::<i64>::new());
    }

    fn search_ids(
        repo: &SqliteClipboardRepository,
        query: &str,
        limit: i32,
        tag_only: bool,
        note_only: bool,
    ) -> Vec<i64> {
        repo.search(query, limit, tag_only, note_only)
            .expect("search history")
            .into_iter()
            .map(|entry| entry.id)
            .collect()
    }

    const GENERAL_FIXTURE: &str = r#"
        INSERT INTO clipboard_history
            (id, content_type, content, source_app, timestamp, sort_at, preview, tags, note)
        VALUES
            (1, 'text', 'needle content', 'App', 100, 100, '', '["needle-a","needle-b","other"]', ''),
            (2, 'text', 'other', 'App', 90, 90, '', '["needle-a","needle-b"]', ''),
            (3, 'text', 'untagged needle', 'App', 80, 80, '', '[]', ''),
            (4, 'image', 'needle', 'App', 75, 75, 'needle', '[]', ''),
            (5, 'image', 'other', 'Needle App', 70, 70, '', '[]', ''),
            (6, 'text', 'other', 'App', 60, 60, '', '[]', 'needle note'),
            (7, 'rich_text', 'other', 'App', 50, 50, 'needle preview', '[]', '');
    "#;

    #[test]
    fn saving_a_session_image_copies_it_out_of_session_storage() {
        let data_dir = std::env::temp_dir().join(format!("cp-session-save-{}", std::process::id()));
        let session_dir = data_dir.join("session").join("attachments");
        std::fs::create_dir_all(&session_dir).unwrap();
        let png = b"\x89PNG\r\n\x1a\nnot really an image";
        let session_file = session_dir.join("img_1.png");
        std::fs::write(&session_file, png).unwrap();

        let repo = search_repository("");
        let entry: ClipboardEntry = serde_json::from_value(serde_json::json!({
            "id": -1,
            "content_type": "image",
            "content": session_file.to_string_lossy(),
            "source_app": "App",
            "timestamp": 1,
            "preview": "",
            "is_pinned": true,
            "is_external": true,
        }))
        .unwrap();
        let id = {
            let conn = repo.conn.lock().unwrap();
            repo.save_with_conn(&conn, &entry, Some(&data_dir), None).unwrap()
        };
        let stored: String = repo
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT content FROM clipboard_history WHERE id = ?", [id], |row| row.get(0))
            .unwrap();
        let stored = std::path::Path::new(&stored);
        assert!(stored.starts_with(data_dir.join("attachments")), "{stored:?}");
        assert_eq!(std::fs::read(stored).unwrap(), png);

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn an_edited_image_is_not_found_as_its_original() {
        let dir = std::env::temp_dir().join(format!("cp-image-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let edited = dir.join("edited.png");
        std::fs::write(&original, b"\x89PNG original").unwrap();
        std::fs::write(&edited, b"\x89PNG edited").unwrap();
        let (original, edited) = (original.to_string_lossy().to_string(), edited.to_string_lossy().to_string());

        let repo = search_repository("");
        let entry: ClipboardEntry = serde_json::from_value(serde_json::json!({
            "id": 0, "content_type": "image", "content": original, "source_app": "App",
            "timestamp": 1, "preview": "", "is_pinned": false, "is_external": true,
        }))
        .unwrap();
        let conn = repo.conn.lock().unwrap();
        let id = repo.save_with_conn(&conn, &entry, None, None).unwrap();

        repo.update_entry_content_with_conn(&conn, id, &edited, "").unwrap();

        assert_eq!(repo.find_by_content_with_conn(&conn, &original, Some("image")).unwrap(), None);
        assert_eq!(repo.find_by_content_with_conn(&conn, &edited, Some("image")).unwrap(), Some(id));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_preserves_matching_fields_order_and_limit_without_duplicate_tag_rows() {
        let repo = search_repository(GENERAL_FIXTURE);
        assert_eq!(
            search_ids(&repo, " NeEdLe ", 20, false, false),
            vec![1, 2, 3, 5, 6, 7]
        );
        assert_eq!(search_ids(&repo, "needle", 2, false, false), vec![1, 2]);
        assert_eq!(
            search_ids(&repo, "needle", 0, false, false),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn tag_search_keeps_case_insensitive_exact_matching_including_literal_wildcards() {
        let repo = search_repository(
            r#"
            INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, sort_at, preview, tags)
            VALUES
                (1, 'text', 'other', 'App', 100, 100, '', '["Work","WORK","work-extra"]'),
                (2, 'text', 'other', 'App', 90, 90, '', '["work"]'),
                (3, 'text', 'work', 'App', 80, 80, '', '["work-extra"]'),
                (4, 'text', 'other', 'App', 70, 70, '', '["work%"]');
        "#,
        );
        assert_eq!(search_ids(&repo, " WoRk ", 20, true, false), vec![1, 2]);
        assert_eq!(search_ids(&repo, "work", 1, true, false), vec![1]);
        assert_eq!(search_ids(&repo, "work%", 20, true, false), vec![4]);
        assert_eq!(search_ids(&repo, "wor", 20, true, false), Vec::<i64>::new());
    }

    #[test]
    fn note_search_and_empty_query_behavior_are_preserved() {
        let repo = search_repository(GENERAL_FIXTURE);
        assert_eq!(search_ids(&repo, "needle", 20, false, true), vec![6]);
        assert_eq!(search_ids(&repo, " ", 20, false, true), vec![6]);
        assert!(search_ids(&repo, " ", 20, false, false).is_empty());
        assert!(search_ids(&repo, " ", 20, true, false).is_empty());
    }

    #[cfg(not(feature = "portable"))]
    #[test]
    fn search_still_scans_sensitive_records_and_decodes_legacy_storage() {
        // Legacy storage exercises decryption without accessing the system keychain.
        let repo = search_repository(
            r#"
            INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, sort_at, preview, tags, note)
            VALUES
                (1, 'text', 'plain:hidden needle', 'App', 100, 100, 'plain:hidden preview', '["password","project"]', 'needle note'),
                (2, 'text', 'needle', 'App', 90, 90, '', '["project"]', '');
        "#,
        );
        let results = repo.search("needle", 20, false, false).unwrap();
        assert_eq!(
            results.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(results[0].content, "hidden needle");
        assert_eq!(results[0].preview, "hidden preview");
        assert_eq!(search_ids(&repo, "project", 20, true, false), vec![1, 2]);
        assert_eq!(search_ids(&repo, "password", 20, true, false), vec![1]);
        assert_eq!(
            search_ids(&repo, "proj", 20, true, false),
            Vec::<i64>::new()
        );
        assert_eq!(search_ids(&repo, "needle", 20, false, true), vec![1]);
        assert_eq!(search_ids(&repo, "", 20, false, true), vec![1]);
    }

    #[test]
    fn an_undecryptable_row_is_skipped_not_fatal() {
        // Valid envelope and base64, but not sealed with this device's key.
        let corrupt = format!("{ENCRYPT_PREFIX}{}", "A".repeat(48));
        assert!(encryption::decrypt_stored_value(&corrupt).is_err());
        let repo = search_repository(&format!(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, sort_at, preview, tags)
             VALUES
                 (1, 'text', 'one', 'App', 1, 1, 'one', '[]'),
                 (2, 'text', 'two', 'App', 2, 2, 'two', '[]'),
                 (3, 'text', 'three', 'App', 3, 3, 'three', '[]'),
                 (4, 'text', '{corrupt}', 'App', 4, 4, '{corrupt}', '[\"sensitive\"]'),
                 (5, 'text', '{corrupt}', 'App', 5, 5, '{corrupt}', '[]'),
                 (6, 'text', 'six', 'App', 6, 6, 'six', '[]');"
        ));
        let ids = |entries: Vec<ClipboardEntry>| entries.into_iter().map(|e| e.id).collect::<Vec<_>>();

        // The page is topped up past skipped rows, so paging still sees more.
        let first_page = repo
            .get_history_sorted(3, 0, None, ClipboardSortMode::Activity)
            .expect("one bad row must not fail the page");
        assert_eq!(ids(first_page), vec![6, 3, 2]);
        let all = repo
            .get_history_sorted(50, 0, None, ClipboardSortMode::Activity)
            .unwrap();
        assert_eq!(ids(all), vec![6, 3, 2, 1]);
        assert_eq!(ids(repo.get_history_since(2, 50, 0, None).unwrap()), vec![6, 3]);

        // Search (plain and encrypted-scan paths) skips them too.
        assert_eq!(search_ids(&repo, "app", 50, false, false), vec![6, 3, 2, 1]);

        // The unreadable rows are left as they were.
        let conn = repo.conn.lock().unwrap();
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM clipboard_history WHERE content = ?1", [&corrupt], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 2);
    }
    #[test]
    fn general_search_treats_percent_and_underscore_literally() {
        let repo = search_repository(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, sort_at, preview, tags)
             VALUES (1, 'text', '100% a_b', 'App', 1, 1, '', '[]'),
                    (2, 'text', '1000 axb', 'App', 2, 2, '', '[]');",
        );
        assert_eq!(search_ids(&repo, "100%", 10, false, false), vec![1]);
        assert_eq!(search_ids(&repo, "a_b", 10, false, false), vec![1]);
    }

    #[test]
    #[cfg(not(feature = "portable"))]
    fn search_ranks_encrypted_full_text_before_a_full_plaintext_page() {
        let repo = search_repository(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, sort_at, preview, tags)
             VALUES (1, 'text', 'needle', 'App', 1, 1, 'needle', '[]');",
        );
        let body = format!("{}needle", "字".repeat(2500));
        let cipher = crate::infrastructure::encryption::encrypt_for_storage(&body).unwrap();
        repo.conn.lock().unwrap().execute(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, sort_at, preview)
             VALUES (2, 'rich_text', ?1, 'App', 2, 2, 'early preview')",
            [cipher],
        ).unwrap();
        let hits = repo.search_filtered("needle", 1, false, false, Some("text")).unwrap();
        assert_eq!(hits[0].id, 2);
        assert_eq!(hits[0].content.chars().count(), 2001);
        assert_eq!(repo.get_entry_content(2).unwrap().as_deref(), Some(body.as_str()));
    }

    #[test]
    fn mixed_pages_skip_unreadable_rows_without_repeating_visible_rows() {
        let repo = search_repository(
            "WITH RECURSIVE entries(id) AS (VALUES(1) UNION ALL SELECT id + 1 FROM entries WHERE id < 1050)
             INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, sort_at, preview)
             SELECT id, 'text', 'body', 'App', id, id, 'preview' FROM entries;
             UPDATE clipboard_history SET content = 'dpapi:unreadable' WHERE id = 1049;",
        );
        let mut session = repo.get_entry_by_id(46).unwrap().unwrap();
        session.id = -1;
        session.content_type = "rich_text".into();
        let fetch = |offset, limit| repo.history_with_session(
            vec![&session], limit, offset, Some("text"), ClipboardSortMode::Manual,
        ).unwrap().into_iter().map(|entry| entry.id).collect::<Vec<_>>();
        let all = fetch(0, 1100);
        assert_eq!(all.len(), 1050);
        assert!(fetch(0, 1001).len() > 1000);
        assert_eq!(fetch(1000, 6), all[1000..1006]);
        assert!(all.contains(&-1));
        assert!(!all.contains(&1049));
    }

    #[test]
    fn default_history_queries_use_the_display_order_index() {
        let repo = search_repository("");
        let conn = repo.history_connection().unwrap();
        let columns = history_columns();
        for mode in [ClipboardSortMode::Manual, ClipboardSortMode::Activity] {
            let order = mode.order_clause();
            let mut stmt = conn.prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT {columns}, 0 FROM clipboard_history
                 WHERE {TYPE_FILTER_SQL} AND {HISTORY_READABLE}
                 ORDER BY {order} LIMIT 81 OFFSET 0",
            )).unwrap();
            let plan = stmt.query_map(rusqlite::named_params! { ":kind": "text" }, |row| row.get::<_, String>(3))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join(" ");
            assert!(plan.contains("idx_fork_history_display_order"), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
    }

    #[test]
    fn conditional_writebacks_reject_a_reused_id_and_image_edits_clear_ocr() {
        let repo = search_repository(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, created_at, preview, ocr_text)
             VALUES (1, 'image', '/tmp/before.png', 'Preview', 20, 20, 'before', 'old image text');",
        );
        assert!(!repo.update_entry_content_if_current(1, 10, "/tmp/other.png", "stale").unwrap());
        assert!(!repo.set_ocr_text_if_current(1, 10, "stale").unwrap());
        assert_eq!(repo.get_entry_content(1).unwrap().as_deref(), Some("/tmp/before.png"));
        assert_eq!(repo.get_ocr_text(1).unwrap().as_deref(), Some("old image text"));
        assert!(repo.update_entry_content_if_current(1, 20, "/tmp/after.png", "current").unwrap());
        assert_eq!(repo.get_ocr_text(1).unwrap(), None);
        assert!(repo.set_ocr_text_if_current(1, 20, "updated text").unwrap());
        assert_eq!(repo.get_ocr_text(1).unwrap().as_deref(), Some("updated text"));
    }

    #[test]
    fn full_content_reads_distinguish_missing_rows_and_unreadable_html() {
        let repo = search_repository(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, preview, html_content)
             VALUES (1, 'rich_text', 'complete body', 'Notes', 1, 'preview', 'dpapi:unreadable');",
        );
        assert!(repo.get_entry_content(404).unwrap().is_none());
        assert!(repo.get_entry_content_with_html(404).unwrap().is_none());
        assert_eq!(repo.get_entry_content(1).unwrap().as_deref(), Some("complete body"));
        assert!(repo.get_entry_content_with_html(1).is_err());
    }

    #[test]
    fn clear_history_keeps_files_when_a_deleted_row_cannot_be_read() {
        let root = std::env::temp_dir().join(format!("copypaste-clear-{}", uuid::Uuid::new_v4()));
        let attachments = root.join("attachments");
        std::fs::create_dir_all(&attachments).unwrap();
        let image = attachments.join("img_1.png");
        std::fs::write(&image, b"attachment").unwrap();
        let repo = search_repository(
            "INSERT INTO clipboard_history
                 (id, content_type, content, source_app, timestamp, preview)
             VALUES (1, 'text', 'dpapi:unreadable', 'Notes', 1, 'preview');",
        );
        repo.conn.lock().unwrap().execute(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, is_external)
             VALUES (2, 'image', ?1, 'Preview', 2, 'preview', 1)", [image.to_str().unwrap()],
        ).unwrap();
        repo.clear(Some(&root)).unwrap();
        let conn = repo.conn.lock().unwrap();
        repo.sweep_attachment_deletions(&conn, &root, Vec::new(), std::iter::empty()).unwrap();
        assert!(image.exists());
        assert_eq!(conn.query_row("SELECT count(*) FROM clipboard_history", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inline_attachment_cleanup_waits_for_the_favorite_reference() {
        let root = std::env::temp_dir().join(format!("copypaste-inline-{}", uuid::Uuid::new_v4()));
        let attachments = root.join("attachments");
        std::fs::create_dir_all(&attachments).unwrap();
        let shared = attachments.join("img_1.png");
        let orphan = attachments.join("img_2.png");
        for path in [&shared, &orphan] {
            std::fs::write(path, b"attachment").unwrap();
        }
        let repo = search_repository("");
        let conn = repo.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, html_content)
             VALUES (1, 'rich_text', 'body', 'Notes', 1, 'preview', ?1)",
            [format!("<img src=\"file://{}\"><img src=\"file://{}\">", shared.display(), orphan.display())],
        ).unwrap();
        conn.execute(
            "INSERT INTO favorites (content_type, content, html_content, sort_order, created_at, updated_at)
             VALUES ('rich_text', 'body', ?1, 1, 1, 1)",
            [format!("<img src=\"file://{}\">", shared.display())],
        ).unwrap();
        repo.delete_with_conn(&conn, 1, Some(&root)).unwrap();
        repo.sweep_attachment_deletions(&conn, &root, Vec::new(), std::iter::empty()).unwrap();
        assert!(shared.exists());
        assert!(!orphan.exists());
        repo.queue_favorite_attachment_deletions(&conn, &root, Some(&format!("<img src=\"file://{}\">", shared.display()))).unwrap();
        conn.execute("DELETE FROM favorites", []).unwrap();
        repo.sweep_attachment_deletions(&conn, &root, Vec::new(), std::iter::empty()).unwrap();
        assert!(!shared.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pasted_attachment_cleanup_survives_reopening_the_database() {
        let root = std::env::temp_dir().join(format!("copypaste-delayed-{}", uuid::Uuid::new_v4()));
        let image = root.join("attachments/img_1.png");
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, b"attachment").unwrap();
        let db_path = root.join("clipboard.db");
        let conn = Connection::open(&db_path).unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, is_external)
             VALUES (1, 'image', ?1, 'Preview', 1, 'preview', 1)",
            [image.to_str().unwrap()],
        ).unwrap();
        let repo = SqliteClipboardRepository::new(Arc::new(Mutex::new(conn)));
        repo.delete_after_paste(1, Some(&root)).unwrap();
        {
            let conn = repo.conn.lock().unwrap();
            repo.sweep_attachment_deletions(&conn, &root, Vec::new(), std::iter::empty()).unwrap();
            assert!(image.exists());
        }
        drop(repo);

        let conn = Connection::open(db_path).unwrap();
        assert_eq!(conn.execute("UPDATE pending_attachment_deletions SET delete_after = 0", []).unwrap(), 1);
        let repo = SqliteClipboardRepository::new(Arc::new(Mutex::new(conn)));
        let conn = repo.conn.lock().unwrap();
        repo.sweep_attachment_deletions(&conn, &root, Vec::new(), std::iter::empty()).unwrap();
        assert!(!image.exists());
        assert_eq!(conn.query_row("SELECT count(*) FROM pending_attachment_deletions", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
