//! `zwp_pointer_gestures_v1` delegate.
//!
//! Smithay's `PointerGesturesState` only needs `SeatHandler` from the
//! compositor — there is no per-protocol handler trait, just the
//! delegate macro to wire dispatch + global-dispatch onto `MargoState`.
//! `input_handler.rs` forwards libinput's pinch and hold gestures to the
//! surface under the pointer, and swipes whose finger count no
//! `gesturebind` uses (`gesture_swipe_begin` & co.); nothing while locked.
