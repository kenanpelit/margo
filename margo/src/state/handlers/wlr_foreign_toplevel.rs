//! Handler: maps `zwlr_foreign_toplevel_management_v1` write-side requests
//! to margo window actions.
//!
//! - `activate`        → switch to the toplevel's tag (if hidden) + focus + raise
//!   (same path as `xdg-activation`'s `request_activation`).
//! - `close`           → `send_close` (Wayland) / X11 close.
//! - `(un)fullscreen`  → `set_client_fullscreen`.
//! - maximize/minimize → no-op (margo is a tiling WM).

use smithay::desktop::WindowSurface;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;

use crate::protocols::wlr_foreign_toplevel::{WlrForeignToplevelHandler, WlrForeignToplevelState};
use crate::state::{FocusTarget, MargoState};

impl MargoState {
    /// Index into `self.clients` of the client owning `surface`, if any.
    pub(crate) fn client_idx_for_surface(&self, surface: &WlSurface) -> Option<usize> {
        self.clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(surface))
    }

    /// Bring the toplevel backing `surface` to the foreground: jump to its
    /// tag if it isn't currently visible, then focus + raise. Shared by
    /// `xdg-activation` and wlr foreign-toplevel `activate`.
    pub(crate) fn activate_window_surface(&mut self, surface: &WlSurface) {
        if let Some(idx) = self.client_idx_for_surface(surface) {
            self.activate_window_idx(idx);
        }
    }

    /// Focus a window by its stable, monotonic `id` (published in `state
    /// snapshot`), resolving it to the current slot index. Unlike
    /// [`Self::activate_window_idx`] this is race-free: the id never aliases a
    /// different window even if the `clients` Vec shifted (another window
    /// closed) between the shell reading the snapshot and this dispatch landing.
    /// Backs the `focuswindowid` IPC dispatch. No-op if the id is gone.
    pub(crate) fn activate_window_id(&mut self, id: u64) {
        if let Some(idx) = self.clients.iter().position(|c| c.id == id) {
            self.activate_window_idx(idx);
        }
    }

    /// Same as [`Self::activate_window_surface`] but addressed by client
    /// index (the `idx` published in `state snapshot`). Used by the
    /// `focuswindow` IPC dispatch so the dock can focus the exact window
    /// a user clicks, not just jump to its tag.
    pub(crate) fn activate_window_idx(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }

        let mask = self.clients[idx].tags;
        let mon_idx = self.clients[idx].monitor;
        let already_visible = self
            .monitors
            .get(mon_idx)
            .map(|m| (mask & m.current_tagset()) != 0)
            .unwrap_or(false);
        if !already_visible {
            // Lowest set bit, mirroring xdg-activation's tag jump.
            let one_bit = mask & mask.wrapping_neg();
            let target = if one_bit != 0 { one_bit } else { mask };
            self.view_tag(target);
        }

        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window.clone())));
        self.space.raise_element(&window, true);
        self.enforce_z_order();
        self.request_repaint();
    }
}

impl WlrForeignToplevelHandler for MargoState {
    fn wlr_foreign_toplevel_state(&mut self) -> &mut WlrForeignToplevelState {
        &mut self.wlr_foreign_toplevel
    }

    fn wlr_ftl_activate(&mut self, surface: WlSurface) {
        self.activate_window_surface(&surface);
    }

    fn wlr_ftl_close(&mut self, surface: WlSurface) {
        if let Some(idx) = self.client_idx_for_surface(&surface) {
            match self.clients[idx].window.underlying_surface() {
                WindowSurface::Wayland(toplevel) => toplevel.send_close(),
                WindowSurface::X11(x11) => {
                    let _ = x11.close();
                }
            }
        }
    }

    fn wlr_ftl_set_fullscreen(&mut self, surface: WlSurface) {
        if let Some(idx) = self.client_idx_for_surface(&surface) {
            self.set_client_fullscreen(idx, true);
        }
    }

    fn wlr_ftl_unset_fullscreen(&mut self, surface: WlSurface) {
        if let Some(idx) = self.client_idx_for_surface(&surface) {
            self.set_client_fullscreen(idx, false);
        }
    }
}

crate::delegate_wlr_foreign_toplevel!(MargoState);
