//! Greeter stylesheet: the grass-compiled `style.scss` baked into the binary,
//! plus runtime application (base sheet + optional matugen colour overlay).

use gtk4::gdk::Display;
use gtk4::{CssProvider, STYLE_PROVIDER_PRIORITY_APPLICATION, STYLE_PROVIDER_PRIORITY_USER};

/// The compiled base stylesheet (default Dracula palette + widget styling).
const STYLE_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

/// Install the greeter styling on `display`. The base sheet carries a complete
/// default palette so the greeter always renders; if a matugen colours CSS is
/// found it is layered on top at USER priority so the login tracks the theme;
/// and an admin theme (`[display] greeter_css`, via `MLOGIND_CSS`) is layered
/// over both, so it wins whatever the wallpaper sync last wrote.
///
/// `matugen_css` and `admin_css` are optional CSS text, typically of the form
/// `:root { --primary: #…; … }` — though the admin file may restyle anything.
pub fn install(display: &Display, matugen_css: Option<&str>, admin_css: Option<&str>) {
    let base = CssProvider::new();
    base.load_from_string(STYLE_CSS);
    gtk4::style_context_add_provider_for_display(
        display,
        &base,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    if let Some(css) = matugen_css {
        let overlay = CssProvider::new();
        overlay.load_from_string(css);
        gtk4::style_context_add_provider_for_display(
            display,
            &overlay,
            STYLE_PROVIDER_PRIORITY_USER,
        );
    }

    if let Some(css) = admin_css {
        let theme = CssProvider::new();
        theme.load_from_string(css);
        // One notch above the matugen overlay: equal-priority ordering is a
        // GTK implementation detail, and "the admin file wins" should not
        // rest on one.
        gtk4::style_context_add_provider_for_display(
            display,
            &theme,
            STYLE_PROVIDER_PRIORITY_USER + 1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::STYLE_CSS;

    /// Exactly the custom properties `mshell-matugen::css_mapping::to_css`
    /// emits. The overlay sets these and nothing else: both mlogind (before it
    /// publishes) and `matugen_css` (when it loads) keep only these.
    const MATUGEN_TOKENS: &[&str] = mlogind_proto::theme_css::TOKENS;

    /// Geometry. Ours alone; matugen has no opinion about a corner radius.
    const OURS: &[&str] = &["radius-lg", "radius-md", "radius-pill"];

    /// Every `var(--name)` in the compiled stylesheet.
    fn used() -> Vec<String> {
        STYLE_CSS
            .match_indices("var(--")
            .map(|(i, m)| {
                STYLE_CSS[i + m.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect()
            })
            .collect()
    }

    /// Every `--name:` declared in the `:root` block.
    fn declared() -> Vec<String> {
        let start = STYLE_CSS.find(":root").expect("the baked palette exists");
        let block = &STYLE_CSS[start..];
        let end = block.find('}').expect(":root is closed");
        block[..end]
            .match_indices("--")
            .map(|(i, m)| {
                block[i + m.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_sanitiser_checks_contrast_against_the_baked_palette() {
        // `theme_css::sanitize` judges an overlay's legibility against the
        // tokens it leaves to this palette, so its copy must be this palette.
        let start = STYLE_CSS.find(":root").expect("the baked palette exists");
        let block = &STYLE_CSS[start..];
        let block = &block[..block.find('}').expect(":root is closed")];
        let mut baked: Vec<(String, String)> = block
            .split(['{', ';'])
            .filter_map(|decl| {
                let (name, value) = decl.split_once(':')?;
                let name = name.trim().strip_prefix("--")?;
                let value = value.trim();
                value
                    .starts_with('#')
                    .then(|| (name.to_string(), value.to_ascii_lowercase()))
            })
            .collect();
        baked.sort();
        let mut copy: Vec<(String, String)> = mlogind_proto::theme_css::GREETER_DEFAULTS
            .iter()
            .map(|(t, v)| (t.to_string(), v.to_string()))
            .collect();
        copy.sort();
        assert_eq!(baked, copy);
    }

    #[test]
    fn every_token_used_has_a_baked_default() {
        // Without one the greeter renders an unset colour — black on black —
        // whenever the matugen overlay is missing, which is every first boot.
        let declared = declared();
        for token in used() {
            assert!(
                declared.contains(&token),
                "--{token} is used but never declared in :root"
            );
        }
    }

    #[test]
    fn every_colour_token_is_one_matugen_can_override() {
        // The overlay is a second CssProvider that only sets the M3 names. A
        // colour outside that set can never be overridden: it silently keeps its
        // Dracula value while everything around it turns. `--bg` and `--danger`
        // were exactly that, and the dim over the wallpaper stayed cold blue-grey
        // under a warm palette until this test existed.
        for token in used() {
            if OURS.contains(&token.as_str()) {
                continue;
            }
            assert!(
                MATUGEN_TOKENS.contains(&token.as_str()),
                "--{token} is a colour matugen never writes; \
                 use an M3 name from mshell-matugen::css_mapping::to_css"
            );
        }
    }

    #[test]
    fn the_baked_palette_declares_nothing_it_does_not_use() {
        // A default for a token nobody reads is a colour that looks maintained
        // and is not.
        let used = used();
        for token in declared() {
            assert!(
                used.contains(&token),
                "--{token} is declared in :root but never used"
            );
        }
    }
}
