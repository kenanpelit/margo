//! `xdg-shell` toplevel + popup handler — the protocol that backs
//! every regular Wayland window (Firefox, kitty, Helium, mpv, GTK,
//! Qt, you name it). The biggest single handler in margo because
//! the toplevel lifecycle is what drives most of the compositor's
//! animation / layout / focus state machines.
//!
//! Key responsibilities:
//!
//! * `new_toplevel` — defer the initial map until the client's first
//!   commit so app_id / title arrive before window-rules apply
//!   (eliminates the "CopyQ flickers between default and rule-driven
//!   geometry" bug). Smart-insert the new client right after the
//!   focused one in scroller layout.
//! * `move_request` / `resize_request` — wire up the smithay grabs
//!   that translate `xdg_toplevel.move/resize` requests into live
//!   pointer drags. Defined in `crate::input::grabs`.
//! * `grab` — direct-focus path for `xdg_popup.grab`. The full
//!   smithay `PopupKeyboardGrab` chain doesn't compose with our
//!   `FocusTarget` (SessionLock targets don't always have a stable
//!   wl_surface relationship), so we side-step it. Works for the
//!   99 % case (single-level popups: GTK file chooser, Chromium
//!   dropdown, GIMP context menu) and walks the same path for
//!   nested popups so focus tracks the latest level.
//! * `toplevel_destroyed` — capture the close animation snapshot
//!   BEFORE removing the client; re-focus prefers the previous
//!   focus (niri-style focus stack recall) before falling back to
//!   the spatially nearest visible client.
//! * `fullscreen_request` / `unfullscreen_request` /
//!   `maximize_request` / `unmaximize_request` — all four route
//!   through `set_client_fullscreen` which flips
//!   `xdg_toplevel::State::Fullscreen` in pending state and sends
//!   the configure. smithay's defaults are `send_configure()` with
//!   no state change, which leaves Helium / Firefox / mpv stuck
//!   windowed even after the client requested fullscreen.
//! * `minimize_request` — `minimize_client` (`state/scratchpad/minimize.rs`).

use smithay::{
    desktop::{PopupKind, Window},
    input::Seat,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_output::WlOutput, wl_seat::WlSeat},
        },
    },
    utils::{Logical, Point, Serial, Size},
    wayland::{
        seat::WaylandFocus,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
    },
};

use crate::state::{FocusTarget, MargoClient, MargoState};

