//! XWayland client-lifecycle methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the `self.clients`-side of
//! XWayland — locating, registering and removing an `X11Surface`-backed
//! toplevel. This is distinct from `state::handlers::x11`, which is the
//! smithay `XWaylandShellHandler` that *reacts* to XWayland protocol events
//! and calls into these methods. `pub(crate)` because those handlers (a
//! sibling module, no longer a descendant of `state.rs`) invoke them.
//!
//! Pure `MargoState` glue; no new types.

use super::*;

impl MargoState {
    pub(crate) fn find_x11_client(&self, window: &X11Surface) -> Option<usize> {
        let id = window.window_id();
        self.clients.iter().position(|c| {
            matches!(c.window.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id)
        })
    }

    pub(crate) fn register_x11_window(&mut self, x11surface: X11Surface) {
        let window = Window::new_x11_window(x11surface);
        let mon_idx = self.focused_monitor();
        let tags = self
            .monitors
            .get(mon_idx)
            .map(|m| m.current_tagset())
            .unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, tags, &self.config);
        client.surface_type = crate::SurfaceType::X11;
        client.title = window.x11_surface().map(|s| s.title()).unwrap_or_default();
        client.app_id = window.x11_surface().map(|s| s.class()).unwrap_or_default();
        // XWayland toplevels opt out of the tiling slide/move animation by
        // default. Animating an X11 window's slot — the tag-switch slide in
        // particular — drives some X11 clients (notably TigerVNC's vncviewer)
        // into a state where they keep rendering the remote framebuffer live
        // but silently stop forwarding pointer/keyboard input to the remote,
        // and only a real resize kicks them back out of it. `animations = 0`
        // avoids it globally; scoping the opt-out to X11 keeps native Wayland
        // animations intact. Set before `apply_window_rules` so an explicit
        // `no_animation` window-rule can still override it per app.
        client.no_animation = true;
        self.apply_window_rules(&mut client);

        // Tag-home redirect: if a windowrule set `tags:N` but DIDN'T pin
        // a `monitor:`, route to the tag's home monitor as defined by
        // `tagrule = id:N, monitor_name:X`. Lets the user write
        //   tagrule = id:7, monitor_name:eDP-1
        //   windowrule = tags:7, appid:^transmission$
        // and the windowrule doesn't have to repeat `monitor:eDP-1`.
        let no_explicit_monitor = !self
            .matching_window_rules(&client.app_id, &client.title)
            .iter()
            .any(|r| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(client.tags) {
                client.monitor = home;
            }
        }

        let target_mon = client.monitor;
        let focus_new = !client.no_focus && !client.open_silent;
        let ft_handle = self
            .foreign_toplevel_list
            .new_toplevel::<Self>(&client.title, &client.app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);
        self.clients.push(client);
        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);
        if focus_new {
            self.focus_surface(Some(FocusTarget::Window(window)));
        }
        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }
        tracing::info!(
            app_id = %self.clients.last().map(|c| c.app_id.as_str()).unwrap_or(""),
            monitor = target_mon,
            "new x11 toplevel",
        );
        // Refresh xdp-gnome's window picker — same path the
        // Wayland finalize_initial_map handler uses.
        self.emit_windows_changed();
    }

    pub(crate) fn remove_x11_window(&mut self, x11surface: &X11Surface) {
        if let Some(idx) = self.find_x11_client(x11surface) {
            let app_id = self.clients[idx].app_id.clone();
            let title = self.clients[idx].title.clone();
            if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
                handle.send_closed();
            }
            let window = self.clients[idx].window.clone();
            let client_id = self.clients[idx].id;
            let group = self.group_of(idx);
            self.mru_remove_window(&window);
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.remove_focus_history_id(client_id);
            self.shift_indices_after_remove(idx);
            if let Some(gid) = group {
                self.repair_group(gid);
            }
            let mon_idx = self.focused_monitor();
            if !self.monitors.is_empty() {
                self.arrange_monitor(mon_idx);
            }
            // Refresh xdp-gnome's window picker — same path the
            // Wayland toplevel_destroyed handler uses.
            self.emit_windows_changed();
            crate::scripting::fire_window_close(self, &app_id, &title);
            return;
        }

        // Override-redirect window (menu / popup / tooltip). These are never
        // in `self.clients` — they live only in the space, mapped by
        // `mapped_override_redirect_window`. Without unmapping them here a
        // dismissed menu lingers in the space forever; the next menu from the
        // same client then opens on top of a stale element and the
        // `or_positions` handoff drifts (the classic "second open lands in the
        // wrong place" XWayland-menu symptom). Unmap it explicitly.
        let id = x11surface.window_id();
        let elem = self
            .space
            .elements()
            .find(
                |e| matches!(e.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id),
            )
            .cloned();
        if let Some(elem) = elem {
            self.space.unmap_elem(&elem);
            self.request_repaint();
        }
    }
}
