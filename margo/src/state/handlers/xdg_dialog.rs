//! `xdg_wm_dialog_v1` handler.
//!
//! Clients tag toplevels as dialogs so the compositor can place /
//! decorate them differently from regular toplevels. Smithay's
//! `XdgDialogHandler` trait has a single optional callback for
//! dialog-hint changes; margo doesn't currently special-case
//! dialogs (they go through the same map / arrange path as any
//! other xdg toplevel), so the default no-op is fine.

use smithay::{delegate_xdg_dialog, wayland::shell::xdg::dialog::XdgDialogHandler};

use crate::state::MargoState;

impl XdgDialogHandler for MargoState {}
delegate_xdg_dialog!(MargoState);
