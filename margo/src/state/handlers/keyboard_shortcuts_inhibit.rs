//! `zwp_keyboard_shortcuts_inhibit_v1` handler.
//!
//! Clients that need to grab every keystroke (vncviewer, RDP clients,
//! VirtualBox, browser-based remote-desktop apps) request an inhibitor
//! for their focused surface. While the inhibitor is active, margo
//! forwards every key (including its own keybindings — Super, Alt+Tab,
//! etc.) straight to the client, so the keys reach the guest session.
//!
//! Policy: auto-activate on creation, mirroring niri. A future
//! confirmation-dialog policy can hook in via `new_inhibitor` if a
//! security/UX preference is added.
//!
//! The actual short-circuit lives in `input_handler.rs`'s
//! `keyboard.input(...)` filter — this file owns the protocol lifecycle
//! only.

use smithay::{
    delegate_keyboard_shortcuts_inhibit,
    reexports::wayland_server::Resource,
    wayland::keyboard_shortcuts_inhibit::{
        KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
        KeyboardShortcutsInhibitor,
    },
};

use crate::state::MargoState;

impl KeyboardShortcutsInhibitHandler for MargoState {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibitor.activate();
        let surface = inhibitor.wl_surface().clone();
        tracing::info!(
            surface = ?surface.id(),
            "keyboard-shortcuts-inhibit: new inhibitor (auto-activated)",
        );
        self.keyboard_shortcuts_inhibiting_surfaces
            .insert(surface, inhibitor);
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        let surface = inhibitor.wl_surface().clone();
        tracing::debug!(
            surface = ?surface.id(),
            "keyboard-shortcuts-inhibit: inhibitor destroyed",
        );
        self.keyboard_shortcuts_inhibiting_surfaces.remove(&surface);
    }
}
delegate_keyboard_shortcuts_inhibit!(MargoState);
