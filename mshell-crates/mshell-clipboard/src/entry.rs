use std::hash::{Hash, Hasher};
use std::sync::Arc;

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

/// Coarse content category used by the clipboard menu's type tabs +
/// per-row icon. Text copies are refined into URL / Colour / Code /
/// Email when they match (see [`detect_text_category`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipCategory {
    Text,
    Image,
    File,
    Url,
    Color,
    Code,
    Email,
}

/// Classify a *text* payload into a finer category. Order matters:
/// colour + url + email are specific shapes checked before the
/// catch-all code/text split.
pub fn detect_text_category(s: &str) -> ClipCategory {
    let t = s.trim();
    if t.is_empty() {
        return ClipCategory::Text;
    }
    if parse_color_hex(t).is_some() || is_rgb_func(t) {
        return ClipCategory::Color;
    }
    let lower = t.to_ascii_lowercase();
    let single_line = !t.contains('\n');
    if single_line
        && (lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("www."))
        && !t.contains(' ')
    {
        return ClipCategory::Url;
    }
    if single_line && looks_like_email(t) {
        return ClipCategory::Email;
    }
    let codey = (t.contains('\n') && (t.contains('{') || t.contains(';') || t.contains("()")))
        || t.starts_with("$ ")
        || t.starts_with("#!/");
    if codey {
        return ClipCategory::Code;
    }
    ClipCategory::Text
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` → normalised lower-case `#…`.
fn parse_color_hex(t: &str) -> Option<String> {
    let hex = t.strip_prefix('#')?;
    if !matches!(hex.len(), 3 | 6 | 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex.to_ascii_lowercase()))
}

fn is_rgb_func(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    (l.starts_with("rgb(") || l.starts_with("rgba(") || l.starts_with("hsl(")) && l.ends_with(')')
}

fn looks_like_email(t: &str) -> bool {
    let mut parts = t.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !t.contains(char::is_whitespace)
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

    /// Normalised colour hex (`#rrggbb`) when this entry is a colour
    /// copy, for the menu's swatch. `None` otherwise.
    pub fn color_hex(&self) -> Option<String> {
        if !self.is_text() {
            return None;
        }
        let text = String::from_utf8_lossy(&self.data);
        parse_color_hex(text.trim())
    }

    /// Coarse content category for the menu's type tabs + per-row icon.
    pub fn category(&self) -> ClipCategory {
        if self.mime_type.starts_with("text/") {
            detect_text_category(&String::from_utf8_lossy(&self.data))
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
        /// `Arc` so cloning a preview (e.g. into a menu row model) is a
        /// refcount bump, not a copy of the whole thumbnail buffer.
        rgba: Arc<[u8]>,
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

#[cfg(test)]
mod category_tests {
    use super::*;

    #[test]
    fn detects_hex_colour() {
        assert_eq!(detect_text_category("#ff8800"), ClipCategory::Color);
        assert_eq!(detect_text_category("#f80"), ClipCategory::Color);
        assert_eq!(
            detect_text_category("rgb(255, 136, 0)"),
            ClipCategory::Color
        );
    }

    #[test]
    fn detects_url() {
        assert_eq!(
            detect_text_category("https://example.com/x"),
            ClipCategory::Url
        );
        assert_eq!(detect_text_category("www.example.com"), ClipCategory::Url);
        // A sentence that mentions a url is NOT a url.
        assert_eq!(
            detect_text_category("see https://x.com please"),
            ClipCategory::Text
        );
    }

    #[test]
    fn detects_email() {
        assert_eq!(
            detect_text_category("kenan@example.com"),
            ClipCategory::Email
        );
        assert_eq!(detect_text_category("not@an@email"), ClipCategory::Text);
        assert_eq!(detect_text_category("plain text"), ClipCategory::Text);
    }

    #[test]
    fn detects_code() {
        assert_eq!(
            detect_text_category("fn main() {\n  let x = 1;\n}"),
            ClipCategory::Code
        );
        assert_eq!(detect_text_category("$ ls -la"), ClipCategory::Code);
    }

    #[test]
    fn plain_text_stays_text() {
        assert_eq!(
            detect_text_category("just a normal note"),
            ClipCategory::Text
        );
    }
}
