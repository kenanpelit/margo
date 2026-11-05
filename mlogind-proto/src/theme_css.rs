//! The greeter's synced colour palette, `/var/lib/mgreet/theme.css`.
//!
//! `mlogind sync-theme` publishes it from the last user's
//! `~/.cache/mshell/last_theme.css`, and `mgreet` layers it over its own
//! stylesheet for every user who comes to the login screen. The source is a
//! file that user can write, so it is never published byte for byte: a user
//! could hide the real labels (`opacity: 0`) or paint a picture of fake
//! instructions (`background-image: url(file://…)`) on the next person's login
//! screen.
//!
//! [`sanitize`] keeps only what mshell-matugen's `css_mapping::to_css` writes:
//! one `:root { … }` block of `--<token>: <colour>;` declarations, the token
//! from [`TOKENS`], the colour a hex or `rgb()`/`rgba()` literal. It returns
//! its own serialisation, never the input's bytes: every colour as opaque
//! `#rrggbb` (alpha is dropped, so `#0000` cannot make text invisible), and
//! nothing at all when the result would leave the login text unreadable
//! (below [`MIN_CONTRAST`] on a [`READABLE_PAIRS`] pair), so the greeter
//! keeps its default palette. Both ends use it: mlogind
//! before it publishes, mgreet when it loads (which also neutralises a file an
//! older mlogind copied verbatim).

/// Exactly the custom properties `mshell-matugen::css_mapping::to_css` emits.
pub const TOKENS: &[&str] = &[
    "surface",
    "on-surface",
    "surface-variant",
    "on-surface-variant",
    "surface-container-highest",
    "surface-container-high",
    "surface-container",
    "surface-container-low",
    "surface-container-lowest",
    "inverse-surface",
    "inverse-on-surface",
    "surface-tint",
    "surface-tint-color",
    "primary",
    "on-primary",
    "primary-container",
    "on-primary-container",
    "secondary",
    "on-secondary",
    "secondary-container",
    "on-secondary-container",
    "tertiary",
    "on-tertiary",
    "tertiary-container",
    "on-tertiary-container",
    "error",
    "on-error",
    "error-container",
    "on-error-container",
    "outline",
    "outline-variant",
];

/// The colour tokens of the greeter's baked palette (`mgreet/src/style.scss`
/// `:root`), which every token the overlay leaves out keeps. mgreet tests that
/// the two agree.
pub const GREETER_DEFAULTS: &[(&str, &str)] = &[
    ("surface", "#282a36"),
    ("surface-container-lowest", "#1e1f29"),
    ("surface-container", "#313442"),
    ("surface-container-high", "#3a3d4d"),
    ("surface-container-highest", "#444759"),
    ("on-surface", "#f8f8f2"),
    ("on-surface-variant", "#bcc0cc"),
    ("primary", "#bd93f9"),
    ("on-primary", "#1e1f29"),
    ("primary-container", "#44415a"),
    ("on-primary-container", "#e9defa"),
    ("outline", "#6272a4"),
    ("error", "#ff5555"),
];

/// `(text, background)` pairs the greeter draws for what a user must read to
/// log in: the card, its fields and their placeholders, the login button, the
/// avatar monogram, the clock over the backdrop, the error line.
const READABLE_PAIRS: &[(&str, &str)] = &[
    ("on-surface", "surface"),
    ("on-surface", "surface-container-lowest"),
    ("on-surface", "surface-container"),
    ("on-surface", "surface-container-high"),
    ("on-surface", "surface-container-highest"),
    ("on-surface-variant", "surface"),
    ("on-surface-variant", "surface-container-lowest"),
    ("on-surface-variant", "surface-container"),
    ("on-surface-variant", "surface-container-high"),
    ("on-primary", "primary"),
    ("on-primary-container", "primary-container"),
    ("error", "surface"),
];

/// The least WCAG contrast ratio a [`READABLE_PAIRS`] pair may have. Not an
/// accessibility bar (that is 4.5): a floor under which text stops being
/// legible at all, low enough that every shipped theme and an M3 palette keep
/// their look.
const MIN_CONTRAST: f64 = 2.0;

