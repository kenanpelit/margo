//! Reusable tile widget for the Control Center grid.
//!
//! A tile is a `gtk::Button` (so it's clickable) with a rounded icon-chip
//! and an optional vertical label stack. Returns a `TileWidget` handle that
//! the caller uses to update icon, subtitle, and active state imperatively.
//!
//! Variants:
//! - normal: icon + title + subtitle
//! - `.wide` css class: spans both grid columns (attach at col 0, span 2)
//! - `.small` css class: icon-chip only, no labels (Dark Mode / Night Light)

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};

/// Handle returned by `build_tile` / `build_small_tile`. The caller uses
/// these references to update the tile's live state without a full relm4 component.
pub(crate) struct TileWidget {
    /// The root button — attach to the grid.
    pub(crate) button: gtk::Button,
    /// Icon widget inside the chip.
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

    /// Show or hide the whole tile.
    #[allow(dead_code)]
    pub(crate) fn set_visible(&self, visible: bool) {
        self.button.set_visible(visible);
    }
}

/// Build a normal tile (icon + title + subtitle). Apply `.wide` externally
/// if needed. Returns a `TileWidget` handle.
pub(crate) fn build_tile(icon_name: &str, title: &str, subtitle: &str) -> TileWidget {
    let button = gtk::Button::new();
    button.add_css_class("control-center-tile");

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 10);

    // Icon chip
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    chip.add_css_class("control-center-tile-icon");
    chip.set_halign(gtk::Align::Center);
    chip.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    chip.append(&icon);

    // Label stack
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
    button.set_child(Some(&outer));

    TileWidget {
        button,
        icon,
        subtitle: subtitle_label,
    }
}

/// Build a small tile (icon-chip only, no labels). Gets the `.small` CSS class.
#[allow(dead_code)]
pub(crate) fn build_small_tile(icon_name: &str) -> TileWidget {
    let button = gtk::Button::new();
    button.add_css_class("control-center-tile");
    button.add_css_class("small");

    // Icon chip
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    chip.add_css_class("control-center-tile-icon");
    chip.set_halign(gtk::Align::Center);
    chip.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    chip.append(&icon);

    button.set_child(Some(&chip));

    // Small tiles have no subtitle; use a dummy hidden label as placeholder.
    let dummy_subtitle = gtk::Label::new(None);
    dummy_subtitle.set_visible(false);

    TileWidget {
        button,
        icon,
        subtitle: dummy_subtitle,
    }
}