impl XdgShellHandler for MargoState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.protocols.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let (app_id, title) = super::super::read_toplevel_identity(&surface);

        let window = Window::new_wayland_window(surface.clone());
        let mon_idx = self.focused_monitor();
        let initial_tags = self
            .monitors
            .get(mon_idx)
            .map(|m| m.current_tagset())
            .unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, initial_tags, &self.config);
        client.app_id = app_id.clone();
        client.title = title.clone();
        // The owning process, from the socket's SO_PEERCRED. state.json /
        // `mctl clients` publish it so tools can map a process (say, the
        // terminal hosting a Claude Code session) to its window.
        client.pid = surface
            .wl_surface()
            .client()
            .and_then(|c| c.get_credentials(&self.display_handle).ok())
            .and_then(|cred| u32::try_from(cred.pid).ok())
            .unwrap_or(0);

        // Defer the actual map / rule-application / arrange / focus
        // until the first commit. Qt clients (CopyQ, KeePassXC, the
        // GTK file picker via `pcmanfm-qt`, …) almost always create
        // the xdg_toplevel role *before* sending `set_app_id`, so at
        // this point `app_id` is empty and any windowrule keyed on
        // `appid:^copyq$` doesn't fire. Holding the map until the
        // first commit (when Qt has had its chance to set app_id and
        // we can look up the right rules) eliminates the visible
        // "open → snap to rule-driven geometry" flicker.
        client.is_initial_map_pending = true;

        // Tell the client its output's scale before its first buffer, so a
        // fractional-scale-aware app renders that first frame sharp. The
        // arrange at map time re-sends if a window rule moves it.
        if let Some(mon) = self.monitors.get(mon_idx) {
            crate::state::handlers::compositor::send_scale_transform(
                surface.wl_surface(),
                mon.output.current_scale(),
                mon.output.current_transform(),
            );
        }

        let ft_handle = self.protocols.foreign_toplevel_list.new_toplevel::<Self>(
            crate::utils::clamp_wire_str(&title),
            crate::utils::clamp_wire_str(&app_id),
        );
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);

        // Smart-insert (niri pattern): in scroller layout, place the new
        // client right after the focused one so closing it returns you near
        // your previous position; elsewhere `new_is_master` puts it first.
        let target_mon = client.monitor;
        let insert_at = self.new_client_position(target_mon);
        let new_idx = match insert_at {
            Some(pos) => {
                self.clients.insert(pos, client);
                self.shift_indices_at_or_after(pos);
                pos
            }
            None => {
                self.clients.push(client);
                self.clients.len() - 1
            }
        };

        tracing::info!(
            "new toplevel: app_id={:?} monitor={target_mon} idx={new_idx} \
             (map deferred until first commit)",
            if app_id.is_empty() {
                "<unset>"
            } else {
                &app_id
            },
        );
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        // Popup placement is a recurring source of "menu opens in the wrong
        // spot" bugs (esp. repeated/second opens). Log the raw client
        // positioner at debug so `mctl log level debug` reveals exactly what
        // the client asked for vs where we end up placing it (see the paired
        // log in `unconstrain_popup`).
        tracing::debug!(?positioner, geometry = ?positioner.get_geometry(), "new_popup: client positioner");
        surface.with_pending_state(|state| {
            state.positioner = positioner;
            state.geometry = positioner.get_geometry();
        });
        // Keep the popup inside its parent's output (flip/slide per the
        // positioner) so edge-of-screen menus don't spill off.
        self.unconstrain_popup(&surface);
        // Same buffer scale / transform as the window or layer it opens
        // from, before the popup's first buffer.
        if let Ok(root) =
            smithay::desktop::find_popup_root_surface(&PopupKind::Xdg(surface.clone()))
            && let Some(output) = super::compositor::output_for_root(self, &root)
        {
            super::compositor::send_scale_transform(
                surface.wl_surface(),
                output.current_scale(),
                output.current_transform(),
            );
        }
        match self.popups.track_popup(PopupKind::Xdg(surface)) {
            Ok(()) => tracing::info!("new_popup: tracked"),
            Err(e) => tracing::warn!(?e, "new_popup: track_popup failed"),
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        tracing::debug!(?positioner, token, geometry = ?positioner.get_geometry(), "reposition_request: client positioner");
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        if self.session_locked {
            return;
        }
        let Some(seat) = Seat::<MargoState>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };

        // Resolve the toplevel back to our MargoClient + Window so
        // the grab can manipulate float_geom directly.
        let wl_surf = surface.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            return;
        };
        let window = self.clients[idx].window.clone();
        let initial_loc =
            Point::<i32, Logical>::from((self.clients[idx].geom.x, self.clients[idx].geom.y));
        // CSD-initiated move via xdg_toplevel.move always starts
        // a regular drag — never the tile-to-tile swap path.
        let original_float_geom = self.clients[idx].float_geom;
        self.clients[idx].interactive_grab = true;

        let grab = crate::input::grabs::MoveSurfaceGrab {
            start_data,
            window,
            initial_loc,
            was_tiled: false,
            original_float_geom,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        if self.session_locked {
            return;
        }
        let Some(seat) = Seat::<MargoState>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };

        let wl_surf = surface.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            return;
        };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc = Point::<i32, Logical>::from((c.geom.x, c.geom.y));
        let initial_size = Size::<i32, Logical>::from((c.geom.width.max(1), c.geom.height.max(1)));
        self.clients[idx].interactive_grab = true;

        let grab = crate::input::grabs::ResizeSurfaceGrab {
            start_data,
            window,
            edges,
            initial_loc,
            initial_size,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        if self.session_locked {
            return;
        }
        tracing::info!("xdg_popup.grab fired serial={:?}", serial);
        // Proper xdg_popup.grab: set up smithay's PopupGrab so the
        // pointer + keyboard track the popup chain. Without the
        // pointer half, browser context menus / Helium's 3-dot menu
        // / Nemo's right-click menu open and INSTANTLY dismiss
        // because pointer events keep going to the toplevel — the
        // toplevel sees a click "outside" the popup it just opened
        // and tears the popup down (the visible "menu doesn't
        // open" symptom).
        //
        // The standard smithay pattern:
        //   1. Resolve the popup's root toplevel (walks
        //      xdg_surface.parent up to the root).
        //   2. Map that root wl_surface back to a `FocusTarget`
        //      (`FocusTarget::Window` for an xdg toplevel).
        //   3. `popups.grab_popup(root, kind, seat, serial)` —
        //      smithay validates the serial, ensures the popup is
        //      the topmost in the chain, sets up the bookkeeping.
        //   4. `keyboard.set_grab(PopupKeyboardGrab)` — keyboard
        //      stays on the popup chain; clicks outside dismiss.
        //   5. `pointer.set_grab(PopupPointerGrab)` — pointer
        //      events drill through the popup chain; pointer.button
        //      OUTSIDE the popup hierarchy dismisses the chain.
        let Some(seat) = Seat::<MargoState>::from_resource(&seat) else {
            return;
        };

        let kind = smithay::desktop::PopupKind::Xdg(surface);
        let Ok(root_wl_surface) = smithay::desktop::find_popup_root_surface(&kind) else {
            // Stale popup — parent already dismissed. Letting it
            // fall through to the helper would just produce a
            // noisier error.
            return;
        };

        // Resolve the root surface back to a FocusTarget. For xdg
        // toplevels it must be FocusTarget::Window. We search the
        // client list for a window whose wl_surface matches; if no
        // match (X11 client, or weird race), abort the grab silently.
        let root_focus = self
            .clients
            .iter()
            .find_map(|c| {
                c.window
                    .wl_surface()
                    .as_deref()
                    .filter(|s| **s == root_wl_surface)
                    .map(|_| FocusTarget::Window(c.window.clone()))
            })
            .or_else(|| {
                // The popup's root may be a layer surface (a bar / the dock),
                // not a toplevel. Resolve it so layer-shell popups (the dock
                // right-click menu) also get the keyboard grab — without this
                // the grab bailed out here and Esc never reached the menu.
                self.space.outputs().find_map(|o| {
                    let map = smithay::desktop::layer_map_for_output(o);
                    map.layer_for_surface(
                        &root_wl_surface,
                        smithay::desktop::WindowSurfaceType::TOPLEVEL,
                    )
                    .map(|l| FocusTarget::LayerSurface(l.layer_surface().clone()))
                })
            });
        let Some(root) = root_focus else {
            return;
        };

        let mut grab = match self.popups.grab_popup(root, kind, &seat, serial) {
            Ok(g) => g,
            Err(err) => {
                tracing::debug!(?err, "xdg_popup.grab rejected");
                return;
            }
        };

        if let Some(keyboard) = seat.get_keyboard() {
            // If somebody else (like a different popup chain) is
            // already grabbing the keyboard, only honour our request
            // if it chains directly off that grab — otherwise it'd
            // be a focus-steal disguised as a popup.
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                let _ = grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                return;
            }
            self.set_keyboard_focus(grab.current_grab(), serial);
            keyboard.set_grab(
                self,
                smithay::desktop::PopupKeyboardGrab::new(&grab),
                serial,
            );
        }

        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                let _ = grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(
                self,
                smithay::desktop::PopupPointerGrab::new(&grab),
                serial,
                smithay::input::pointer::Focus::Keep,
            );
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surf = surface.wl_surface().clone();
        tracing::info!("toplevel destroyed");
        match self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        {
            // Focus recall, close animation, arrange, the portal window
            // list and the `on_window_close` hook all happen in there —
            // the same path X11 windows take.
            Some(idx) => {
                self.remove_client(idx);
            }
            // Never mapped as a client; still refresh xdp-gnome's window
            // picker — see finalize_initial_map.
            None => self.emit_windows_changed(),
        }
    }

    // ── Fullscreen / maximize requests ──────────────────────────────────────
    //
    // smithay's defaults call `send_configure()` without flipping any
    // state, so the client never sees a configure with the Fullscreen
    // flag set — Helium / Firefox / mpv stay windowed even after
    // `xdg_toplevel.set_fullscreen()`. F11 in browsers (which
    // implement the JS Fullscreen API by calling set_fullscreen) hits
    // the same path. The dispatch-side `togglefullscreen` action
    // worked because we toggled `is_fullscreen` on our MargoClient
    // and let arrange_monitor give it a full-output rect, but
    // well-behaved clients keep rendering windowed chrome until the
    // configure event lands with the right state.

    fn fullscreen_request(&mut self, toplevel: ToplevelSurface, wl_output: Option<WlOutput>) {
        let wl_surf = toplevel.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            // Unmapped client (initial-map deferral pending) — fall
            // through to the default configure so the client doesn't
            // hang waiting.
            toplevel.send_configure();
            return;
        };

        // Optional output target: migrate the client to that monitor
        // before going fullscreen so Helium / browsers that pass the
        // meeting-screen output land on the right panel. Moved like
        // `tagmon` does: onto a tag the target shows (else it would be
        // fullscreen but invisible) and with the source re-arranged.
        if let Some(target_output) = wl_output
            && let Some(target_mon) = self
                .monitors
                .iter()
                .position(|m| m.enabled && m.output.owns(&target_output))
            && self.clients[idx].monitor != target_mon
        {
            let was_selected = self.monitors[self.clients[idx].monitor].selected == Some(idx);
            let target_tagset = self.monitors[target_mon].current_tagset();
            let tags = if self.clients[idx].tags & target_tagset != 0 {
                self.clients[idx].tags
            } else {
                target_tagset
            };
            self.move_client_to_monitor(idx, target_mon, tags);
            if was_selected {
                self.monitors[target_mon].selected = Some(idx);
            }
        }

        self.set_client_fullscreen(idx, true);
    }

    fn unfullscreen_request(&mut self, toplevel: ToplevelSurface) {
        let wl_surf = toplevel.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            toplevel.send_configure();
            return;
        };
        self.set_client_fullscreen(idx, false);
    }

    fn maximize_request(&mut self, toplevel: ToplevelSurface) {
        // margo doesn't have a separate maximized-vs-tiled state
        // (we're a tiling compositor — the layout drives sizing).
        // Treat maximize the same as fullscreen so apps like
        // gnome-calculator / pavucontrol get the screen-filling
        // behaviour they expect from `set_maximized`. Better than
        // smithay's default of "configure with no state change".
        let wl_surf = toplevel.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            toplevel.send_configure();
            return;
        };
        self.set_client_fullscreen(idx, true);
    }

    /// The app's own minimize (a CSD minimize button, `gtk_window_minimize`).
    /// xdg-shell has no unminimize request: the window comes back through
    /// `restore_*`, a taskbar or any activation.
    fn minimize_request(&mut self, toplevel: ToplevelSurface) {
        if let Some(idx) = self.client_idx_for_surface(toplevel.wl_surface()) {
            self.minimize_client(idx);
        }
    }

    fn unmaximize_request(&mut self, toplevel: ToplevelSurface) {
        let wl_surf = toplevel.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            toplevel.send_configure();
            return;
        };
        self.set_client_fullscreen(idx, false);
    }
}

