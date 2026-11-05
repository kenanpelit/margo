//! Touchpad swipe / pinch / hold gestures and switch-free gesture binds:
//! begin/update/end tracking, the motion-code classifier and the swipe
//! dispatcher shared with touchscreen gestures.

use smithay::{backend::input::InputTime, utils::SERIAL_COUNTER};
use tracing::{debug, info};

use super::keyboard::smithay_mods_to_margo;
use crate::state::MargoState;

// ── Touchpad swipe gestures ──────────────────────────────────────────────────
//
// We accumulate dx/dy across the gesture, then on End we resolve a cardinal
// direction (or diagonal) and look up `gesturebind` from the config. This
// mirrors how mango (C) and niri handle touchpad swipes.

/// Hardcoded fallback when `Config::swipe_min_threshold` isn't set (or is
/// set to its default of 1, which is too aggressive for natural use).
/// Mango's C version uses ~30 px and that feels right; the config knob
/// gives users with very-short-throw swipes a way to lower it.
const SWIPE_MIN_DISTANCE_DEFAULT: f64 = 30.0;

/// A touchpad swipe with `fingers` belongs to margo when any `gesturebind`
/// uses that finger count; otherwise it goes to the client (browser
/// back/forward, GTK swipes). Decided at begin: the direction a bind
/// matches on is only known at the end.
fn swipe_is_bound(config: &margo_config::Config, fingers: u32) -> bool {
    config.gesture_bindings.iter().any(|b| b.fingers == fingers)
}

/// The pointer that gesture events go to, unless the session is locked.
fn gesture_pointer(
    state: &MargoState,
) -> Option<smithay::input::pointer::PointerHandle<MargoState>> {
    if state.session_locked {
        return None;
    }
    state.seat.get_pointer()
}

pub(crate) fn gesture_swipe_begin(state: &mut MargoState, fingers: u32, time_ms: u32) {
    use smithay::input::pointer::GestureSwipeBeginEvent;
    state.input_gesture = Default::default();
    state.input_gesture.fingers = fingers;
    if swipe_is_bound(&state.config, fingers) {
        state.input_gesture.swipe_active = true;
        return;
    }
    let Some(pointer) = gesture_pointer(state) else {
        return;
    };
    state.input_gesture.swipe_forwarded = true;
    pointer.gesture_swipe_begin(
        state,
        &GestureSwipeBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time_ms),
            fingers,
        },
    );
}

pub(crate) fn gesture_swipe_update(state: &mut MargoState, time_ms: u32, delta: (f64, f64)) {
    use smithay::input::pointer::GestureSwipeUpdateEvent;
    if state.input_gesture.swipe_active {
        state.input_gesture.dx += delta.0;
        state.input_gesture.dy += delta.1;
    } else if state.input_gesture.swipe_forwarded
        && let Some(pointer) = state.seat.get_pointer()
    {
        pointer.gesture_swipe_update(
            state,
            &GestureSwipeUpdateEvent {
                time: InputTime::from_millis(time_ms),
                delta: delta.into(),
            },
        );
    }
}

pub(crate) fn gesture_swipe_end(state: &mut MargoState, time_ms: u32, cancelled: bool) {
    use smithay::input::pointer::GestureSwipeEndEvent;
    let g = std::mem::take(&mut state.input_gesture);
    if g.swipe_active {
        if !cancelled {
            dispatch_swipe(state, g.dx, g.dy, g.fingers, "touchpad");
        }
    } else if g.swipe_forwarded
        && let Some(pointer) = state.seat.get_pointer()
    {
        pointer.gesture_swipe_end(
            state,
            &GestureSwipeEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: InputTime::from_millis(time_ms),
                cancelled,
            },
        );
    }
}

pub(crate) fn gesture_pinch_begin(state: &mut MargoState, fingers: u32, time_ms: u32) {
    use smithay::input::pointer::GesturePinchBeginEvent;
    let Some(pointer) = gesture_pointer(state) else {
        return;
    };
    state.input_gesture.pinch_forwarded = true;
    pointer.gesture_pinch_begin(
        state,
        &GesturePinchBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time_ms),
            fingers,
        },
    );
}

pub(crate) fn gesture_pinch_update(
    state: &mut MargoState,
    time_ms: u32,
    delta: (f64, f64),
    scale: f64,
    rotation: f64,
) {
    use smithay::input::pointer::GesturePinchUpdateEvent;
    if !state.input_gesture.pinch_forwarded {
        return;
    }
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    pointer.gesture_pinch_update(
        state,
        &GesturePinchUpdateEvent {
            time: InputTime::from_millis(time_ms),
            delta: delta.into(),
            scale,
            rotation,
        },
    );
}

pub(crate) fn gesture_pinch_end(state: &mut MargoState, time_ms: u32, cancelled: bool) {
    use smithay::input::pointer::GesturePinchEndEvent;
    if !std::mem::take(&mut state.input_gesture.pinch_forwarded) {
        return;
    }
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    pointer.gesture_pinch_end(
        state,
        &GesturePinchEndEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time_ms),
            cancelled,
        },
    );
}

pub(crate) fn gesture_hold_begin(state: &mut MargoState, fingers: u32, time_ms: u32) {
    use smithay::input::pointer::GestureHoldBeginEvent;
    let Some(pointer) = gesture_pointer(state) else {
        return;
    };
    state.input_gesture.hold_forwarded = true;
    pointer.gesture_hold_begin(
        state,
        &GestureHoldBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time_ms),
            fingers,
        },
    );
}

