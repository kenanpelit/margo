//! Shared hex-colour input helper for Settings colour pickers.
//!
//! GTK4's `ColorDialogButton` opens its chooser as a **separate toplevel
//! window**. Settings runs as a layer-shell surface, and while its menu is
//! revealed margo keeps keyboard focus on that (Exclusive) layer — so the
//! separate dialog window never receives keyboard input, only the pointer
//! does. The colour wheel / sliders work by mouse, but the dialog's hex
//! field can't be typed into.
//!
//! To give every colour picker a keyboard path, we pair the dialog button
//! with an inline `gtk::Entry` that lives *inside* the layer surface (and
//! therefore does get keyboard focus). This module owns the hex parsing
//! the entry needs; the entry itself is wired per-page so its `#[watch]`
//! can bind to that page's colour field.

use relm4::gtk::gdk;

/// Parse a hex colour string into `(r, g, b, a)` bytes. Accepts `#rgb`,
/// `#rrggbb`, `#rrggbbaa` and the `0x…` / bare-hex forms; surrounding
/// whitespace is trimmed. Returns `None` for anything it can't read.
pub(crate) fn parse_hex_bytes(text: &str) -> Option<(u8, u8, u8, u8)> {
    let mut t = text.trim();
    t = t.strip_prefix('#').unwrap_or(t);
    t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    // Guard before byte-slicing: a multi-byte char would otherwise panic on
    // a non-char-boundary slice.
    if !t.is_ascii() {
        return None;
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match t.len() {
        // `#rgb` shorthand — expand each nibble (`f` → `ff`).
        3 => {
            let nib = |c: &str| u8::from_str_radix(c, 16).ok().map(|v| v * 17);
            Some((nib(&t[0..1])?, nib(&t[1..2])?, nib(&t[2..3])?, 255))
        }
        6 => Some((byte(&t[0..2])?, byte(&t[2..4])?, byte(&t[4..6])?, 255)),
        8 => Some((
            byte(&t[0..2])?,
            byte(&t[2..4])?,
            byte(&t[4..6])?,
            byte(&t[6..8])?,
        )),
        _ => None,
    }
}

/// Parse a hex colour string into a `gdk::RGBA`. `None` when unparseable.
pub(crate) fn parse_hex_rgba(text: &str) -> Option<gdk::RGBA> {
    let (r, g, b, a) = parse_hex_bytes(text)?;
    Some(gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_hex_bytes;

    #[test]
    fn parses_rrggbb_with_and_without_hash() {
        assert_eq!(parse_hex_bytes("#1a2b3c"), Some((0x1a, 0x2b, 0x3c, 0xff)));
        assert_eq!(parse_hex_bytes("1a2b3c"), Some((0x1a, 0x2b, 0x3c, 0xff)));
    }

    #[test]
    fn parses_rrggbbaa() {
        assert_eq!(parse_hex_bytes("#1a2b3c80"), Some((0x1a, 0x2b, 0x3c, 0x80)));
    }

    #[test]
    fn parses_0x_prefixed_form() {
        // margo / matugen emit `0xRRGGBBAA`; the entry must round-trip it.
        assert_eq!(
            parse_hex_bytes("0x1a2b3cff"),
            Some((0x1a, 0x2b, 0x3c, 0xff))
        );
    }

    #[test]
    fn parses_short_rgb() {
        assert_eq!(parse_hex_bytes("#0f0"), Some((0x00, 0xff, 0x00, 0xff)));
    }

    #[test]
    fn ignores_surrounding_whitespace() {
        assert_eq!(
            parse_hex_bytes("  #1a2b3c  "),
            Some((0x1a, 0x2b, 0x3c, 0xff))
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_hex_bytes(""), None);
        assert_eq!(parse_hex_bytes("nope"), None);
        assert_eq!(parse_hex_bytes("#12345"), None);
        assert_eq!(parse_hex_bytes("#zzzzzz"), None);
        // Non-ASCII must not panic on a non-char-boundary slice.
        assert_eq!(parse_hex_bytes("#éèçñ"), None);
    }
}
