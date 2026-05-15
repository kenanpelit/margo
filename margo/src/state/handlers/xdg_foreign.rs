//! `xdg_foreign_v2` handler.
//!
//! Cross-process surface embedding: a client exports its surface,
//! shares the handle with another client (typically over D-Bus or
//! command-line argument), which then imports it to use as a parent.
//! Used by Firefox / Chromium Picture-in-Picture and by
//! xdg-desktop-portal screencast for target-window selection.
//!
//! Smithay's `XdgForeignState` handles all the protocol-level
//! bookkeeping; the handler trait is a single-method state accessor.

use smithay::{
    delegate_xdg_foreign,
    wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState},
};

use crate::state::MargoState;

impl XdgForeignHandler for MargoState {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}
delegate_xdg_foreign!(MargoState);
