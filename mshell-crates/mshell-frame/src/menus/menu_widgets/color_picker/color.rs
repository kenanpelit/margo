//! Colour values for the Color Picker menu: the three text forms the menu
//! edits (HEX, RGB, HSL), each with an optional alpha, and the conversions
//! between them. Pure; the menu only wires entries to these functions.
//!
//! Accepted input (what the fields take on Enter):
//!
//! | Form | Text |
//! | --- | --- |
//! | HEX | `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA` (the `#` is optional) |
//! | RGB | `r, g, b` or `r, g, b, a`, optionally wrapped in `rgb(…)` / `rgba(…)` |
//! | HSL | `h, s%, l%` or `h, s%, l%, a`, optionally wrapped in `hsl(…)` / `hsla(…)` (`%` optional) |
//!
//! Alpha is `0`–`1` or a percentage (`50%`). Spaces or commas separate.

pub(crate) use mshell_cache::color_history::Rgba;

/// Alpha as the text fields show it: `0.5`, `0.25`, `1` (at most 2 places).
fn alpha_text(a: u8) -> String {
    let v = f64::from(a) / 255.0;
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `#RRGGBB`, or `#RRGGBBAA` when the colour is not opaque.
pub(crate) fn format_hex(c: Rgba) -> String {
    c.to_hex()
}

/// `r, g, b`, or `r, g, b, a` when not opaque.
pub(crate) fn format_rgb(c: Rgba) -> String {
    if c.a == 255 {
        format!("{}, {}, {}", c.r, c.g, c.b)
    } else {
        format!("{}, {}, {}, {}", c.r, c.g, c.b, alpha_text(c.a))
    }
}

/// `h, s%, l%`, or `h, s%, l%, a` when not opaque. Whole numbers.
pub(crate) fn format_hsl(c: Rgba) -> String {
    let (h, s, l) = rgb_to_hsl(c.r, c.g, c.b);
    let (h, s, l) = (h.round() as u32 % 360, s.round() as u32, l.round() as u32);
    if c.a == 255 {
        format!("{h}, {s}%, {l}%")
    } else {
        format!("{h}, {s}%, {l}%, {}", alpha_text(c.a))
    }
}

/// `(h 0..360, s 0..100, l 0..100)`.
pub(crate) fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l * 100.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s * 100.0, l * 100.0)
}

/// RGB bytes for `h` (degrees, any value — wrapped), `s` / `l` (0..100).
pub(crate) fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

/// Parse a HEX field.
pub(crate) fn parse_hex(text: &str) -> Option<Rgba> {
    Rgba::from_hex(text)
}

/// The numbers inside `name(…)` / `namea(…)` or a bare list, split on
/// commas, spaces and `/`.
fn components<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let inner = [format!("{name}a("), format!("{name}(")]
        .iter()
        .find(|p| lower.starts_with(p.as_str()))
        .and_then(|p| t[p.len()..].strip_suffix(')'))
        .unwrap_or(t);
    inner
        .split([',', ' ', '/'])
        .filter(|s| !s.is_empty())
        .collect()
}

/// `0`–`1`, or `N%`, as an alpha byte.
fn parse_alpha(s: &str) -> Option<u8> {
    let v = match s.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().ok()? / 100.0,
        None => s.parse::<f64>().ok()?,
    };
    (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8)
}

/// A number in `0..=max`, with an optional `%` (meaning a share of `max`
/// when `percent_of_max`, else just ignored).
fn parse_channel(s: &str, max: f64, percent_of_max: bool) -> Option<f64> {
    let (num, pct) = match s.strip_suffix('%') {
        Some(p) => (p.trim(), true),
        None => (s, false),
    };
    let v = num.parse::<f64>().ok()?;
    let v = if pct && percent_of_max {
        v / 100.0 * max
    } else {
        v
    };
    (v.is_finite() && (0.0..=max).contains(&v)).then_some(v)
}