impl MargoState {
    /// Re-position an xdg popup so it stays inside the output its parent sits
    /// on, applying the positioner's flip/slide/resize constraint adjustment.
    /// Without this, GTK / Chromium menus (right-click menus, dropdowns, the
    /// dock context menu) opened near a screen edge spilled off-screen because
    /// we honoured the client's raw requested geometry verbatim. The parent is
    /// either a mapped toplevel window or a layer surface (bars / the dock).
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        use smithay::desktop::{
            WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords,
            layer_map_for_output,
        };
        use smithay::utils::{Logical, Rectangle};

        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };

        // Parent root geometry in global coordinates.
        let root_geo: Rectangle<i32, Logical> = if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().map(|s| *s == root).unwrap_or(false))
        {
            match self.space.element_geometry(window) {
                Some(g) => g,
                None => return,
            }
        } else if let Some(geo) = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
            let lgeo = map.layer_geometry(layer)?;
            let ogeo = self.space.output_geometry(o)?;
            Some(Rectangle::new(
                (ogeo.loc.x + lgeo.loc.x, ogeo.loc.y + lgeo.loc.y).into(),
                lgeo.size,
            ))
        }) {
            geo
        } else {
            return;
        };

        // The output that contains the parent.
        let Some(output_geo) = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .find(|g| g.overlaps(root_geo))
        else {
            return;
        };

        // Express the available output area in the popup's parent-local frame,
        // then let smithay clamp the geometry to it.
        let mut target = output_geo;
        target.loc -= root_geo.loc;
        let toplevel_coords = get_popup_toplevel_coords(&kind);
        target.loc -= toplevel_coords;

        let final_geo = popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
            state.geometry
        });
        // Paired with the `new_popup` / `reposition_request` logs: shows the
        // parent root + output geometry and the final placed geometry, so a
        // drifting second popup can be pinned to either a stale parent
        // `root_geo` / `toplevel_coords` (compositor) or a changed client
        // positioner (client).
        tracing::debug!(
            ?root_geo,
            ?output_geo,
            ?toplevel_coords,
            ?target,
            ?final_geo,
            "unconstrain_popup: placed"
        );
    }
}
