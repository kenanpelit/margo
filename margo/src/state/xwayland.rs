//! XWayland client-lifecycle methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the `self.clients`-side of
//! XWayland — locating, registering and removing an `X11Surface`-backed
//! toplevel. This is distinct from `state::handlers::x11`, which is the
//! smithay `XWaylandShellHandler` that *reacts* to XWayland protocol events
//! and calls into these methods. `pub(crate)` because those handlers (a
//! sibling module, no longer a descendant of `state.rs`) invoke them.
//!
//! Mostly `MargoState` glue, plus the pure `ConfigureRequest` decision
//! ([`x11_configure_reply`]) so it is testable without an XWayland.

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
        // _NET_WM_PID, as XWayland reports it (unset for some clients).
        client.pid = window.x11_surface().and_then(|s| s.pid()).unwrap_or(0);
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
            .any(|(_, r)| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(client.tags) {
                client.monitor = home;
            }
        }

        let target_mon = client.monitor;
        let focus_new = !client.no_focus && !client.open_silent;
        let ft_handle = self.protocols.foreign_toplevel_list.new_toplevel::<Self>(
            crate::utils::clamp_wire_str(&client.title),
            crate::utils::clamp_wire_str(&client.app_id),
        );
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);
        let app_id = client.app_id.clone();
        match self.new_client_position(target_mon) {
            Some(pos) => {
                self.clients.insert(pos, client);
                self.shift_indices_at_or_after(pos);
            }
            None => self.clients.push(client),
        }
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
            app_id = %app_id,
            monitor = target_mon,
            "new x11 toplevel",
        );
        // Refresh xdp-gnome's window picker — same path the
        // Wayland finalize_initial_map handler uses.
        self.emit_windows_changed();
    }

    pub(crate) fn remove_x11_window(&mut self, x11surface: &X11Surface) {
        if let Some(idx) = self.find_x11_client(x11surface) {
            // Same teardown as a Wayland toplevel (focus recall, close
            // animation, arrange of the window's own monitor, hooks).
            self.remove_client(idx);
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

/// What margo answers an X11 `ConfigureRequest` with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum X11ConfigureReply {
    /// A compositor-placed window (tiled or fullscreen): re-send its
    /// current geometry and ignore the request.
    Reassert(Rect),
    /// A managed floating window: accept the request (clamped) and
    /// remember it as the window's floating geometry.
    Float(Rect),
    /// A window margo doesn't manage yet: echo the request.
    Echo(Rect),
}

/// The managed client's side of [`x11_configure_reply`]: `fullscreen` is
/// any fullscreen mode other than off.
#[derive(Debug, Clone, Copy)]
pub(crate) struct X11ManagedGeometry {
    pub is_floating: bool,
    pub fullscreen: bool,
    pub geom: Rect,
}

pub(crate) fn x11_configure_reply(
    managed: Option<X11ManagedGeometry>,
    current: Rect,
    x: Option<i32>,
    y: Option<i32>,
    w: Option<u32>,
    h: Option<u32>,
) -> X11ConfigureReply {
    let requested = Rect {
        x: x.unwrap_or(current.x),
        y: y.unwrap_or(current.y),
        width: w.map_or(current.width, |v| i32::try_from(v).unwrap_or(i32::MAX)),
        height: h.map_or(current.height, |v| i32::try_from(v).unwrap_or(i32::MAX)),
    };
    let Some(m) = managed else {
        return X11ConfigureReply::Echo(requested);
    };
    if m.is_floating && !m.fullscreen {
        // Same bounds `movewin` / `resizewin` keep floats in.
        const MAX: i32 = crate::protocols::output_management::MAX_POSITION;
        const MIN_SIZE: i32 = super::dispatch::MIN_FLOAT_SIZE;
        return X11ConfigureReply::Float(Rect {
            x: requested.x.clamp(-MAX, MAX),
            y: requested.y.clamp(-MAX, MAX),
            width: requested.width.clamp(MIN_SIZE, MAX),
            height: requested.height.clamp(MIN_SIZE, MAX),
        });
    }
    if m.geom.width > 0 && m.geom.height > 0 {
        return X11ConfigureReply::Reassert(m.geom);
    }
    // Not placed yet (the first arrange hasn't run): nothing to re-assert.
    X11ConfigureReply::Echo(requested)
}
