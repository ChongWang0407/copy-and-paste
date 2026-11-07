use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClipboardEntry {
    pub id: i64,
    pub content_type: String, // 'text', 'image', 'code', 'file', 'video'
    pub content: String,
    #[serde(default)]
    pub html_content: Option<String>,
    pub source_app: String,
    #[serde(default)]
    pub source_app_path: Option<String>,
    /// Legacy-compatible list timestamp. Mirrors `sort_at`.
    pub timestamp: i64,
    /// Immutable first-capture/import time.
    #[serde(default)]
    pub created_at: i64,
    /// Most recent explicit use (paste/open), if any.
    #[serde(default)]
    pub last_used_at: i64,
    /// Mutable ordering key used by "move to top after paste".
    #[serde(default)]
    pub sort_at: i64,
    pub preview: String,
    pub is_pinned: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Free-text remark on this entry (not a category tag).
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub use_count: i32,
    #[serde(default)]
    pub is_external: bool, // New field to track if content is a file path
    #[serde(default)]
    pub pinned_order: i64, // For manual sorting of pinned items
    #[serde(default = "default_true")]
    pub file_preview_exists: bool, // Transient field: does the file exist on disk?
    /// Text recognized in an image. Lists leave it out; search and 粘贴为文字 read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_text: Option<String>,
    /// Search results only: an image found by the text in it (its card gets the blue rim).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ocr_match: bool,
}

fn default_true() -> bool {
    true
}
