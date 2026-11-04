//! A cursor image change repaints only the output under the pointer, and
//! not at all when nothing changed.

use smithay::input::SeatHandler;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};

use super::fixture::Fixture;

fn fixture() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    // Pointer on DP-2.
    fx.server.state.input_pointer.x = 2500.0;
    fx.server.state.input_pointer.y = 500.0;
    settle(&mut fx);
    fx
}

fn settle(fx: &mut Fixture) {
    let state = &mut fx.server.state;
    let tokens: Vec<_> = state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        state.loop_handle.remove(token);
    }
    for clock in state.per_output_clocks.values_mut() {
        clock.dirty = false;
        clock.pending_vblank = false;
    }
    state.take_repaint_request();
}

fn dirty(fx: &Fixture, name: &str) -> bool {
    fx.server
        .state
        .per_output_clocks
        .get(name)
        .is_some_and(|clock| clock.dirty)
}

fn set_cursor(fx: &mut Fixture, status: CursorImageStatus) {
    let seat = fx.server.state.seat.clone();
    fx.server.state.cursor_image(&seat, status);
}

#[test]
fn a_new_cursor_image_repaints_only_the_output_under_the_pointer() {
    let mut fx = fixture();
    set_cursor(&mut fx, CursorImageStatus::Named(CursorIcon::Pointer));
    assert!(dirty(&fx, "DP-2"));
    assert!(!dirty(&fx, "DP-1"), "the cursor is not on DP-1");
}

#[test]
fn the_same_cursor_image_again_repaints_nothing() {
    let mut fx = fixture();
    set_cursor(&mut fx, CursorImageStatus::Named(CursorIcon::Pointer));
    settle(&mut fx);
    set_cursor(&mut fx, CursorImageStatus::Named(CursorIcon::Pointer));
    assert!(!dirty(&fx, "DP-1"));
    assert!(!dirty(&fx, "DP-2"));
    set_cursor(&mut fx, CursorImageStatus::Hidden);
    assert!(dirty(&fx, "DP-2"), "hiding it is a change");
    settle(&mut fx);
    set_cursor(&mut fx, CursorImageStatus::Hidden);
    assert!(!dirty(&fx, "DP-2"));
}

#[test]
fn a_cursor_change_off_every_output_repaints_nothing() {
    let mut fx = fixture();
    fx.server.state.input_pointer.x = -50.0;
    set_cursor(&mut fx, CursorImageStatus::Named(CursorIcon::Wait));
    assert!(!dirty(&fx, "DP-1"));
    assert!(!dirty(&fx, "DP-2"));
}
