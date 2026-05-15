//! `xdg_toplevel_icon_v1` handler.
//!
//! Toplevels ship their own icon (PNG / SVG buffer or icon name).
//! Default no-op accepts the icon — smithay caches it on the
//! surface as `ToplevelIconCachedState`. A future mshell taskbar
//! / active-window pill consumer can pull it from there.

use smithay::{
    delegate_xdg_toplevel_icon,
    wayland::xdg_toplevel_icon::XdgToplevelIconHandler,
};

use crate::state::MargoState;

impl XdgToplevelIconHandler for MargoState {}
delegate_xdg_toplevel_icon!(MargoState);
