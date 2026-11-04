//! Touchpad gestures reach clients through `zwp_pointer_gestures_v1`:
//! pinch and hold always go to the surface under the pointer, a swipe
//! only when no `gesturebind` uses its finger count (margo keeps those).
//! Before, the global was advertised but no event was ever forwarded:
//! pinch-to-zoom never worked and unbound swipes were swallowed.

use margo_config::{Config, GestureBinding};

use super::client::ClientId;
use super::fixture::Fixture;
use crate::input_handler as input;

/// One mapped window under the pointer, its client subscribed to all
/// three gesture kinds.
fn window_under_pointer(config: Config) -> (Fixture, ClientId, impl Sized) {
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

    let pointer = fx.client(id).create_pointer();
    let gestures = fx.client(id).subscribe_gestures(&pointer);
    fx.roundtrip(id);

    let state = &mut fx.server.state;
    let g = state.clients[0].geom;
    state.input_pointer.x = f64::from(g.x + 10);
    state.input_pointer.y = f64::from(g.y + 10);
    state.refresh_pointer_focus();
    assert!(
        state
            .seat
            .get_pointer()
            .and_then(|p| p.current_focus())
            .is_some(),
        "the window must have pointer focus"
    );
    (fx, id, (toplevel, surface, buffer, pointer, gestures))
}

fn events(fx: &mut Fixture, id: ClientId) -> Vec<String> {
    fx.roundtrip(id);
    std::mem::take(&mut fx.client(id).state.gesture_events)
}

#[test]
fn pinch_and_hold_reach_the_client() {
    let (mut fx, id, _keep) = window_under_pointer(Config::default());
    let state = &mut fx.server.state;
    input::gesture_pinch_begin(state, 2, 1);
    input::gesture_pinch_update(state, 2, (1.0, 0.0), 1.5, 0.0);
    input::gesture_pinch_end(state, 3, false);
    input::gesture_hold_begin(state, 3, 4);
    input::gesture_hold_end(state, 5, true);

    assert_eq!(
        events(&mut fx, id),
        [
            "pinch begin 2",
            "pinch update 1.5",
            "pinch end",
            "hold begin 3",
            "hold end"
        ]
    );
}

#[test]
fn an_unbound_swipe_reaches_the_client_and_a_bound_one_does_not() {
    let mut config = Config::default();
    config.gesture_bindings.push(GestureBinding {
        modifiers: margo_config::Modifiers::empty(),
        motion: 2,
        fingers: 3,
        action: "viewtoright".into(),
        arg: Default::default(),
    });
    let (mut fx, id, _keep) = window_under_pointer(config);
    let state = &mut fx.server.state;

    input::gesture_swipe_begin(state, 4, 1);
    input::gesture_swipe_update(state, 2, (40.0, 0.0));
    input::gesture_swipe_end(state, 3, false);
    assert_eq!(
        events(&mut fx, id),
        ["swipe begin 4", "swipe update", "swipe end"]
    );

    let state = &mut fx.server.state;
    input::gesture_swipe_begin(state, 3, 4);
    input::gesture_swipe_update(state, 5, (40.0, 0.0));
    input::gesture_swipe_end(state, 6, false);
    assert!(
        events(&mut fx, id).is_empty(),
        "a swipe with a gesturebind's finger count stays margo's"
    );
}

#[test]
fn nothing_is_forwarded_while_locked() {
    let (mut fx, id, _keep) = window_under_pointer(Config::default());
    let state = &mut fx.server.state;
    state.session_locked = true;
    input::gesture_pinch_begin(state, 2, 1);
    input::gesture_pinch_end(state, 2, false);
    input::gesture_swipe_begin(state, 4, 3);
    input::gesture_swipe_end(state, 4, false);
    assert!(events(&mut fx, id).is_empty());
}
