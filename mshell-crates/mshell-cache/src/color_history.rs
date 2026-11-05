//! Persisted history of picked colours for the Color Picker menu.
//!
//! A small reactive singleton (same shape as [`crate::privacy_history`]):
//! the newest colour first, at most [`HISTORY_LIMIT`] entries, each with its
//! alpha. Every pick the shell starts (bar pill, Control Center tile, quick
//! action, the menu's eyedropper) lands here, and so does a colour copied
//! from the menu's edit fields. Clicking a swatch in the menu copies it
//! again. Saved as `~/.cache/mshell/color_history.json`.

use reactive_graph::prelude::{ReadUntracked, Update};
use reactive_stores::{ArcStore, Store};
use relm4::gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// How many colours the history keeps (the noctalia plugin's 12).
pub const HISTORY_LIMIT: usize = 12;

/// An sRGB colour with straight (non-premultiplied) alpha, one byte each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parse `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` (`#` optional,
    /// either case, surrounding spaces ignored).
    pub fn from_hex(text: &str) -> Option<Self> {
        let hex = text.trim();
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 | 4 => Some(Self {
                r: nibble(0)?,
                g: nibble(1)?,
                b: nibble(2)?,
                a: if hex.len() == 4 { nibble(3)? } else { 255 },
            }),
            6 | 8 => Some(Self {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: if hex.len() == 8 { byte(6)? } else { 255 },
            }),
            _ => None,
        }
    }

    /// `#RRGGBB`, or `#RRGGBBAA` when not opaque. Upper case, like mpicker.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct ColorHistoryState {
    /// Newest colour first, no repeats.
    pub colors: Vec<Rgba>,
}

/// On-disk shape: hex strings, so the file stays readable.
#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    colors: Vec<String>,
}

static COLOR_HISTORY: LazyLock<ArcStore<ColorHistoryState>> = LazyLock::new(|| {
    ArcStore::new(ColorHistoryState {
        colors: load_from(&history_path()),
    })
});

pub fn color_history_store() -> ArcStore<ColorHistoryState> {
    COLOR_HISTORY.clone()
}

/// `colors` with `color` moved (or added) to the front, capped at `limit`.
fn pushed(colors: &[Rgba], color: Rgba, limit: usize) -> Vec<Rgba> {
    std::iter::once(color)
        .chain(colors.iter().copied().filter(|c| *c != color))
        .take(limit)
        .collect()
}

/// Record a colour (newest first; an existing entry moves to the front) and
/// persist. Main thread: it updates a reactive store the menu watches.
pub fn record(color: Rgba) {
    let store = color_history_store();
    let next = pushed(&store.read_untracked().colors, color, HISTORY_LIMIT);
    if next == store.read_untracked().colors {
        return;
    }
    store.update(|s| s.colors = next);
    persist();
}

/// [`record`] a picker's hex output; ignored when it isn't a colour.
pub fn record_hex(hex: &str) {
    if let Some(color) = Rgba::from_hex(hex) {
        record(color);
    }
}

/// Forget every colour.
pub fn clear() {
    let store = color_history_store();
    if store.read_untracked().colors.is_empty() {
        return;
    }
    store.update(|s| s.colors.clear());
    persist();
}

fn history_path() -> PathBuf {
    glib::user_cache_dir()
        .join("mshell")
        .join("color_history.json")
}

/// The saved history; unreadable entries are skipped, a missing or broken
/// file is an empty history.
fn load_from(path: &Path) -> Vec<Rgba> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let file: HistoryFile = serde_json::from_str(&text).unwrap_or_default();
    let mut colors = Vec::new();
    for color in file.colors.iter().filter_map(|h| Rgba::from_hex(h)) {
        if !colors.contains(&color) && colors.len() < HISTORY_LIMIT {
            colors.push(color);
        }
    }
    colors
}

fn save_to(path: &Path, colors: &[Rgba]) -> std::io::Result<()> {
    let file = HistoryFile {
        colors: colors.iter().map(|c| c.to_hex()).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    margo_fs::atomic_write::atomic_write(path, &bytes)
}

fn persist() {
    let colors = color_history_store().read_untracked().colors.clone();
    if let Err(e) = save_to(&history_path(), &colors) {
        tracing::warn!("color history: save failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_every_length_and_formats_upper_case() {
        assert_eq!(Rgba::from_hex("#AB0CEF"), Some(Rgba::rgb(0xAB, 0x0C, 0xEF)));
        assert_eq!(
            Rgba::from_hex("ab0cef80"),
            Some(Rgba {
                a: 0x80,
                ..Rgba::rgb(0xAB, 0x0C, 0xEF)
            })
        );
        assert_eq!(Rgba::from_hex(" #f0a "), Some(Rgba::rgb(0xFF, 0x00, 0xAA)));
        assert_eq!(
            Rgba::from_hex("f0a8"),
            Some(Rgba {
                a: 0x88,
                ..Rgba::rgb(0xFF, 0x00, 0xAA)
            })
        );
        for bad in [
            "",
            "#",
            "#12345",
            "#1234567",
            "#GGGGGG",
            "#12 345",
            "#ＡＢＣ",
        ] {
            assert_eq!(Rgba::from_hex(bad), None, "{bad:?}");
        }
        assert_eq!(Rgba::rgb(1, 2, 3).to_hex(), "#010203");
        assert_eq!(
            Rgba {
                a: 0,
                ..Rgba::rgb(1, 2, 3)
            }
            .to_hex(),
            "#01020300"
        );
    }

    #[test]
    fn push_puts_newest_first_moves_repeats_and_caps() {
        let red = Rgba::rgb(255, 0, 0);
        let green = Rgba::rgb(0, 255, 0);
        let blue = Rgba::rgb(0, 0, 255);
        let half_red = Rgba { a: 128, ..red };
        let h = pushed(&[], red, 3);
        let h = pushed(&h, green, 3);
        assert_eq!(h, [green, red]);
        // The same colour again moves to the front instead of repeating.
        assert_eq!(pushed(&h, red, 3), [red, green]);
        // Alpha makes it a different colour.
        assert_eq!(pushed(&h, half_red, 3), [half_red, green, red]);
        // The oldest falls off at the cap.
        assert_eq!(
            pushed(&[blue, green, red], half_red, 3),
            [half_red, blue, green]
        );
    }

    #[test]
    fn history_file_round_trips_and_tolerates_junk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("color_history.json");
        assert!(load_from(&path).is_empty(), "missing file");
        let colors = vec![
            Rgba::rgb(0x28, 0x2A, 0x36),
            Rgba {
                a: 0x80,
                ..Rgba::rgb(0xBD, 0x93, 0xF9)
            },
        ];
        save_to(&path, &colors).expect("save");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("\"#282A36\""), "{text}");
        assert!(text.contains("\"#BD93F980\""), "{text}");
        assert_eq!(load_from(&path), colors);

        fs::write(
            &path,
            r##"{"colors":["#FF0000","nope","#ff0000","#00FF00"]}"##,
        )
        .expect("write");
        assert_eq!(
            load_from(&path),
            [Rgba::rgb(255, 0, 0), Rgba::rgb(0, 255, 0)],
            "bad entries skipped, repeats dropped"
        );
        fs::write(&path, "not json").expect("write");
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn a_long_saved_history_is_capped_on_load() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("h.json");
        let many: Vec<Rgba> = (0..30).map(|i| Rgba::rgb(i, 0, 0)).collect();
        save_to(&path, &many).expect("save");
        assert_eq!(load_from(&path).len(), HISTORY_LIMIT);
    }
}
