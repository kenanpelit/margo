//! `wp_pointer_warp_v1` handler.
//!
//! Clients can request that the compositor move the cursor to a
//! surface-local position. Default trait body is a no-op, which is
//! a sensible policy: programmatic cursor warping is opt-in. A
//! future policy could honor warps from focused surfaces with the
//! correct enter serial.

use smithay::{delegate_pointer_warp, wayland::pointer_warp::PointerWarpHandler};

use crate::state::MargoState;

impl PointerWarpHandler for MargoState {}
delegate_pointer_warp!(MargoState);
