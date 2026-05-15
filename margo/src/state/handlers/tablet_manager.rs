//! `zwp_tablet_manager_v2` delegate + `TabletSeatHandler` impl.
//!
//! Smithay drives the actual tablet protocol surface from
//! `SeatHandler`; the per-protocol handler is just one optional
//! callback for clients that ask for a custom tool cursor image.
//! margo follows the cursor-shape protocol for now, so the callback
//! is a default no-op.

use smithay::{
    delegate_tablet_manager,
    wayland::tablet_manager::TabletSeatHandler,
};

use crate::state::MargoState;

impl TabletSeatHandler for MargoState {}
delegate_tablet_manager!(MargoState);
