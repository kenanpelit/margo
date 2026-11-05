//! Touchscreen fingers reach the client under them over `wl_touch`.
//!
//! The seat never had the touch capability and touch events only fed the
//! `gesturebind` recogniser, so tapping a button in an app on a touch
//! panel did nothing. The recogniser also measured in normalised 0..1
//! device units against a pixel threshold, so a touchscreen swipe could
//! never fire a bind.

use margo_config::{Config, GestureBinding};
use smithay::backend::input::{InputTime, TouchSlot};
use smithay::utils::{Logical, Point};
use wayland_client::Proxy;

use super::client::ClientId;
use super::fixture::Fixture;
use crate::input::touch::{touch_cancel, touch_down_at, touch_frame, touch_motion_at, touch_up_at};
use crate::layout::LayoutId;

/// One mapped 400x300 window, its client holding a `wl_touch`.
fn window_with_touch(config: Config) -> (Fixture, ClientId, u32, impl Sized) {
    let mut fx = Fixture::with_config(config);
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let buffer = fx.client(id).create_shm_buffer(400, 300);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let touch = fx.client(id).create_touch();
    fx.roundtrip(id);
    let sid = surface.id().protocol_id();
    (fx, id, sid, (toplevel, surface, buffer, touch))
}

/// A point `(dx, dy)` inside the first window, in global logical space.
fn in_window(fx: &Fixture, dx: i32, dy: i32) -> Point<f64, Logical> {
    let g = fx.server.state.clients[0].geom;
    (f64::from(g.x + dx), f64::from(g.y + dy)).into()
}

fn slot(n: u32) -> TouchSlot {
    Some(n).into()
}

fn events(fx: &mut Fixture, id: ClientId) -> Vec<String> {
    fx.roundtrip(id);
    std::mem::take(&mut fx.client(id).state.touch_events)
}

fn now() -> InputTime {
    InputTime::now()
}

#[test]
fn a_tap_reaches_the_window_under_the_finger() {
    let (mut fx, id, sid, _keep) = window_with_touch(Config::default());
    let p = in_window(&fx, 10, 20);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(0), p, now());
    touch_frame(state);
    touch_up_at(state, slot(0), now());
    touch_frame(state);

    assert_eq!(
        events(&mut fx, id),
        [
            format!("down 0 {sid} 10 20"),
            "frame".into(),
            "up 0".into(),
            "frame".into()
        ]
    );
}

#[test]
fn a_finger_keeps_its_surface_while_it_moves_off_it() {
    let (mut fx, id, sid, _keep) = window_with_touch(Config::default());
    let p = in_window(&fx, 10, 10);
    let outside = in_window(&fx, 600, 10);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(3), p, now());
    touch_motion_at(state, slot(3), outside, now());
    touch_frame(state);
    touch_up_at(state, slot(3), now());

    assert_eq!(
        events(&mut fx, id),
        [
            format!("down 3 {sid} 10 10"),
            "motion 3 600 10".into(),
            "frame".into(),
            "up 3".into()
        ]
    );
}

#[test]
fn a_tap_focuses_the_window() {
    let (mut fx, _id, _sid, _keep) = window_with_touch(Config::default());
    let p = in_window(&fx, 5, 5);
    let state = &mut fx.server.state;
    state.set_keyboard_focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
    touch_down_at(state, slot(0), p, now());
    touch_up_at(state, slot(0), now());
    assert_eq!(state.focused_client_idx(), Some(0));
}

fn three_finger_right_bind() -> Config {
    let mut config = Config::default();
    config.gesture_bindings.push(GestureBinding {
        modifiers: margo_config::Modifiers::empty(),
        motion: 2, // right
        fingers: 3,
        action: "setlayout".into(),
        arg: margo_config::Arg {
            v: Some("monocle".into()),
            ..Default::default()
        },
    });
    config
}

#[test]
fn a_bound_multi_finger_swipe_is_margos_and_cancels_the_clients_sequence() {
    let (mut fx, id, sid, _keep) = window_with_touch(three_finger_right_bind());
    assert_ne!(
        fx.server.state.monitors[0].current_layout(),
        LayoutId::Monocle
    );
    let starts: Vec<_> = (0..3).map(|i| in_window(&fx, 10 + 30 * i, 50)).collect();
    let state = &mut fx.server.state;
    for (i, p) in starts.iter().enumerate() {
        touch_down_at(state, slot(i as u32), *p, now());
    }
    touch_frame(state);
    // 300 logical px to the right: well past the swipe threshold.
    for (i, p) in starts.iter().enumerate() {
        touch_motion_at(state, slot(i as u32), (p.x + 300.0, p.y).into(), now());
    }
    touch_frame(state);
    for i in 0..3 {
        touch_up_at(state, slot(i), now());
    }

    assert_eq!(
        fx.server.state.monitors[0].current_layout(),
        LayoutId::Monocle,
        "the 3-finger swipe ran its gesturebind"
    );
    assert_eq!(
        events(&mut fx, id),
        [
            format!("down 0 {sid} 10 50"),
            format!("down 1 {sid} 40 50"),
            "cancel".into()
        ],
        "the third finger claims the sequence; nothing after the cancel"
    );

    // A later tap is the client's again.
    let p = in_window(&fx, 10, 10);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(5), p, now());
    touch_up_at(state, slot(5), now());
    assert_eq!(
        events(&mut fx, id),
        [format!("down 5 {sid} 10 10"), "up 5".into()]
    );
}