pub(crate) fn gesture_hold_end(state: &mut MargoState, time_ms: u32, cancelled: bool) {
    use smithay::input::pointer::GestureHoldEndEvent;
    if !std::mem::take(&mut state.input_gesture.hold_forwarded) {
        return;
    }
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    pointer.gesture_hold_end(
        state,
        &GestureHoldEndEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time_ms),
            cancelled,
        },
    );
}

/// Map a 2D displacement into the 0..=7 motion code the gesture
/// binding table uses (matches `margo-config::parse_motion()`):
/// UP=0, DOWN=1, RIGHT=2, LEFT=3, UP_RIGHT=4, UP_LEFT=5, DOWN_LEFT=6,
/// DOWN_RIGHT=7. Diagonal threshold: each axis must contribute > 40 %
/// of the total magnitude.
fn derive_motion_code(dx: f64, dy: f64) -> u32 {
    let total = (dx * dx + dy * dy).sqrt();
    let ax = dx.abs();
    let ay = dy.abs();
    let diag = ax > 0.4 * total && ay > 0.4 * total;
    if diag {
        match (dx.is_sign_positive(), dy.is_sign_positive()) {
            (true, false) => 4,  // UP_RIGHT
            (false, false) => 5, // UP_LEFT
            (false, true) => 6,  // DOWN_LEFT
            (true, true) => 7,   // DOWN_RIGHT
        }
    } else if ax > ay {
        if dx > 0.0 { 2 } else { 3 } // RIGHT / LEFT
    } else if dy < 0.0 {
        0 // UP
    } else {
        1 // DOWN
    }
}

/// Common dispatch path used by **both** the touchpad swipe gesture
/// (libinput `GestureSwipeEnd`) and the touchscreen multi-finger
/// swipe (`InputEvent::TouchUp` after ≥ 2 fingers had touched). Looks
/// up `(fingers, motion, mods)` in `Config::gesture_bindings` and
/// fires the matched action.
///
/// `source` is just a log tag so the trace clearly says which input
/// path matched — handy when binding-debugging on a 2-in-1 with both
/// a touchpad and a touchscreen.
pub(crate) fn dispatch_swipe(
    state: &mut MargoState,
    dx: f64,
    dy: f64,
    fingers: u32,
    source: &'static str,
) {
    if state.session_locked {
        return;
    }
    let total = (dx * dx + dy * dy).sqrt();
    let cfg_threshold = state.config.swipe_min_threshold as f64;
    let threshold = if cfg_threshold > 1.0 {
        cfg_threshold
    } else {
        SWIPE_MIN_DISTANCE_DEFAULT
    };
    if total < threshold {
        debug!(
            "{source} swipe ignored (too short): total={:.1} threshold={:.1} fingers={}",
            total, threshold, fingers
        );
        return;
    }
    let motion = derive_motion_code(dx, dy);

    let mods = state
        .seat
        .get_keyboard()
        .map(|k| smithay_mods_to_margo(&k.modifier_state()))
        .unwrap_or_else(margo_config::Modifiers::empty);

    let binding = state
        .config
        .gesture_bindings
        .iter()
        .find(|b| b.fingers == fingers && b.motion == motion && b.modifiers == mods);

    if let Some(binding) = binding {
        info!(
            source = source,
            fingers = fingers,
            motion = motion,
            mods = ?mods,
            action = ?binding.action,
            "swipe match",
        );
        let action = binding.action.clone();
        let arg = binding.arg.clone();
        crate::dispatch::dispatch_action(state, &action, &arg);
        state.request_repaint();
    } else {
        debug!(
            "{source} swipe unmatched: fingers={} motion={} mods={:?}",
            fingers, motion, mods
        );
    }
}

#[cfg(test)]
mod gesture_tests {
    use super::derive_motion_code;

    #[test]
    fn cardinal_directions_map_to_codes_0_through_3() {
        assert_eq!(derive_motion_code(0.0, -100.0), 0); // UP
        assert_eq!(derive_motion_code(0.0, 100.0), 1); // DOWN
        assert_eq!(derive_motion_code(100.0, 0.0), 2); // RIGHT
        assert_eq!(derive_motion_code(-100.0, 0.0), 3); // LEFT
    }

    #[test]
    fn balanced_diagonals_map_to_codes_4_through_7() {
        assert_eq!(derive_motion_code(70.0, -70.0), 4); // UP_RIGHT
        assert_eq!(derive_motion_code(-70.0, -70.0), 5); // UP_LEFT
        assert_eq!(derive_motion_code(-70.0, 70.0), 6); // DOWN_LEFT
        assert_eq!(derive_motion_code(70.0, 70.0), 7); // DOWN_RIGHT
    }

    #[test]
    fn small_off_axis_component_is_not_diagonal() {
        // 80 px right, 20 px down — secondary axis at 25 % of magnitude,
        // below the 40 % threshold. Should resolve to a pure direction.
        assert_eq!(derive_motion_code(80.0, 20.0), 2); // RIGHT, not DOWN_RIGHT
        assert_eq!(derive_motion_code(20.0, -80.0), 0); // UP, not UP_RIGHT
    }

    #[test]
    fn zero_displacement_falls_through_to_down() {
        // Defensive: an exact-zero swipe shouldn't crash. The current
        // code path lands on DOWN (1) — we just lock in that the
        // function is total over (0, 0).
        assert_eq!(derive_motion_code(0.0, 0.0), 1);
    }
}
