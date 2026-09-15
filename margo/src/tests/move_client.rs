//! `move_client` — directional "push aside" reorder (mango 0.17 port,
//! c844a107). Distinct from `exchange_client`'s list-order swap: this
//! finds the geometrically nearest neighbour in a direction and
//! re-inserts the focused client next to it, shifting anything in
//! between rather than swapping two fixed slots.

use margo_config::{Config, Direction};

use super::fixture::Fixture;
use crate::layout::{LayoutId, Rect};
use crate::state::FocusTarget;

/// Three tiled clients (A, B, C) with controlled geometry: A occupies
/// the left half full-height (a "master" slot); B and C sit in the
/// right half, both equidistant in x from A, but B is a short cell
/// near the top and C a tall cell whose centre sits much closer to
/// A's — so "move right" from A must reach past B to land next to C.
/// Returns the fixture plus each client's stable id, in creation
/// order (A, B, C).
fn three_tiled_clients() -> (Fixture, [u64; 3]) {
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    for _ in 0..3 {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id("kitty".into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    fx.server.state.monitors[0].pertag.ltidxs[1] = LayoutId::Tile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;
    fx.server.state.arrange_monitor(0);

    fx.server.state.clients[0].geom = Rect::new(0, 0, 960, 1080);
    fx.server.state.clients[1].geom = Rect::new(960, 0, 960, 300);
    fx.server.state.clients[2].geom = Rect::new(960, 300, 960, 780);

    let ids = [
        fx.server.state.clients[0].id,
        fx.server.state.clients[1].id,
        fx.server.state.clients[2].id,
    ];

    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));
    (fx, ids)
}

fn id_order(fx: &Fixture) -> Vec<u64> {
    fx.server.state.clients.iter().map(|c| c.id).collect()
}

#[test]
fn move_client_reaches_past_the_closer_neighbour_to_the_nearest_one() {
    let (mut fx, [a, b, c]) = three_tiled_clients();

    fx.server.state.move_client(Direction::Right);

    assert_eq!(
        id_order(&fx),
        vec![b, c, a],
        "A must land adjacent to C (the geometrically nearest neighbour), \
         shifting B forward — a plain swap would give [C, B, A] or [B, A, C]"
    );
}

#[test]
fn move_client_is_a_noop_with_no_neighbour_that_way() {
    let (mut fx, ids) = three_tiled_clients();

    fx.server.state.move_client(Direction::Left); // nothing left of A

    assert_eq!(
        id_order(&fx),
        ids.to_vec(),
        "no neighbour left of A — order must be untouched"
    );
}

#[test]
fn move_client_ignores_a_floating_focus() {
    let (mut fx, ids) = three_tiled_clients();
    fx.server.state.clients[0].is_floating = true;

    fx.server.state.move_client(Direction::Right);

    assert_eq!(
        id_order(&fx),
        ids.to_vec(),
        "a floating focus must not be moved"
    );
}

#[test]
fn move_client_keeps_the_moved_window_focused() {
    let (mut fx, [a, ..]) = three_tiled_clients();

    fx.server.state.move_client(Direction::Right);

    let focused_id = fx
        .server
        .state
        .focused_client_idx()
        .map(|idx| fx.server.state.clients[idx].id);
    assert_eq!(focused_id, Some(a), "the moved window must stay focused");
}
