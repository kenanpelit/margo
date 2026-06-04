use gtk4_layer_shell;
use gtk4_layer_shell::LayerShell;
use relm4::gtk::prelude::WidgetExt;
use relm4::{RelmWidgetExt, gtk};

pub fn wire_entry_focus(entry: &gtk::Entry) {
    entry.connect_realize(|entry| {
        let fc = gtk::EventControllerFocus::new();
        if let Some(window) = entry.toplevel_window() {
            let window_clone = window.clone();
            let entry_clone = entry.clone();
            fc.connect_enter(move |_| {
                // Only flip the host layer surface to Exclusive when the
                // entry is actually on screen. Menus are built eagerly per
                // monitor, so this entry lives in the widget tree even while
                // its menu is collapsed/hidden; at frame map-time GTK can
                // hand initial keyboard focus to this (unmapped) entry,
                // whose `connect_enter` would then strand the *whole frame*
                // in Exclusive keyboard mode with no menu revealed —
                // trapping Wayland keyboard focus in the invisible
                // full-screen layer until something runs the frame's
                // `sync_keyboard_mode` (e.g. Esc → CloseMenus). When the
                // menu is genuinely open the frame is already Exclusive via
                // `sync_keyboard_mode`, so this only ever needs to act for a
                // mapped entry.
                if !entry_clone.is_mapped() {
                    tracing::info!("key_mode: entry focus ENTER ignored (entry not mapped)");
                    return;
                }
                tracing::info!("key_mode: entry focus ENTER -> Exclusive");
                window_clone.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::Exclusive);
            });

            let window_clone = window.clone();
            fc.connect_leave(move |_| {
                window_clone.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
            });

            entry.add_controller(fc);
        }
    });
}
