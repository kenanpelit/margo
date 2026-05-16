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

use smithay::{
    backend::renderer::element::Id as RenderElementId,
    delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::Seat,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::{wl_output::WlOutput, wl_seat::WlSeat},
    },
    utils::{Logical, Point, Serial, Size},
    wayland::{
        seat::WaylandFocus,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
    },
};

use crate::state::{ClosingClient, FocusTarget, MargoClient, MargoState};

impl XdgShellHandler for MargoState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
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

        let ft_handle = self
            .foreign_toplevel_list
            .new_toplevel::<Self>(&title, &app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);

        // Smart-insert (niri pattern): in scroller layout, place the new
        // client right after the focused one so closing it returns you near
        // your previous position. Other layouts are order-agnostic.
        let target_mon = client.monitor;
        let insert_at = self.scroller_insert_position(target_mon);
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
            if app_id.is_empty() { "<unset>" } else { &app_id },
        );
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
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
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
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
        let initial_loc = Point::<i32, Logical>::from((
            self.clients[idx].geom.x,
            self.clients[idx].geom.y,
        ));
        // CSD-initiated move via xdg_toplevel.move always starts
        // a regular drag — never the tile-to-tile swap path.
        let original_float_geom = self.clients[idx].float_geom;

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
        let root_focus = self.clients.iter().find_map(|c| {
            c.window
                .wl_surface()
                .as_deref()
                .filter(|s| **s == root_wl_surface)
                .map(|_| FocusTarget::Window(c.window.clone()))
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
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, smithay::desktop::PopupKeyboardGrab::new(&grab), serial);
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
        // Capture identity BEFORE any removal so the on_window_close
        // hook can fire with (app_id, title) — by the time the hook
        // runs, the MargoClient is gone.
        let mut closed_identity: Option<(String, String)> = None;
        if let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        {
            closed_identity = Some((
                self.clients[idx].app_id.clone(),
                self.clients[idx].title.clone(),
            ));
            if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
                handle.send_closed();
            }
            // Enqueue a close animation entry BEFORE removing the
            // client. The renderer captures the wl_surface to a
            // texture on its very next frame (the surface is still
            // alive — Wayland clients destroy their xdg_toplevel role
            // first, then their wl_surface), and from then on draws
            // the texture scaled+faded out around the slot's centre.
            //
            // We still unmap and remove from `clients` immediately so
            // every other state machine (focus stack, layout, scene
            // ordering) treats the close as having happened. The
            // closing entry lives in `closing_clients` purely as a
            // render-side concern.
            if self.config.animations
                && self.config.animation_duration_close > 0
                && !self.clients[idx].no_animation
            {
                let kind_str = self.clients[idx]
                    .animation_type_close
                    .clone()
                    .unwrap_or_else(|| self.config.animation_type_close.clone());
                let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
                let now = crate::utils::now_ms();
                let c = &self.clients[idx];
                self.closing_clients.push(ClosingClient {
                    id: RenderElementId::new(),
                    texture: None,
                    capture_pending: true,
                    geom: c.geom,
                    monitor: c.monitor,
                    tags: c.tags,
                    time_started: now,
                    duration: self.config.animation_duration_close,
                    progress: 0.0,
                    kind,
                    extreme_scale: self.config.zoom_end_ratio.clamp(0.05, 1.0),
                    border_radius: self.config.border_radius as f32,
                    source_surface: Some(wl_surf.clone()),
                });
                self.request_repaint();
            }
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.shift_indices_after_remove(idx);
            // Re-focus, preferring the previous focus (niri-style
            // focus stack recall), falling back to the spatially
            // nearest visible window.
            let mon_idx = self.focused_monitor();
            if mon_idx < self.monitors.len() {
                let tagset = self.monitors[mon_idx].current_tagset();
                let prev = self.monitors[mon_idx].prev_selected;
                let target = prev
                    .filter(|&i| {
                        i < self.clients.len()
                            && self.clients[i].is_visible_on(mon_idx, tagset)
                    })
                    .or_else(|| {
                        // Spatial fallback: window whose geom is
                        // closest (in vec order) to the removed slot.
                        (0..self.clients.len()).rev().find(|&i| {
                            i < idx && self.clients[i].is_visible_on(mon_idx, tagset)
                        })
                    })
                    .or_else(|| {
                        self.clients
                            .iter()
                            .position(|c| c.is_visible_on(mon_idx, tagset))
                    });
                match target {
                    Some(i) => {
                        let w = self.clients[i].window.clone();
                        self.monitors[mon_idx].selected = Some(i);
                        self.focus_surface(Some(FocusTarget::Window(w)));
                    }
                    None => {
                        self.monitors[mon_idx].selected = None;
                        self.focus_surface(None);
                    }
                }
            }
            // Re-arrange so the scroller centers the new focus immediately.
            if mon_idx < self.monitors.len() {
                self.arrange_monitor(mon_idx);
            }
        }
        tracing::info!("toplevel destroyed");
        // Refresh xdp-gnome's window picker — see finalize_initial_map.
        self.emit_windows_changed();
        // Fire user scripting hook AFTER state is consistent
        // (client removed, focus shifted, arrange done) so handlers
        // observing `client_count()` / `focused_appid()` see the
        // post-close world.
        if let Some((app_id, title)) = closed_identity {
            crate::scripting::fire_window_close(self, &app_id, &title);
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

    fn fullscreen_request(
        &mut self,
        toplevel: ToplevelSurface,
        wl_output: Option<WlOutput>,
    ) {
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
        // meeting-screen output land on the right panel.
        if let Some(target_output) = wl_output {
            if let Some(target_mon) = self
                .monitors
                .iter()
                .position(|m| m.output.owns(&target_output))
            {
                if self.clients[idx].monitor != target_mon {
                    self.clients[idx].monitor = target_mon;
                }
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
delegate_xdg_shell!(MargoState);
