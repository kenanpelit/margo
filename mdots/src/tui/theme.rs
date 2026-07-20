//! TUI colour theme, optionally driven by margo's matugen palette.
//!
//! On first use the theme is loaded once (cached in a [`OnceLock`]) from
//! `~/.cache/margo/mshell-colors.toml` — the same Material You palette the
//! shell and `mvpn`/`mlogind` read — so the TUI's accent matches the rest of
//! the desktop. If that file is missing or unreadable, a fixed fallback
//! palette is used, so the TUI always has sensible colours.
//!
//! Health/status colours (ok/warn/danger) stay semantic green/yellow/red and
//! are intentionally *not* taken from the palette, so a doctor report or a
//! failing check always reads the same regardless of wallpaper.

use std::path::PathBuf;
use std::sync::OnceLock;

use ratatui::style::Color;

/// The resolved colour set used across the TUI.
pub struct Theme {
    /// Primary accent — borders, the titlebar, and the sidebar selection.
    pub accent: Color,
    /// Readable foreground for text drawn *on* an `accent` background.
    pub accent_fg: Color,
    /// Body text. Was a hardcoded `White` at every call site, which is
    /// simply wrong under a light palette.
    pub text: Color,
    /// De-emphasised text: hints, units, secondary columns.
    pub dim: Color,
}

impl Theme {
    /// The built-in palette used when no matugen file is available. These
    /// are the colours the TUI used before it read the palette at all.
    fn fallback() -> Self {
        Self {
            accent: Color::Blue,
            accent_fg: Color::Black,
            text: Color::White,
            dim: Color::DarkGray,
        }
    }
}

static THEME: OnceLock<Theme> = OnceLock::new();

/// The process-wide theme, loaded (and cached) on first access.
pub fn current() -> &'static Theme {
    THEME.get_or_init(load)
}

/// Convenience accessor for the accent colour — the one most call sites want.
pub fn accent() -> Color {
    current().accent
}

/// Convenience accessor for the on-accent foreground colour.
pub fn accent_fg() -> Color {
    current().accent_fg
}

/// Convenience accessor for the body-text colour.
pub fn text() -> Color {
    current().text
}

/// Convenience accessor for the de-emphasised text colour.
pub fn dim() -> Color {
    current().dim
}

fn load() -> Theme {
    palette_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|content| parse_palette(&content))
        .unwrap_or_else(Theme::fallback)
}

/// `~/.cache/margo/mshell-colors.toml`.
fn palette_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("margo").join("mshell-colors.toml"))
}

/// Pull the accent (`primary_color.base`) and its on-colour
/// (`primary_color.text`) out of the matugen palette. Returns `None` if the
/// accent isn't present, so the caller falls back to the built-in palette.
///
/// The file is small and machine-generated with a stable shape, so this does
/// a deliberately tiny line scan rather than pulling in a full TOML parser
/// (mdots has no `toml` dependency):
///
/// ```toml
/// [appearance]
/// primary_color = { base = "#5ec8c5", text = "#10212a" }
/// ```
///
/// Body text comes from `text_color`, and the de-emphasised tone from
/// `background_color.neutral` — the palette's own "raised surface" colour,
/// which is by construction visible against the background but quieter than
/// the text. Both fall back to the fixed defaults when absent, so a partial
/// palette degrades a shade at a time rather than all-or-nothing.
fn parse_palette(content: &str) -> Option<Theme> {
    let fallback = Theme::fallback();
    let mut accent = None;
    let mut accent_fg = None;
    let mut text = None;
    let mut dim = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("primary_color") {
            accent = hex_after(line, "base");
            accent_fg = hex_after(line, "text");
        } else if trimmed.starts_with("text_color") {
            text = hex_after(line, "text_color");
        } else if trimmed.starts_with("background_color") {
            dim = hex_after(line, "neutral");
        }
    }
    Some(Theme {
        accent: accent?,
        accent_fg: accent_fg.unwrap_or(fallback.accent_fg),
        text: text.unwrap_or(fallback.text),
        dim: dim.unwrap_or(fallback.dim),
    })
}

/// Find `key` in `line`, then parse the next `#rrggbb` after it into a colour.
fn hex_after(line: &str, key: &str) -> Option<Color> {
    let idx = line.find(key)?;
    let rest = &line[idx + key.len()..];
    let hash = rest.find('#')?;
    parse_hex6(&rest[hash + 1..])
}

/// Parse the first 6 chars of `s` as `rrggbb` into an RGB colour.
fn parse_hex6(s: &str) -> Option<Color> {
    let hex = s.get(..6)?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_accent_and_fg_from_matugen_palette() {
        let content = "\
# Auto-generated
[appearance]
background_color = { base = \"#1b1e2b\", weak = \"#252939\", neutral = \"#2f3346\", text = \"#d9def0\" }
primary_color    = { base = \"#5ec8c5\", text = \"#10212a\" }
danger_color     = \"#f7768e\"
text_color       = \"#d9def0\"
";
        let theme = parse_palette(content).expect("accent present");
        assert_eq!(theme.accent, Color::Rgb(0x5e, 0xc8, 0xc5));
        assert_eq!(theme.accent_fg, Color::Rgb(0x10, 0x21, 0x2a));
        assert_eq!(theme.text, Color::Rgb(0xd9, 0xde, 0xf0));
        assert_eq!(theme.dim, Color::Rgb(0x2f, 0x33, 0x46));
    }

    /// A palette carrying only the accent must still produce a usable theme
    /// — every other slot degrades to the built-in colour on its own.
    #[test]
    fn partial_palette_degrades_one_slot_at_a_time() {
        let theme = parse_palette("primary_color = { base = \"#abcdef\" }\n").expect("accent");
        let fallback = Theme::fallback();
        assert_eq!(theme.accent, Color::Rgb(0xab, 0xcd, 0xef));
        assert_eq!(theme.text, fallback.text);
        assert_eq!(theme.dim, fallback.dim);
    }

    /// `text_color` sits on its own line, but `background_color` also has a
    /// `text = ` key inside it — the scan must not pick that one up.
    #[test]
    fn body_text_comes_from_text_color_not_the_background_row() {
        let content = "\
background_color = { base = \"#1b1e2b\", neutral = \"#2f3346\", text = \"#ffffff\" }
primary_color = { base = \"#5ec8c5\" }
text_color = \"#111213\"
";
        let theme = parse_palette(content).expect("accent present");
        assert_eq!(theme.text, Color::Rgb(0x11, 0x12, 0x13));
    }

    #[test]
    fn missing_primary_color_yields_none() {
        let content = "[appearance]\ntext_color = \"#d9def0\"\n";
        assert!(parse_palette(content).is_none());
    }

    #[test]
    fn primary_without_text_falls_back_to_black_fg() {
        let content = "primary_color = { base = \"#abcdef\" }\n";
        let theme = parse_palette(content).expect("accent present");
        assert_eq!(theme.accent, Color::Rgb(0xab, 0xcd, 0xef));
        assert_eq!(theme.accent_fg, Color::Black);
    }

    #[test]
    fn rejects_malformed_hex() {
        assert_eq!(hex_after("base = \"#zzxxyy\"", "base"), None);
        assert_eq!(hex_after("base = \"#abc\"", "base"), None);
    }
}
