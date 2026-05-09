//! `text-input-v3` + `input-method-v2` handlers.
//!
//! Qt's `text-input-v3` plugin backs every `QML.TextInput` field on
//! Wayland. It probes for both `wp_text_input_v3` and
//! `zwp_input_method_v2` globals at activate-time; if either one is
//! missing, Qt falls back to a degraded path where keystrokes are
//! NOT routed to the focused TextInput even though `wl_keyboard.key`
//! is being delivered to the surface (the noctalia lock-screen
//! "password field stays empty" symptom).
//!
//! Smithay handles all the protocol plumbing as long as the globals
//! are registered. We do NOT drive an IME ourselves (no fcitx/ibus
//! integration here), so the handler is intentionally minimal:
//! input-method popups get tracked through the regular xdg popup
//! manager so they render at the right location, and dismissal
//! hooks back into PopupManager.

use smithay::{
    delegate_input_method_manager, delegate_text_input_manager,
    desktop::{PopupKind, PopupManager},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::{
        input_method::{InputMethodHandler, PopupSurface as InputMethodPopupSurface},
        seat::WaylandFocus,
    },
};

use crate::state::MargoState;

impl InputMethodHandler for MargoState {
    fn new_popup(&mut self, surface: InputMethodPopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("input_method: failed to track popup: {err}");
        }
    }

    fn popup_repositioned(&mut self, _surface: InputMethodPopupSurface) {}

    fn dismiss_popup(&mut self, surface: InputMethodPopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        // Look up the parent toplevel and report its window-geometry
        // so input-method popups (e.g. fcitx candidate window) can
        // position relative to the cursor inside the focused window.
        self.space
            .elements()
            .find_map(|w| (w.wl_surface().as_deref() == Some(parent)).then(|| w.geometry()))
            .unwrap_or_default()
    }
}
delegate_text_input_manager!(MargoState);
delegate_input_method_manager!(MargoState);
