//! `zwp_xwayland_keyboard_grab_v1` handler.
//!
//! XWayland clients use this to request an exclusive keyboard grab
//! through the compositor (the X11-side mechanism for the same
//! VNC / VM / remote-desktop story that `keyboard_shortcuts_inhibit_v1`
//! covers Wayland-side). Smithay's default `grab()` impl already
//! installs the grab on the seat's keyboard; the one thing margo
//! must supply is the surface → FocusTarget mapping so the grab
//! can target the matching X11 toplevel.

use smithay::{
    delegate_xwayland_keyboard_grab,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{seat::WaylandFocus, xwayland_keyboard_grab::XWaylandKeyboardGrabHandler},
};

use crate::state::{FocusTarget, MargoState};

impl XWaylandKeyboardGrabHandler for MargoState {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<FocusTarget> {
        // Look up the margo client whose Window carries this
        // wl_surface (XWayland gives X11 surfaces a backing
        // wl_surface). Return its Window as a FocusTarget so the
        // grab gets attached to the X11-backed toplevel.
        self.clients
            .iter()
            .find(|c| c.window.wl_surface().as_deref() == Some(surface))
            .map(|c| FocusTarget::Window(c.window.clone()))
    }
}
delegate_xwayland_keyboard_grab!(MargoState);
