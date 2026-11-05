//! Touchscreen input (a true touch panel or 2-in-1; touchpad gestures
//! arrive as `InputEvent::Gesture*` instead).
//!
//! Every finger is delivered to the client under it over `wl_touch` (the
//! seat advertises the touch capability): `down` picks the surface under
//! the point (drilled through subsurfaces and popups, or the lock surface
//! while locked) and smithay keeps sending that finger's motion / up to
//! it. A first finger also gives keyboard focus to the window it lands on,
//! like a click.
//!
//! The same fingers feed the multi-finger swipe recogniser behind
//! `gesturebind`: once as many fingers are down as some `gesturebind` uses,
//! margo claims the sequence — the client gets `wl_touch.cancel` and
//! nothing more until every finger is up — and the swipe is resolved when
//! the last finger lifts. With no matching bind, multi-finger touches
//! (pinch-zoom in a browser) stay the client's.

use smithay::backend::input::{
    AbsolutePositionEvent, InputBackend, InputTime, TouchDownEvent, TouchMotionEvent, TouchSlot,
    TouchUpEvent,
};
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Transform};
use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;

use crate::input::{TouchPoint, TouchRelease};
use crate::state::{FocusTarget, MargoState};

/// Connector-name prefixes of built-in panels, where a touchscreen sits.
const INTERNAL_PANEL_PREFIXES: &[&str] = &["eDP", "LVDS", "DSI"];

/// Which output (index into `names`) a touchscreen maps to: the built-in
/// panel when there is one, else the first output.
pub fn pick_touch_output<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<usize> {
    let names: Vec<&str> = names.into_iter().collect();
    names
        .iter()
        .position(|n| INTERNAL_PANEL_PREFIXES.iter().any(|p| n.starts_with(p)))
        .or((!names.is_empty()).then_some(0))
}

/// A touch at normalised device coordinates `norm` (0..1 on each axis, in
/// the panel's own orientation) as a point in global logical space, for an
/// output at `geo` (logical, after its transform) shown with `transform`.
pub fn touch_point_in_output(
    norm: (f64, f64),
    geo: Rectangle<i32, Logical>,
    transform: Transform,
) -> Point<f64, Logical> {
    let size = transform.invert().transform_size(geo.size).to_f64();
    let on_panel: Point<f64, Logical> = (norm.0 * size.w, norm.1 * size.h).into();
    transform.transform_point_in(on_panel, &size) + geo.loc.to_f64()
}

/// Whether margo takes a touch sequence for a `gesturebind` once
/// `fingers_down` fingers are on the screen: some bind uses exactly that
/// many fingers. One finger is always the client's.
pub fn gesture_claims_touch(fingers_down: usize, bound_fingers: &[u32]) -> bool {
    fingers_down >= 2 && bound_fingers.iter().any(|&f| f as usize == fingers_down)
}

/// Where `event` touches, in global logical space.
fn touch_position<B: InputBackend>(
    state: &MargoState,
    event: &impl AbsolutePositionEvent<B>,
) -> Option<Point<f64, Logical>> {
    let outputs: Vec<_> = state.space.outputs().cloned().collect();
    let names: Vec<String> = outputs.iter().map(|o| o.name()).collect();
    let output = outputs.get(pick_touch_output(names.iter().map(String::as_str))?)?;
    let geo = state.space.output_geometry(output)?;
    let norm = (event.x_transformed(1), event.y_transformed(1));
    Some(touch_point_in_output(norm, geo, output.current_transform()))
}

pub fn handle_touch_down<B: InputBackend, E: TouchDownEvent<B>>(state: &mut MargoState, event: E) {
    let Some(pos) = touch_position(state, &event) else {
        return;
    };
    touch_down_at(state, event.slot(), pos, event.time());
}

pub fn handle_touch_motion<B: InputBackend, E: TouchMotionEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let Some(pos) = touch_position(state, &event) else {
        return;
    };
    touch_motion_at(state, event.slot(), pos, event.time());
}

pub fn handle_touch_up<B: InputBackend, E: TouchUpEvent<B>>(state: &mut MargoState, event: E) {
    touch_up_at(state, event.slot(), event.time());
}

