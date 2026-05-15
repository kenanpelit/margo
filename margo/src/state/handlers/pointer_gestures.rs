//! `zwp_pointer_gestures_v1` delegate.
//!
//! Smithay's `PointerGesturesState` only needs `SeatHandler` from the
//! compositor — there is no per-protocol handler trait, just the
//! delegate macro to wire dispatch + global-dispatch onto `MargoState`.
//! libinput-side gesture events are already plumbed through margo's
//! existing `input_handler.rs` swipe / pinch / hold paths; the
//! protocol global just lets clients subscribe to them.

use smithay::delegate_pointer_gestures;

use crate::state::MargoState;

delegate_pointer_gestures!(MargoState);