/// An opaque sRGB colour from `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, or
/// `rgb(…)`/`rgba(…)` with numbers or percentages separated by commas or
/// spaces. Any alpha is checked and dropped: a palette colour is always
/// opaque, so `#0000` cannot turn the next user's labels invisible.
fn parse_colour(value: &str) -> Option<[u8; 3]> {
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 | 4 => Some([nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17]),
            6 | 8 => Some([byte(0)?, byte(2)?, byte(4)?]),
            _ => None,
        };
    }
    let args = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))?;
    let parts: Vec<&str> = args.split([',', ' ']).filter(|p| !p.is_empty()).collect();
    if !matches!(parts.len(), 3 | 4) {
        return None;
    }
    let channel = |p: &str| -> Option<u8> {
        let (num, percent) = match p.strip_suffix('%') {
            Some(n) => (n, true),
            None => (p, false),
        };
        if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return None;
        }
        let v: f64 = num.parse().ok()?;
        let v = if percent { v * 255.0 / 100.0 } else { v };
        Some(v.round().clamp(0.0, 255.0) as u8)
    };
    // The alpha must still be a number; only its value is ignored.
    if let Some(alpha) = parts.get(3) {
        channel(alpha)?;
    }
    Some([channel(parts[0])?, channel(parts[1])?, channel(parts[2])?])
}

