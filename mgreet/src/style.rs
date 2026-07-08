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
