//! Reusable tile widget for the Control Center grid.
//!
//! A tile is a `gtk::Button` (so it's clickable) with a flat icon box
//! (no coloured chip — the WHOLE button fills with `--primary` when active,
//! GNOME quick-settings style) and an optional vertical label stack.
//! Returns a `TileWidget` handle that the caller uses to update icon,
//! subtitle, and active state imperatively.
//!
//! Variants:
//! - normal: icon + title + subtitle
//! - expandable: same layout + a trailing `>` chevron (`go-next-symbolic`)
//! - `.small` css class: icon only, no labels

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};

/// Handle returned by `build_tile` / `build_expand_tile` / `build_small_tile`.
/// The caller uses these references to update the tile's live state without a
/// full relm4 component.
pub(crate) struct TileWidget {
    /// The root button — attach to the grid.
    pub(crate) button: gtk::Button,
    /// Icon widget inside the chip box.
    pub(crate) icon: gtk::Image,
    /// Subtitle label (hidden on small tiles).
    pub(crate) subtitle: gtk::Label,
}

impl TileWidget {
    /// Set the `.active` CSS class on the tile button.
    pub(crate) fn set_active(&self, active: bool) {
        if active {
            self.button.add_css_class("active");
        } else {
            self.button.remove_css_class("active");
        }
    }

    /// Update the subtitle text.
    pub(crate) fn set_subtitle(&self, text: &str) {
        self.subtitle.set_label(text);
    }

    /// Update the icon name.
    pub(crate) fn set_icon(&self, icon_name: &str) {
        self.icon.set_icon_name(Some(icon_name));
    }
}

/// Build a normal tile (icon + title + subtitle, no chevron).
/// Returns a `TileWidget` handle.
pub(crate) fn build_tile(icon_name: &str, title: &str, subtitle: &str) -> TileWidget {
    build_tile_inner(icon_name, title, subtitle, false)
}

/// Build an expandable tile (icon + title + subtitle + trailing `>` chevron).
/// Use for tiles that open a detail sub-page (Wi-Fi, Bluetooth, etc.).
pub(crate) fn build_expand_tile(icon_name: &str, title: &str, subtitle: &str) -> TileWidget {
    build_tile_inner(icon_name, title, subtitle, true)
}

/// Internal helper shared by `build_tile` and `build_expand_tile`.
fn build_tile_inner(icon_name: &str, title: &str, subtitle: &str, expandable: bool) -> TileWidget {
    let button = gtk::Button::new();
    button.add_css_class("control-center-tile");

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 10);

    // Icon box — flat, no background fill (the whole button carries colour).
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    chip.add_css_class("control-center-tile-icon");
    chip.set_halign(gtk::Align::Center);
    chip.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    // Flat icon, no chip box (the tonal square was removed — it read as
    // misaligned). Vertically centre the glyph against the 2-line label;
    // horizontally every tile's icon sits at the same leading edge and the
    // column is uniform because each symbolic icon is an --icon-md square.
    icon.set_valign(gtk::Align::Center);
    chip.append(&icon);

    // Label stack — hexpand so the chevron is pushed to the trailing edge.
    let label_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    label_box.set_valign(gtk::Align::Center);
    label_box.set_hexpand(true);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("control-center-tile-title");
    title_label.set_halign(gtk::Align::Start);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.add_css_class("control-center-tile-subtitle");
    subtitle_label.set_halign(gtk::Align::Start);
    subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    label_box.append(&title_label);
    label_box.append(&subtitle_label);

    outer.append(&chip);
    outer.append(&label_box);

    // Trailing chevron — only on expandable tiles.
    if expandable {
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("control-center-tile-chevron");
        chevron.set_halign(gtk::Align::End);
        chevron.set_valign(gtk::Align::Center);
        outer.append(&chevron);
    }

    button.set_child(Some(&outer));

    TileWidget {
        button,
        icon,
        subtitle: subtitle_label,
    }
}
