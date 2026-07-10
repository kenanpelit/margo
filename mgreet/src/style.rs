//! Greeter stylesheet: the grass-compiled `style.scss` baked into the binary,
//! plus runtime application (base sheet + optional matugen colour overlay).

use gtk4::gdk::Display;
use gtk4::{CssProvider, STYLE_PROVIDER_PRIORITY_APPLICATION, STYLE_PROVIDER_PRIORITY_USER};

/// The compiled base stylesheet (default Dracula palette + widget styling).
const STYLE_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

/// Install the greeter styling on `display`. The base sheet carries a complete
/// default palette so the greeter always renders; if a matugen colours CSS is
/// found it is layered on top at USER priority so the login tracks the theme.
///
/// `matugen_css` is optional CSS text of the form `:root { --primary: #…; … }`.
pub fn install(display: &Display, matugen_css: Option<&str>) {
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
}

#[cfg(test)]
mod tests {
    use super::STYLE_CSS;

    /// Exactly the custom properties `mshell-matugen::css_mapping::to_css`
    /// emits. The overlay sets these and nothing else.
    const MATUGEN_TOKENS: &[&str] = &[
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
