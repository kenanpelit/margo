//! `xdg_system_bell_v1` handler.
//!
//! Clients ring the system bell. For now we just log; routing to
//! a sound daemon / mshell notification toast is a future
//! enhancement.

use smithay::{
    delegate_xdg_system_bell,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::xdg_system_bell::XdgSystemBellHandler,
};

use crate::state::MargoState;

impl XdgSystemBellHandler for MargoState {
    fn ring(&mut self, surface: Option<WlSurface>) {
        tracing::debug!(?surface, "xdg_system_bell: ring");
    }
}
delegate_xdg_system_bell!(MargoState);
