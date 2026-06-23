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
        let win = Window::new_x11_window(window);
        let geo = win.x11_surface().map(|s| s.geometry());
        let pos = geo.map(|g| (g.loc.x, g.loc.y)).unwrap_or((0, 0));
        tracing::debug!(
            "xwm OR mapped: x11_geometry={:?} -> space pos {:?} | outputs={:?}",
            geo,
            pos,
            self.monitors
                .iter()
                .map(|m| (
                    m.name.clone(),
                    (
                        m.monitor_area.x,
                        m.monitor_area.y,
                        m.monitor_area.width,
                        m.monitor_area.height
                    )
                ))
                .collect::<Vec<_>>(),
        );
        self.space.map_element(win, pos, false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
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
