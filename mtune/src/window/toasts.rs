// SPDX-License-Identifier: GPL-3.0-or-later

//! In-window toasts (routed to the Radio & Podcasts dialog while it covers
//! the window).

use super::*;

impl Window {
    pub fn add_toast(&self, msg: String) {
        let toast = adw::Toast::new(&msg);
        self.toast_target().add_toast(toast);
    }

    /// The window's overlay, or the Radio & Podcasts dialog's while it
    /// covers the window.
    fn toast_target(&self) -> adw::ToastOverlay {
        crate::discover::open_toasts().unwrap_or_else(|| self.imp().toast_overlay.get())
    }

    pub fn add_skip_to_toast(&self, msg: String, button: String, pos: u32) {
        let toast = adw::Toast::new(&msg);
        toast.set_button_label(Some(&button));
        toast.set_action_name(Some("win.skip-to"));
        toast.set_action_target_value(Some(&pos.to_variant()));
        self.toast_target().add_toast(toast);
    }
}
