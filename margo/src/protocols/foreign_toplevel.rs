//! ext-foreign-toplevel-list-v1 server state.
//!
//! Exposes the toplevel window list to taskbar clients (e.g. Waybar's
//! `foreign-toplevel` module).  Uses smithay's built-in
//! `ForeignToplevelListState` which implements the
//! `ext-foreign-toplevel-list-v1` protocol.
//!
//! Lifecycle:
//!   - `new_toplevel()`  → called from `XdgShellHandler::new_toplevel` / X11 map
//!   - `ForeignToplevelHandle::send_title/send_app_id/send_done` → title changes
//!   - `ForeignToplevelHandle::send_closed()` → window destroyed

pub use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};
