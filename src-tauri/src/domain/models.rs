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

/// Identifies a capture even when SQLite reuses a deleted row's ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryIdentity {
    pub id: i64,
    pub created_at: i64,
}

impl EntryIdentity {
    pub fn from_entry(entry: &ClipboardEntry) -> Self {
        Self { id: entry.id, created_at: entry.created_at }
    }

    pub fn matches(self, entry: &ClipboardEntry) -> bool {
        self == Self::from_entry(entry)
    }
}

/// The content a background job read, independent of pinning and paste order.
#[derive(Clone)]
pub(crate) struct EntryVersion {
    pub id: i64,
    created_at: i64,
    content_type: String,
    pub content: String,
}

impl EntryVersion {
    pub fn identity(&self) -> EntryIdentity {
        EntryIdentity { id: self.id, created_at: self.created_at }
    }

    pub fn from_entry(entry: &ClipboardEntry) -> Self {
        Self {
            id: entry.id,
            created_at: entry.created_at,
            content_type: entry.content_type.clone(),
            content: entry.content.clone(),
        }
    }

    pub fn matches(&self, entry: &ClipboardEntry) -> bool {
        entry.id == self.id
            && entry.created_at == self.created_at
            && entry.content_type == self.content_type
            && entry.content == self.content
    }
}