/// Parse an RGB field.
pub(crate) fn parse_rgb(text: &str) -> Option<Rgba> {
    let parts = components(text, "rgb");
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let ch = |i: usize| parse_channel(parts[i], 255.0, true).map(|v| v.round() as u8);
    let a = match parts.get(3) {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    Some(Rgba {
        r: ch(0)?,
        g: ch(1)?,
        b: ch(2)?,
        a,
    })
}

/// Parse an HSL field.
pub(crate) fn parse_hsl(text: &str) -> Option<Rgba> {
    let parts = components(text, "hsl");
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let h = parts[0]
        .strip_suffix("deg")
        .unwrap_or(parts[0])
        .parse::<f64>()
        .ok()
        .filter(|h| h.is_finite())?;
    let s = parse_channel(parts[1], 100.0, false)?;
    let l = parse_channel(parts[2], 100.0, false)?;
    let a = match parts.get(3) {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(Rgba { r, g, b, a })
}

/// One of the menu's text forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorForm {
    Hex,
    Rgb,
    Hsl,
}

impl ColorForm {
    pub(crate) fn format(self, c: Rgba) -> String {
        match self {
            Self::Hex => format_hex(c),
            Self::Rgb => format_rgb(c),
            Self::Hsl => format_hsl(c),
        }
    }

    pub(crate) fn parse(self, text: &str) -> Option<Rgba> {
        match self {
            Self::Hex => parse_hex(text),
            Self::Rgb => parse_rgb(text),
            Self::Hsl => parse_hsl(text),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hex => "HEX",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }

    #[test]
    fn hex_forms_parse_and_format() {
        assert_eq!(parse_hex("#AB0CEF"), Some(c(0xAB, 0x0C, 0xEF, 255)));
        assert_eq!(parse_hex("ab0cef80"), Some(c(0xAB, 0x0C, 0xEF, 0x80)));
        assert_eq!(parse_hex(" #f0a "), Some(c(0xFF, 0x00, 0xAA, 255)));
        assert_eq!(parse_hex("#f0a8"), Some(c(0xFF, 0x00, 0xAA, 0x88)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
        assert_eq!(parse_hex(""), None);
        assert_eq!(format_hex(c(1, 2, 3, 255)), "#010203");
        assert_eq!(format_hex(c(1, 2, 3, 0x80)), "#01020380");
    }

    #[test]
    fn rgb_forms_parse_and_format() {
        assert_eq!(parse_rgb("12, 34, 56"), Some(c(12, 34, 56, 255)));
        assert_eq!(parse_rgb("rgb(12 34 56)"), Some(c(12, 34, 56, 255)));
        assert_eq!(parse_rgb("RGBA(12,34,56,0.5)"), Some(c(12, 34, 56, 128)));
        assert_eq!(parse_rgb("12, 34, 56, 50%"), Some(c(12, 34, 56, 128)));
        assert_eq!(parse_rgb("100%, 0, 0"), Some(c(255, 0, 0, 255)));
        assert_eq!(parse_rgb("256, 0, 0"), None);
        assert_eq!(parse_rgb("1, 2"), None);
        assert_eq!(parse_rgb("1, 2, 3, 2"), None, "alpha above 1");
        assert_eq!(parse_rgb("a, b, c"), None);
        assert_eq!(format_rgb(c(12, 34, 56, 255)), "12, 34, 56");
        assert_eq!(format_rgb(c(12, 34, 56, 128)), "12, 34, 56, 0.5");
        assert_eq!(format_rgb(c(12, 34, 56, 0)), "12, 34, 56, 0");
    }

    #[test]
    fn hsl_of_reference_colours() {
        for (rgb, hsl) in [
            ((255, 0, 0), "0, 100%, 50%"),
            ((0, 255, 0), "120, 100%, 50%"),
            ((0, 0, 255), "240, 100%, 50%"),
            ((255, 255, 255), "0, 0%, 100%"),
            ((0, 0, 0), "0, 0%, 0%"),
            ((128, 128, 128), "0, 0%, 50%"),
            ((255, 0, 2), "0, 100%, 50%"),
        ] {
            assert_eq!(format_hsl(c(rgb.0, rgb.1, rgb.2, 255)), hsl, "{rgb:?}");
        }
        assert_eq!(format_hsl(c(255, 0, 0, 64)), "0, 100%, 50%, 0.25");
    }

    #[test]
    fn hsl_forms_parse() {
        assert_eq!(parse_hsl("0, 100%, 50%"), Some(c(255, 0, 0, 255)));
        assert_eq!(parse_hsl("hsl(120 100 50)"), Some(c(0, 255, 0, 255)));
        assert_eq!(
            parse_hsl("hsla(240deg, 100%, 50%, 0.5)"),
            Some(c(0, 0, 255, 128))
        );
        assert_eq!(parse_hsl("-120, 100, 50"), Some(c(0, 0, 255, 255)), "wraps");
        assert_eq!(parse_hsl("0, 101%, 50%"), None);
        assert_eq!(parse_hsl("0, 100%"), None);
    }

    #[test]
    fn every_form_round_trips_through_its_text() {
        for col in [
            c(0, 0, 0, 255),
            c(255, 255, 255, 255),
            c(0x28, 0x2A, 0x36, 255),
            c(0xBD, 0x93, 0xF9, 0xCC),
            c(10, 200, 30, 0),
        ] {
            assert_eq!(ColorForm::Hex.parse(&ColorForm::Hex.format(col)), Some(col));
            assert_eq!(ColorForm::Rgb.parse(&ColorForm::Rgb.format(col)), Some(col));
            // HSL text is whole numbers, so allow a 1-step rounding error.
            let back = ColorForm::Hsl
                .parse(&ColorForm::Hsl.format(col))
                .expect("hsl");
            for (x, y) in [(back.r, col.r), (back.g, col.g), (back.b, col.b)] {
                assert!(x.abs_diff(y) <= 3, "{col:?} -> {back:?}");
            }
            assert_eq!(back.a, col.a);
        }
    }

    #[test]
    fn hsl_to_rgb_covers_every_sector() {
        for (h, rgb) in [
            (0.0, (255, 0, 0)),
            (60.0, (255, 255, 0)),
            (120.0, (0, 255, 0)),
            (180.0, (0, 255, 255)),
            (240.0, (0, 0, 255)),
            (300.0, (255, 0, 255)),
            (360.0, (255, 0, 0)),
        ] {
            assert_eq!(hsl_to_rgb(h, 100.0, 50.0), rgb, "h={h}");
        }
    }
}
