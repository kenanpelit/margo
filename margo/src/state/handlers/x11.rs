//! XWayland integration handlers.
//!
//! Two related impls:
//!
//! * [`XWaylandShellHandler`] is the wayland-side anchor smithay needs
//!   to track which `wl_surface` belongs to which X11 window.
//! * [`XwmHandler`] is the X11-side window-manager loop — every map
//!   / unmap / configure request on an X11 client routes through it.
//!
//! Selection bridging lives here too because XWayland's `xwm` is the
//! canonical place to mirror clipboard / primary-selection state
//! into the X11 selection model.

use std::os::unix::io::OwnedFd;

use smithay::{
    desktop::{Window, WindowSurface},
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge, X11Window, XwmId},
    },
};

use crate::state::{FocusTarget, MargoState};

impl XWaylandShellHandler for MargoState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XwmHandler for MargoState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("X11Wm not initialized")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).ok();
        self.register_x11_window(window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let id = window.window_id();
        let win = Window::new_x11_window(window);
        let stale = win.x11_surface().map(|s| s.geometry());
        // The toolkit usually moves the popup to its anchor via a
        // ConfigureNotify that arrives BEFORE this map event, when the surface
        // isn't in the space yet — so we stashed that location in
        // `or_positions`. Prefer it; `X11Surface::geometry()` still reads the
        // stale (0,0) creation rect here, which would dump the menu in the
        // top-left corner.
        //
        // Read with `get`, NOT `remove`: Qt REUSES one X11 window for a menu
        // and, on a repeated open at the same spot, re-maps it WITHOUT sending
        // a fresh ConfigureNotify (nothing moved from its POV). If we consumed
        // the stash on the first map, that second open would find no entry,
        // fall back to the stale (0,0) geometry, and drift to the top-left
        // corner — the exact "second open lands wrong" symptom. Keeping the
        // entry lets a re-used, un-reconfigured window remember its last anchor;
        // a real move still overwrites it via `configure_notify`, and
        // `destroyed_window` clears it for good.
        let pos = self
            .or_positions
            .get(&id)
            .copied()
            .or_else(|| stale.map(|g| (g.loc.x, g.loc.y)))
            .unwrap_or((0, 0));
        tracing::debug!(
            "xwm OR mapped: id={} stale_geom={:?} -> space pos {:?}",
            id,
            stale,
            pos
        );
        self.space.map_element(win, pos, false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Deliberately do NOT drop the `or_positions` entry here: an unmap is
        // often just a menu being hidden, and Qt re-maps the SAME window for
        // the next open without re-anchoring it. Keeping the stashed position
        // is what lets that re-open land at its last spot instead of (0,0).
        // The entry is cleared for real in `destroyed_window`.
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.or_positions.remove(&window.window_id());
        self.remove_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let geom = window.geometry();
        let new_geom = Rectangle::new(
            (x.unwrap_or(geom.loc.x), y.unwrap_or(geom.loc.y)).into(),
            (
                w.map(|v| v as i32).unwrap_or(geom.size.w),
                h.map(|v| v as i32).unwrap_or(geom.size.h),
            )
                .into(),
        );
        window.configure(new_geom).ok();
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if let Some(idx) = self.find_x11_client(&window) {
            tracing::debug!("xwm managed configure_notify: geometry={:?}", geometry);
            self.clients[idx].geom = crate::layout::Rect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            };
            return;
        }
        tracing::debug!("xwm OR configure_notify: geometry={:?}", geometry);
        // Remember the anchor even if the surface isn't mapped into the space
        // yet (Qt/GTK configure the popup to its anchor BEFORE the map event);
        // `mapped_override_redirect_window` consumes this.
        self.or_positions
            .insert(window.window_id(), (geometry.loc.x, geometry.loc.y));
        // Override-redirect surface (menu / popup / tooltip). It is never
        // registered in `self.clients` — it lives only in the space and
        // positions itself. `mapped_override_redirect_window` placed it once
        // at its initial geometry; we must also follow LATER moves here, or
        // the surface freezes at its first location. Qt5/GTK X11 popups
        // routinely map first and then move to their anchor, so without this
        // re-map menus open detached / in the wrong place under XWayland
        // (the symptom that disappears under a single-output nested
        // compositor). Mirrors Smithay anvil's override-redirect handling.
        let id = window.window_id();
        let elem = self
            .space
            .elements()
            .find(
                |e| matches!(e.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id),
            )
            .cloned();
        if let Some(elem) = elem {
            self.space.map_element(elem, geometry.loc, false);
            self.request_repaint();
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let Some(FocusTarget::Window(window)) = keyboard.current_focus() else {
            return false;
        };
        window
            .x11_surface()
            .and_then(|surface| surface.xwm_id())
            .map(|focused_xwm| focused_xwm == xwm)
            .unwrap_or(false)
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    tracing::error!(?err, "failed to request Wayland clipboard for XWayland");
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    tracing::error!(
                        ?err,
                        "failed to request Wayland primary selection for XWayland"
                    );
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }
}
