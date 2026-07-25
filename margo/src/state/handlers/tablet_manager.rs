//! `zwp_tablet_manager_v2` delegate + `TabletSeatHandler` impl.
//!
//! Smithay drives the actual tablet protocol surface from
//! `SeatHandler`; the per-protocol handler is just one optional
//! callback for clients that ask for a custom tool cursor image.
//! margo follows the cursor-shape protocol for now, so the callback
//! is a default no-op.

// Moved from `smithay::wayland::tablet_manager` to `smithay::input::tablet`
// in the 2026-07-25 smithay bump (the wayland-side re-export went private).
use smithay::input::tablet::TabletSeatHandler;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::state::MargoState;

impl TabletSeatHandler for MargoState {
    // Tablet-tool focus surface, newly required by the 2026-07-25 smithay
    // bump. smithay's own doc example uses WlSurface; margo's handler is
    // otherwise a no-op (the cursor-shape protocol drives the tool cursor),
    // so the plain surface type is all that's needed.
    type ToolFocus = WlSurface;
}