/// WCAG 2 relative luminance.
fn luminance([r, g, b]: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG 2 contrast ratio: 1.0 (same colour) to 21.0 (black on white).
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Whether every [`READABLE_PAIRS`] pair stays legible once `overlay` is laid
/// over [`GREETER_DEFAULTS`].
fn readable(overlay: &[(&str, [u8; 3])]) -> bool {
    let colour = |token: &str| {
        overlay
            .iter()
            .rev() // CSS resolves duplicate declarations to the last one.
            .find(|(t, _)| *t == token)
            .map(|(_, c)| *c)
            .or_else(|| {
                GREETER_DEFAULTS
                    .iter()
                    .find(|(t, _)| *t == token)
                    .and_then(|(_, v)| parse_colour(v))
            })
    };
    READABLE_PAIRS
        .iter()
        .all(|(text, bg)| match (colour(text), colour(bg)) {
            (Some(text), Some(bg)) => contrast(text, bg) >= MIN_CONTRAST,
            _ => false,
        })
}

/// Reduce `css` to the palette declarations it carries, re-serialised as one
/// `:root` block. `None` when the input is anything but a single `:root { … }`
/// block (another selector, an at-rule, a comment, trailing text), when no
/// declaration survives, or when the palette would hide the login text. A declaration with an unknown token or a value that is
/// not a plain colour is dropped; the rest are kept.
pub fn sanitize(css: &str) -> Option<String> {
    let body = css
        .trim()
        .strip_prefix(":root")?
        .trim_start()
        .strip_prefix('{')?
        .strip_suffix('}')?;
    // One block: no nesting, no second rule, no comment that could hide one.
    if body.contains(['{', '}', '@', '\\']) || body.contains("/*") {
        return None;
    }

    let mut kept: Vec<(&str, [u8; 3])> = Vec::new();
    for decl in body.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((name, value)) = decl.split_once(':') else {
            continue;
        };
        let (Some(token), value) = (name.trim().strip_prefix("--"), value.trim()) else {
            continue;
        };
        if let (Some(token), Some(colour)) = (
            TOKENS.iter().copied().find(|t| *t == token),
            parse_colour(value),
        ) {
            kept.push((token, colour));
        }
    }
    if kept.is_empty() {
        return None;
    }
    // Text the next user could not read means the default palette instead.
    if !readable(&kept) {
        return None;
    }
    let mut out = String::from(":root {\n");
    for (token, [r, g, b]) in &kept {
        out.push_str(&format!("    --{token}: #{r:02x}{g:02x}{b:02x};\n"));
    }
    out.push_str("}\n");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matugen_palette_survives_unchanged_in_meaning() {
        let css = ":root {\n    --surface: #282a36;\n    --primary: #BD93F9;\n    \
                   --outline-variant: rgba(68, 71, 90, 0.5);\n}";
        assert_eq!(
            sanitize(css).as_deref(),
            Some(
                ":root {\n    --surface: #282a36;\n    --primary: #bd93f9;\n    \
                 --outline-variant: #44475a;\n}\n"
            )
        );
    }

    #[test]
    fn styling_beyond_the_palette_is_refused() {
        // Another selector, an import, a url, a comment: discarded whole.
        for css in [
            ":root { --primary: #fff; } label { opacity: 0; }",
            "@import url(\"file:///var/lib/mgreet/avatar\"); :root { --primary: #fff; }",
            "window { background-image: url(\"file:///var/lib/mgreet/avatar\"); }",
            ":root { --primary: #fff; /* } label { opacity: 0 } */ }",
            "a { }",
            "",
        ] {
            assert_eq!(sanitize(css), None, "{css}");
        }
    }

    #[test]
    fn unknown_tokens_and_non_colour_values_are_dropped() {
        let css = ":root { --primary: #fff; --opacity: 0; --surface: url(x); \
                   --error: red; --outline: rgb(1,2,3) !important; color: #000; }";
        assert_eq!(
            sanitize(css).as_deref(),
            Some(":root {\n    --primary: #ffffff;\n}\n")
        );
        assert_eq!(sanitize(":root { --surface: url(x); }"), None);
    }

    #[test]
    fn colour_literals() {
        for ok in [
            "#abc",
            "#abcd",
            "#a1b2c3",
            "#a1b2c3d4",
            "rgb(1, 2, 3)",
            "rgba(1,2,3,0.5)",
            "rgb(1 2 3)",
            "rgb(10%, 20%, 30%)",
        ] {
            assert!(parse_colour(ok).is_some(), "{ok}");
        }
        for bad in [
            "#ab",
            "#ggg",
            "rgb()",
            "rgb(var(--x))",
            "rgba(1,2,3",
            "red",
            "url(x)",
            "rgb(1, 2)",
            "rgba(1, 2, 3, x)",
            "#abcde",
        ] {
            assert!(parse_colour(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn alpha_is_dropped_so_every_colour_is_opaque() {
        let css = ":root { --secondary-container: #0000; --secondary: #a1b2c3d4; \
                   --tertiary: rgba(1, 2, 3, 0); --outline: #ABC; --error: rgb(100%, 0%, 50%); }";
        assert_eq!(
            sanitize(css).as_deref(),
            Some(
                ":root {\n    --secondary-container: #000000;\n    --secondary: #a1b2c3;\n    \
                 --tertiary: #010203;\n    --outline: #aabbcc;\n    --error: #ff0080;\n}\n"
            )
        );
    }

    #[test]
    fn a_palette_that_hides_the_login_text_is_refused_whole() {
        // Each of these leaves the next user's username/password labels (or
        // the login button text) unreadable on the greeter.
        for css in [
            // Transparent text: alpha is dropped, black on the dark surface.
            ":root { --on-surface: #0000; }",
            // Text painted in the default surface colour.
            ":root { --on-surface: #282a36; }",
            // The surface painted in the default text colour.
            ":root { --surface: #f8f8f2; }",
            // A whole consistent palette with text = background.
            ":root { --surface: #123456; --surface-container-lowest: #123456; \
             --surface-container: #123456; --surface-container-high: #123456; \
             --surface-container-highest: #123456; --on-surface: #123456; }",
            ":root { --primary: #bd93f9; --on-primary: #bd93f9; }",
            ":root { --on-surface-variant: rgba(0, 0, 0, 0); }",
        ] {
            assert_eq!(sanitize(css), None, "{css}");
        }
    }

    #[test]
    fn contrast_uses_the_last_declaration_just_like_css() {
        assert_eq!(
            sanitize(":root { --on-surface: #ffffff; --on-surface: #282a36; }"),
            None,
            "a second declaration must not bypass the contrast check"
        );
    }

    #[test]
    fn the_baked_defaults_are_readable() {
        assert!(sanitize(&defaults_css()).is_some());
    }

    #[test]
    fn a_light_palette_is_kept() {
        let css = ":root { --surface: #fdf8fd; --surface-container-lowest: #ffffff; \
                   --surface-container: #f1ecf1; --surface-container-high: #ebe6eb; \
                   --surface-container-highest: #e6e1e6; --on-surface: #1c1b1e; \
                   --on-surface-variant: #49454e; --primary: #6750a4; --on-primary: #ffffff; \
                   --primary-container: #e9ddff; --on-primary-container: #22005d; \
                   --error: #ba1a1a; }";
        assert!(sanitize(css).is_some());
    }

    fn defaults_css() -> String {
        let mut css = String::from(":root {");
        for (token, value) in GREETER_DEFAULTS {
            css.push_str(&format!(" --{token}: {value};"));
        }
        css.push('}');
        css
    }
}
