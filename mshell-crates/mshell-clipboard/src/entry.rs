use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Clone, Debug)]
pub struct ClipboardEntry {
    pub id: u64,
    pub timestamp: OffsetDateTime,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub preview: EntryPreview,
    pub content_hash: u64,
    /// Pinned (favourite) entries are exempt from `max_entries`
    /// eviction and every auto-clear policy, and persist to disk
    /// regardless of the persist mode.
    pub pinned: bool,
}

/// Coarse content category used by the clipboard menu's type tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipCategory {
    Text,
    Image,
    File,
}

impl ClipboardEntry {
    pub fn content_hash(data: &[u8]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }

    pub fn is_text(&self) -> bool {
        self.mime_type.starts_with("text/")
    }

    /// Coarse content category for the menu's type tabs.
    pub fn category(&self) -> ClipCategory {
        if self.mime_type.starts_with("text/") {
            ClipCategory::Text
        } else if self.mime_type.starts_with("image/") {
            ClipCategory::Image
        } else {
            ClipCategory::File
        }
    }

    /// Lower-cased haystack for substring search. Text entries match
    /// on their *full* content (not the 200-char preview), so a query
    /// finds text that scrolled off the visible snippet. Image /
    /// binary entries match on their MIME type, so e.g. `png` still
    /// surfaces a copied image.
    pub fn search_haystack(&self) -> String {
        if self.is_text() {
            String::from_utf8_lossy(&self.data).to_lowercase()
        } else {
            self.mime_type.to_lowercase()
        }
    }
}

#[derive(Clone, Debug)]
pub enum EntryPreview {
    Text(String),
    Image {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
    Binary {
        mime_type: String,
        size: usize,
    },
}

impl EntryPreview {
    pub const TEXT_PREVIEW_LEN: usize = 200;
    pub const THUMBNAIL_SIZE: u32 = 512;

    /// Build a preview from raw clipboard bytes. Shared by the live
    /// watcher and the on-disk loader so a restored entry renders
    /// identically to a freshly-copied one.
    pub fn build(mime_type: &str, data: &[u8]) -> EntryPreview {
        if mime_type.starts_with("text/") {
            let text = String::from_utf8_lossy(data);
            // Trim leading/trailing whitespace so a copied line with a
            // leading newline/indent doesn't render as a blank line
            // under the `#id`. Only the *preview* is trimmed — the
            // full `data` (what actually gets re-copied) is untouched.
            let truncated: String = text.trim().chars().take(Self::TEXT_PREVIEW_LEN).collect();
            EntryPreview::Text(truncated)
        } else if mime_type.starts_with("image/") {
            crate::thumbnail::generate_thumbnail(data).unwrap_or_else(|| EntryPreview::Binary {
                mime_type: mime_type.to_string(),
                size: data.len(),
            })
        } else {
            EntryPreview::Binary {
                mime_type: mime_type.to_string(),
                size: data.len(),
            }
        }
    }
}

/// On-disk form of a clipboard entry. The (potentially large) RGBA
/// `preview` is intentionally NOT serialized — it's regenerated
/// from `data` on load. Text payloads are stored inline; image /
/// binary payloads live in a content-addressed blob file and are
/// referenced here by `content_hash`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedEntry {
    pub id: u64,
    /// Unix seconds.
    pub timestamp: i64,
    pub mime_type: String,
    pub content_hash: u64,
    pub pinned: bool,
    /// `Some` for text payloads (stored inline). `None` means the
    /// payload is in `blobs/<content_hash>.bin`.
    pub inline_text: Option<String>,
}

impl PersistedEntry {
    pub fn from_entry(e: &ClipboardEntry) -> Self {
        let inline_text = if e.is_text() {
            Some(String::from_utf8_lossy(&e.data).into_owned())
        } else {
            None
        };
        Self {
            id: e.id,
            timestamp: e.timestamp.unix_timestamp(),
            mime_type: e.mime_type.clone(),
            content_hash: e.content_hash,
            pinned: e.pinned,
            inline_text,
        }
    }
}
