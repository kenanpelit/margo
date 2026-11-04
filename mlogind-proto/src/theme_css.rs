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
//! its own serialisation, never the input's bytes. Both ends use it: mlogind
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

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, or `rgb(…)`/`rgba(…)` holding only
/// numbers, `.`, `%`, `,` and spaces.
fn is_colour(value: &str) -> bool {
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let args = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'));
    args.is_some_and(|args| {
        !args.trim().is_empty()
            && args
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'%' | b',' | b' '))
    })
}

/// Reduce `css` to the palette declarations it carries, re-serialised as one
/// `:root` block. `None` when the input is anything but a single `:root { … }`
/// block (another selector, an at-rule, a comment, trailing text), or when no
/// declaration survives. A declaration with an unknown token or a value that is
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

    let mut out = String::from(":root {\n");
    let mut kept = 0usize;
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
        if TOKENS.contains(&token) && is_colour(value) {
            out.push_str(&format!("    --{token}: {value};\n"));
            kept += 1;
        }
    }
    out.push_str("}\n");
    (kept > 0).then_some(out)
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
                ":root {\n    --surface: #282a36;\n    --primary: #BD93F9;\n    \
                 --outline-variant: rgba(68, 71, 90, 0.5);\n}\n"
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
            Some(":root {\n    --primary: #fff;\n}\n")
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
        ] {
            assert!(is_colour(ok), "{ok}");
        }
        for bad in [
            "#ab",
            "#ggg",
            "rgb()",
            "rgb(var(--x))",
            "rgba(1,2,3",
            "red",
            "url(x)",
        ] {
            assert!(!is_colour(bad), "{bad}");
        }
    }
}