/// Whether touches go to clients right now. The overviews show scaled
/// thumbnails, so a surface-local position would be wrong there.
fn delivers_to_clients(state: &MargoState) -> bool {
    !state.input_touch.claimed && !state.is_overview_open() && !state.is_scroller_overview_open()
}

/// A finger touched down at `pos` (global logical).
pub(crate) fn touch_down_at(
    state: &mut MargoState,
    slot: TouchSlot,
    pos: Point<f64, Logical>,
    time: InputTime,
) {
    let id: i32 = slot.into();
    state.input_touch.points.push(TouchPoint {
        id,
        x: pos.x,
        y: pos.y,
        start_x: pos.x,
        start_y: pos.y,
        start_time: time.millis(),
    });
    let fingers = state.input_touch.points.len();
    if fingers >= 2 {
        state.input_touch.gesture_armed = true;
    }

    let bound: Vec<u32> = state
        .config
        .gesture_bindings
        .iter()
        .map(|b| b.fingers)
        .collect();
    if !state.input_touch.claimed && !state.session_locked && gesture_claims_touch(fingers, &bound)
    {
        state.input_touch.claimed = true;
        if let Some(touch) = state.seat.get_touch() {
            touch.cancel(state);
        }
        return;
    }
    if !delivers_to_clients(state) {
        return;
    }
    if fingers == 1 && !state.session_locked {
        focus_for_touch(state, pos);
    }
    let (_, surface) = crate::input_handler::hit_test(state, pos);
    if let Some(touch) = state.seat.get_touch() {
        touch.down(
            state,
            surface,
            &DownEvent {
                slot,
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }
}

/// Keyboard focus follows a first finger onto a window or a layer surface
/// that takes keyboard input, like a click (a `keyboard_interactivity =
/// none` layer such as the mkeys on-screen keyboard never takes it).
fn focus_for_touch(state: &mut MargoState, pos: Point<f64, Logical>) {
    let (focus, _) = crate::input_handler::hit_test(state, pos);
    let Some((target, _)) = focus else {
        return;
    };
    let takes_keyboard = match &target {
        FocusTarget::LayerSurface(layer) => layer
            .with_cached_state(|data| data.keyboard_interactivity != KeyboardInteractivity::None),
        _ => true,
    };
    if takes_keyboard {
        state.focus_surface(Some(target));
    }
}

/// A finger moved to `pos` (global logical).
pub(crate) fn touch_motion_at(
    state: &mut MargoState,
    slot: TouchSlot,
    pos: Point<f64, Logical>,
    time: InputTime,
) {
    let id: i32 = slot.into();
    if let Some(p) = state.input_touch.points.iter_mut().find(|p| p.id == id) {
        p.x = pos.x;
        p.y = pos.y;
    }
    if !delivers_to_clients(state) {
        return;
    }
    let (_, surface) = crate::input_handler::hit_test(state, pos);
    if let Some(touch) = state.seat.get_touch() {
        touch.motion(
            state,
            surface,
            &MotionEvent {
                slot,
                location: pos,
                time,
            },
        );
    }
}

/// A finger lifted. When it was the last one, a claimed or armed
/// multi-finger sequence is resolved as a swipe.
pub(crate) fn touch_up_at(state: &mut MargoState, slot: TouchSlot, time: InputTime) {
    let id: i32 = slot.into();
    if delivers_to_clients(state)
        && let Some(touch) = state.seat.get_touch()
    {
        touch.up(
            state,
            &UpEvent {
                slot,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }

    let removed = state
        .input_touch
        .points
        .iter()
        .position(|p| p.id == id)
        .map(|i| state.input_touch.points.remove(i));
    if state.input_touch.gesture_armed
        && let Some(p) = removed
    {
        state.input_touch.releases.push(TouchRelease {
            start_x: p.start_x,
            start_y: p.start_y,
            end_x: p.x,
            end_y: p.y,
        });
    }

    // The sequence ends when every finger is up.
    if !state.input_touch.points.is_empty() {
        return;
    }
    let claimed = std::mem::take(&mut state.input_touch.claimed);
    if !state.input_touch.gesture_armed {
        return;
    }
    let releases = std::mem::take(&mut state.input_touch.releases);
    state.input_touch.gesture_armed = false;
    // Only a sequence we actually took from the client may dispatch a
    // compositor gesture. Several overlapping fingers can contribute to
    // `releases` without ever being down together at a bound count.
    if !claimed || releases.len() < 2 {
        // Not a multi-finger gesture; could be a tap that briefly
        // overlapped a pre-existing touch. Drop without dispatching.
        return;
    }

    // Average displacement (logical px) across every finger that
    // contributed — smooths out the natural finger-to-finger variation in
    // a hand-driven swipe.
    let n = releases.len() as f64;
    let avg_dx: f64 = releases.iter().map(|r| r.end_x - r.start_x).sum::<f64>() / n;
    let avg_dy: f64 = releases.iter().map(|r| r.end_y - r.start_y).sum::<f64>() / n;
    crate::input_handler::dispatch_swipe(
        state,
        avg_dx,
        avg_dy,
        releases.len() as u32,
        "touchscreen",
    );
}

/// End of one set of touch events (libinput `TouchFrame`).
pub(crate) fn touch_frame(state: &mut MargoState) {
    if !delivers_to_clients(state) {
        return;
    }
    if let Some(touch) = state.seat.get_touch() {
        touch.frame(state);
    }
}

/// The device cancelled the whole sequence (libinput `TouchCancel`, e.g.
/// palm rejection): tell the clients and forget the fingers.
pub(crate) fn touch_cancel(state: &mut MargoState) {
    if let Some(touch) = state.seat.get_touch() {
        touch.cancel(state);
    }
    state.input_touch = Default::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_panel_gets_the_touchscreen() {
        assert_eq!(pick_touch_output(["DP-1", "eDP-1"]), Some(1));
        assert_eq!(pick_touch_output(["HDMI-A-1", "DSI-1"]), Some(1));
        assert_eq!(pick_touch_output(["DP-1", "HDMI-A-1"]), Some(0));
        assert_eq!(pick_touch_output(std::iter::empty::<&str>()), None);
    }

    #[test]
    fn normalised_touches_map_onto_the_output_in_logical_px() {
        let geo = Rectangle::new((100, 50).into(), (1000, 800).into());
        let p = touch_point_in_output((0.5, 0.25), geo, Transform::Normal);
        assert_eq!((p.x, p.y), (600.0, 250.0));
        let p = touch_point_in_output((0.0, 0.0), geo, Transform::Normal);
        assert_eq!((p.x, p.y), (100.0, 50.0));
    }

    #[test]
    fn a_rotated_output_maps_panel_corners_onto_its_own_corners() {
        // A 1000x800 panel shown rotated: the output is 800x1000 logical.
        let geo = Rectangle::new((0, 0).into(), (800, 1000).into());
        for transform in [
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
        ] {
            let mut corners: Vec<(i64, i64)> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                .into_iter()
                .map(|n| {
                    let p = touch_point_in_output(n, geo, transform);
                    (p.x.round() as i64, p.y.round() as i64)
                })
                .collect();
            corners.sort();
            assert_eq!(
                corners,
                vec![(0, 0), (0, 1000), (800, 0), (800, 1000)],
                "{transform:?}"
            );
            let c = touch_point_in_output((0.5, 0.5), geo, transform);
            assert_eq!((c.x, c.y), (400.0, 500.0), "{transform:?}");
        }
        // 90°: the panel's top-left lands on a corner other than the
        // output's top-left (the rotation is applied, not ignored).
        let p = touch_point_in_output((0.0, 0.0), geo, Transform::_90);
        assert_ne!((p.x.round(), p.y.round()), (0.0, 0.0));
    }

    #[test]
    fn a_gesture_claims_the_touch_only_at_a_bound_finger_count() {
        assert!(
            !gesture_claims_touch(1, &[1, 3]),
            "one finger is the client's"
        );
        assert!(
            !gesture_claims_touch(2, &[3, 4]),
            "two-finger pinch stays the client's"
        );
        assert!(gesture_claims_touch(3, &[3, 4]));
        assert!(gesture_claims_touch(4, &[4]));
        assert!(!gesture_claims_touch(3, &[]));
    }
}
