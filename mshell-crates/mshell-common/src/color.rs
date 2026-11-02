//! Hex colour parsing shared by the frame painter, Settings colour inputs,
//! the monitor-layout preview and plugin panels (each used to carry its own
//! variant, two of which panicked on non-ASCII input).

/// Parse a hex colour into `(r, g, b, a)` bytes. Accepts `rgb`, `rgba`,
/// `rrggbb` and `rrggbbaa`, with an optional `#` or `0x` prefix (margo's
/// `0xrrggbbaa` form) and surrounding whitespace. `None` for anything else.
pub fn parse_hex_bytes(text: &str) -> Option<(u8, u8, u8, u8)> {
    let t = text.trim();
    let t = t
        .strip_prefix('#')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    // Guard before byte-slicing: a multi-byte char would otherwise panic on
    // a non-char-boundary slice.
    if !t.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    // Shorthand: each nibble doubles (`f` → `ff`).
    let nib = |i: usize| u8::from_str_radix(&t[i..i + 1], 16).ok().map(|v| v * 17);
    match t.len() {
        3 => Some((nib(0)?, nib(1)?, nib(2)?, 255)),
        4 => Some((nib(0)?, nib(1)?, nib(2)?, nib(3)?)),
        6 => Some((byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some((byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// [`parse_hex_bytes`] as unit floats (`0.0..=1.0`), the form cairo takes.
pub fn parse_hex_rgba(text: &str) -> Option<(f64, f64, f64, f64)> {
    let (r, g, b, a) = parse_hex_bytes(text)?;
    let unit = |v: u8| f64::from(v) / 255.0;
    Some((unit(r), unit(g), unit(b), unit(a)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rrggbb_with_and_without_prefix() {
        assert_eq!(parse_hex_bytes("#1a2b3c"), Some((0x1a, 0x2b, 0x3c, 0xff)));
        assert_eq!(parse_hex_bytes("1a2b3c"), Some((0x1a, 0x2b, 0x3c, 0xff)));
        assert_eq!(
            parse_hex_bytes("0x1a2b3cff"),
            Some((0x1a, 0x2b, 0x3c, 0xff))
        );
        assert_eq!(parse_hex_bytes("0X1A2B3C"), Some((0x1a, 0x2b, 0x3c, 0xff)));
    }

    #[test]
    fn parses_alpha_forms() {
        assert_eq!(parse_hex_bytes("#1a2b3c80"), Some((0x1a, 0x2b, 0x3c, 0x80)));
        assert_eq!(
            parse_hex_bytes("0x44475a14"),
            Some((0x44, 0x47, 0x5a, 0x14))
        );
    }

    #[test]
    fn parses_shorthand() {
        assert_eq!(parse_hex_bytes("#0f0"), Some((0x00, 0xff, 0x00, 0xff)));
        assert_eq!(parse_hex_bytes("#0f08"), Some((0x00, 0xff, 0x00, 0x88)));
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            parse_hex_bytes("  #1a2b3c  "),
            Some((0x1a, 0x2b, 0x3c, 0xff))
        );
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for bad in ["", "#", "nope", "#12345", "#zzzzzz", "#éèçñ", "#ä12", "0x"] {
            assert_eq!(parse_hex_bytes(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn unit_floats() {
        let (r, g, b, a) = parse_hex_rgba("#ff000080").unwrap();
        assert_eq!((r, g, b), (1.0, 0.0, 0.0));
        assert!((a - 128.0 / 255.0).abs() < 1e-9);
    }
}
