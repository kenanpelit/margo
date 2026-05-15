//! `xdg_toplevel_tag_v1` handler.
//!
//! Clients attach semantic tags + description strings to their
//! toplevels (e.g. "browser-window", "settings-dialog"). Margo
//! could feed these into window-rule matching in the future; for
//! now the trait's default no-ops are kept.

use smithay::{
    delegate_xdg_toplevel_tag,
    wayland::xdg_toplevel_tag::XdgToplevelTagHandler,
};

use crate::state::MargoState;

impl XdgToplevelTagHandler for MargoState {}
delegate_xdg_toplevel_tag!(MargoState);