#[test]
fn an_unbound_two_finger_pinch_stays_the_clients() {
    let (mut fx, id, sid, _keep) = window_with_touch(three_finger_right_bind());
    let (a, b) = (in_window(&fx, 100, 100), in_window(&fx, 200, 100));
    let state = &mut fx.server.state;
    touch_down_at(state, slot(0), a, now());
    touch_down_at(state, slot(1), b, now());
    touch_motion_at(state, slot(1), (b.x + 50.0, b.y).into(), now());
    touch_up_at(state, slot(0), now());
    touch_up_at(state, slot(1), now());
    assert_eq!(
        events(&mut fx, id),
        [
            format!("down 0 {sid} 100 100"),
            format!("down 1 {sid} 200 100"),
            "motion 1 250 100".into(),
            "up 0".into(),
            "up 1".into()
        ]
    );
}

#[test]
fn a_device_cancel_reaches_the_client_and_resets_the_recogniser() {
    let (mut fx, id, sid, _keep) = window_with_touch(Config::default());
    let p = in_window(&fx, 10, 10);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(0), p, now());
    touch_cancel(state);
    assert!(state.input_touch.points.is_empty());
    assert_eq!(
        events(&mut fx, id),
        [format!("down 0 {sid} 10 10"), "cancel".into()]
    );
}

#[test]
fn locking_cancels_a_finger_that_is_down_on_a_window() {
    // smithay keeps a finger's focus from its `down`: without a cancel at
    // lock time, its motion would keep reaching the window behind the lock.
    let (mut fx, id, sid, _keep) = window_with_touch(Config::default());
    let p = in_window(&fx, 10, 10);
    touch_down_at(&mut fx.server.state, slot(0), p, now());
    let locker = fx.add_client();
    let _lock = fx.client(locker).create_session_lock();
    fx.roundtrip(locker);
    assert!(fx.server.state.session_locked);
    let moved = in_window(&fx, 20, 20);
    touch_motion_at(&mut fx.server.state, slot(0), moved, now());
    touch_up_at(&mut fx.server.state, slot(0), now());

    assert_eq!(
        events(&mut fx, id),
        [format!("down 0 {sid} 10 10"), "cancel".into()]
    );
}

#[test]
fn touches_never_reach_a_window_while_locked() {
    let (mut fx, id, _sid, _keep) = window_with_touch(Config::default());
    let locker = fx.add_client();
    let _lock = fx.client(locker).create_session_lock();
    fx.roundtrip(locker);
    let p = in_window(&fx, 10, 10);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(0), p, now());
    touch_up_at(state, slot(0), now());
    assert!(events(&mut fx, id).is_empty());
}

#[test]
fn the_tablet_global_is_not_advertised_until_tablet_input_is_routed() {
    // Tablet tool events are not handled yet (roadmap W2.3); advertising
    // zwp_tablet_manager_v2 made apps believe a tablet exists.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(names.iter().any(|n| n == "wl_seat"), "{names:?}");
    assert!(
        !names.iter().any(|n| n == "zwp_tablet_manager_v2"),
        "{names:?}"
    );
}

#[test]
fn overlapping_fingers_that_never_reach_the_bound_count_do_not_run_a_bind() {
    let (mut fx, _id, _sid, _keep) = window_with_touch(three_finger_right_bind());
    let p = in_window(&fx, 10, 50);
    let state = &mut fx.server.state;
    touch_down_at(state, slot(0), p, now());
    touch_down_at(state, slot(1), p, now());
    touch_motion_at(state, slot(1), (p.x + 300.0, p.y).into(), now());
    touch_up_at(state, slot(1), now());
    // A third finger over the life of the sequence, but never three at
    // once: the sequence wasn't claimed/cancelled, so must stay the app's.
    touch_down_at(state, slot(2), p, now());
    touch_motion_at(state, slot(2), (p.x + 300.0, p.y).into(), now());
    touch_up_at(state, slot(2), now());
    touch_motion_at(state, slot(0), (p.x + 300.0, p.y).into(), now());
    touch_up_at(state, slot(0), now());
    assert_ne!(state.monitors[0].current_layout(), LayoutId::Monocle);
}
