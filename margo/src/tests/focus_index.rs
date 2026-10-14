//! `Monitor.selected` / `prev_selected` are raw indices into `clients`.
//! Reordering dispatches (`exchange_stack`, `zoom`, directional moves,
//! drag-swap, group tab moves) must keep them on the same *windows*,
//! or `focuslast` jumps to whichever window took the old slot.

use super::fixture::Fixture;
use crate::state::FocusTarget;

fn three_windows(fx: &mut Fixture) {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.animations = false;
    for _ in 0..3 {
        let id = fx.add_client();
        let (_toplevel, surface) = fx.client(id).create_toplevel();
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
}

fn focus(fx: &mut Fixture, idx: usize) {
    let w = fx.server.state.clients[idx].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w)));
}

#[test]
fn focuslast_after_exchange_stack_returns_to_the_previous_window() {
    let mut fx = Fixture::new();
    three_windows(&mut fx);

    let ids: Vec<u64> = fx.server.state.clients.iter().map(|c| c.id).collect();
    focus(&mut fx, 0);
    focus(&mut fx, 1);
    let previous = ids[0];
    let current = ids[1];

    fx.server.state.exchange_stack(1);
    let focused = fx.server.state.focused_client_idx().unwrap();
    assert_eq!(fx.server.state.clients[focused].id, current);

    fx.server.state.focus_last();
    let focused = fx.server.state.focused_client_idx().unwrap();
    assert_eq!(
        fx.server.state.clients[focused].id, previous,
        "focuslast must go back to the window focused before, not the swap partner"
    );
}

#[test]
fn move_client_keeps_selected_on_the_same_window() {
    let mut fx = Fixture::new();
    three_windows(&mut fx);
    focus(&mut fx, 2);
    let id = fx.server.state.clients[2].id;

    fx.server.state.move_client_index(2, 0);
    let sel = fx.server.state.monitors[0].selected.unwrap();
    assert_eq!(fx.server.state.clients[sel].id, id);
}
