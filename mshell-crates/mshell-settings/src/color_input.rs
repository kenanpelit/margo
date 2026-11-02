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
//! the entry needs (a `gdk::RGBA` view of `mshell_common::color`); the
//! entry itself is wired per-page so its `#[watch]` can bind to that page's
//! colour field.

use relm4::gtk::gdk;

use mshell_common::color::parse_hex_bytes;

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
