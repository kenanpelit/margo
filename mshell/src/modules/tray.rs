//! System tray — stage-6 placeholder.
//!
//! Returns an empty `GtkBox` for now. Implementing
//! StatusNotifierItem (KDE/freedesktop spec) means standing up a
//! D-Bus watcher (`org.kde.StatusNotifierWatcher`), a host
//! (`org.kde.StatusNotifierHost-<pid>`), and per-item proxies that
//! expose the item's icon, tooltip and context menu via DBusMenu.
//! That's roughly the same scope as the notifications server, so
//! both land together in Stage 8.
//!
//! The placeholder keeps the bar's right region positioned the same
//! way it will be once the real tray slots in.

use gtk::prelude::*;
use gtk::{Box as GtkBox, Orientation};

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("tray")
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    row.add_css_class("module");
    row.add_css_class("tray");
    row
}
