//! `hot_corner_disable_on_fullscreen` — suppress the hot-corner dwell
//! trigger while the focused window is fullscreen (mango
//! `hotarea_disable_on_fullscreen` port).

use margo_config::Config;

use super::fixture::Fixture;
use crate::input_handler::update_hot_corner;
use crate::state::{FocusTarget, HotCorner};

fn fixture_with_top_left_bind(disable_on_fullscreen: bool) -> Fixture {
    let mut fx = Fixture::with_config(Config {
        hot_corner_top_left: "toggle_overview".to_string(),
        hot_corner_disable_on_fullscreen: disable_on_fullscreen,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));

    // Top-left corner of the first (0,0)-origin output.
    fx.server.state.input_pointer.x = 0.0;
    fx.server.state.input_pointer.y = 0.0;
    fx
}

#[test]
fn fullscreen_focus_suppresses_the_hot_corner_by_default() {
    let mut fx = fixture_with_top_left_bind(true);
    fx.server.state.clients[0].is_fullscreen = true;

    update_hot_corner(&mut fx.server.state);

    assert_eq!(
        fx.server.state.hot_corner_dwelling, None,
        "a fullscreen focused window must block corner entry from arming at all"
    );
}

#[test]
fn non_fullscreen_focus_still_arms_the_hot_corner() {
    let mut fx = fixture_with_top_left_bind(true);
    // Client stays non-fullscreen (default).

    update_hot_corner(&mut fx.server.state);

    assert_eq!(
        fx.server.state.hot_corner_dwelling,
        Some(HotCorner::TopLeft),
        "a non-fullscreen focus must not be affected by the fullscreen guard"
    );
}

#[test]
fn disabling_the_guard_lets_fullscreen_arm_the_corner_too() {
    let mut fx = fixture_with_top_left_bind(false);
    fx.server.state.clients[0].is_fullscreen = true;

    update_hot_corner(&mut fx.server.state);

    assert_eq!(
        fx.server.state.hot_corner_dwelling,
        Some(HotCorner::TopLeft),
        "with hot_corner_disable_on_fullscreen off, fullscreen must not suppress the corner"
    );
}
