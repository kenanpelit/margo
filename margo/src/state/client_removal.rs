//! Removing a managed window from `MargoState` — the one path shared by
//! the Wayland (`toplevel_destroyed`) and X11 (`remove_x11_window`)
//! handlers, so the two can't drift apart again.

use super::*;

impl MargoState {
    /// Tear down `clients[idx]` after its toplevel went away: tell
    /// foreign-toplevel clients, queue the close animation, drop it from
    /// the space / MRU / focus history / group, recall focus if it had
    /// it, re-arrange its monitor, refresh the portal window list, and
    /// fire the `on_window_close` script hook once the state is
    /// consistent. Returns the closed window's `(app_id, title)`.
    pub(crate) fn remove_client(&mut self, idx: usize) -> (String, String) {
        let app_id = self.clients[idx].app_id.clone();
        let title = self.clients[idx].title.clone();
        if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
            handle.send_closed();
        }
        self.queue_close_animation(idx);

        let mon = self.clients[idx].monitor;
        // Decide before removal: afterwards the window is gone from
        // `clients` and neither question can be answered.
        let had_focus = self.focused_client_idx() == Some(idx);
        let was_selected = self
            .monitors
            .get(mon)
            .is_some_and(|m| m.selected == Some(idx));

        let window = self.clients[idx].window.clone();
        let client_id = self.clients[idx].id;
        // Capture the group before the client vanishes so we can
        // repair the one-active-member invariant afterwards.
        let group = self.group_of(idx);
        self.mru_remove_window(&window);
        self.space.unmap_elem(&window);
        if let Some(surface) = window.wl_surface() {
            self.forget_decoration_ids(&surface);
        }
        self.clients.remove(idx);
        self.remove_focus_history_id(client_id);
        self.shift_indices_after_remove(idx);
        self.session_end_check();
        if let Some(gid) = group {
            self.repair_group(gid);
        }

        if mon < self.monitors.len() {
            if had_focus || was_selected {
                self.recall_selection_after_close(mon, idx, had_focus);
            }
            // Re-arrange so the freed space is reclaimed (and the
            // scroller centres the new focus) on the monitor the window
            // was actually on — not whichever monitor has focus now.
            self.arrange_monitor(mon);
        }

        // Refresh xdp-gnome's window picker — see finalize_initial_map.
        self.emit_windows_changed();
        crate::scripting::fire_window_close(self, &app_id, &title);
        (app_id, title)
    }

    /// Pick the next `selected` window on `mon` after the client at
    /// `removed_idx` (already removed) closed: the previous selection
    /// (niri-style focus-stack recall), else the nearest visible window
    /// before the removed slot, else any visible one. Keyboard focus only
    /// moves when the closed window held it; a closed window that was
    /// merely the remembered selection of a background monitor just
    /// updates that monitor's selection.
    fn recall_selection_after_close(&mut self, mon: usize, removed_idx: usize, had_focus: bool) {
        let tagset = self.monitors[mon].current_tagset();
        let prev = self.monitors[mon].prev_selected;
        let target = prev
            .filter(|&i| i < self.clients.len() && self.clients[i].is_visible_on(mon, tagset))
            .or_else(|| {
                (0..self.clients.len())
                    .rev()
                    .find(|&i| i < removed_idx && self.clients[i].is_visible_on(mon, tagset))
            })
            .or_else(|| {
                self.clients
                    .iter()
                    .position(|c| c.is_visible_on(mon, tagset))
            });
        self.monitors[mon].selected = target;
        if had_focus {
            let focus = target.map(|i| FocusTarget::Window(self.clients[i].window.clone()));
            self.focus_surface(focus);
        }
    }

    /// Queue the close animation for `clients[idx]` (still present). The
    /// renderer captures the surface to a texture on its next frame — the
    /// wl_surface outlives the toplevel role — and then draws that texture
    /// scaled / faded out. Skipped when animations are off, the window
    /// opted out, or it has no wl_surface left to capture.
    fn queue_close_animation(&mut self, idx: usize) {
        if !self.config.animations
            || self.config.animation_duration_close == 0
            || self.clients[idx].no_animation
        {
            return;
        }
        let Some(surface) = self.clients[idx].window.wl_surface().map(|s| (*s).clone()) else {
            return;
        };
        let kind_str = self.clients[idx]
            .animation_type_close
            .clone()
            .unwrap_or_else(|| self.config.animation_type_close.clone());
        let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
        let border_radius = self.client_radius(idx);
        let c = &self.clients[idx];
        self.closing_clients.push(ClosingClient {
            id: smithay::backend::renderer::element::Id::new(),
            texture: None,
            capture_pending: true,
            geom: c.geom,
            monitor: c.monitor,
            tags: c.tags,
            time_started: crate::utils::now_ms(),
            duration: self.config.animation_duration_close,
            progress: 0.0,
            kind,
            extreme_scale: self.config.zoom_end_ratio.clamp(0.05, 1.0),
            border_radius,
            source_surface: Some(surface),
        });
        self.request_repaint();
    }
}
