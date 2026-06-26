//! `active_output` resolution — which monitor a keybind/IPC menu open
//! (launcher, settings, every pill menu) targets.
//!
//! mshell routes a toggled menu to the Frame on the monitor named by
//! margo's `active_output` snapshot field. That field is last-writer-wins
//! between two input signals (`ActiveOutputSource`): keyboard activity on
//! the focused monitor (`Focus`) vs. the pointer crossing into a monitor
//! (`Pointer`). These tests lock down both arms — in particular the bug
//! they fix: with the cursor parked on another output, a keyboard-driven
//! menu open must still land on the *focused* monitor, not the cursor's.

use super::fixture::Fixture;
use crate::state::ActiveOutputSource;

/// Centre the pointer on a monitor and run the crossing detector, exactly
/// as a real motion event would. This also flips the active-output source
/// to `Pointer` (the production behaviour we assert separately below).
fn move_pointer_onto(fx: &mut Fixture, mon: usize) {
    let area = fx.server.state.monitors[mon].monitor_area;
    fx.server.state.input_pointer.x = (area.x + area.width / 2) as f64;
    fx.server.state.input_pointer.y = (area.y + area.height / 2) as f64;
    fx.server.state.refresh_pointer_monitor_tracking();
}

/// Map a single focused toplevel; it lands on monitor 0 and holds
/// keyboard focus (so `focused_monitor()` resolves to 0).
fn map_focused_window(fx: &mut Fixture) {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

fn active_output(fx: &Fixture) -> String {
    fx.server.state.build_state_snapshot()["active_output"]
        .as_str()
        .expect("active_output is a string")
        .to_string()
}

/// The pointer crossing into a monitor makes the pointer the active
/// source, so a menu open follows the cursor.
#[test]
fn pointer_crossing_flips_source_to_pointer() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    // Default at startup is keyboard-focus.
    assert_eq!(
        fx.server.state.active_output_source,
        ActiveOutputSource::Focus
    );

    move_pointer_onto(&mut fx, 1);

    assert_eq!(
        fx.server.state.active_output_source,
        ActiveOutputSource::Pointer,
        "crossing into a monitor must make the pointer the active source",
    );
    assert_eq!(
        active_output(&fx),
        "DP-2",
        "pointer-sourced active_output follows the cursor's monitor",
    );
}

/// The bug fix: working on the keyboard-focused monitor while the cursor
/// is parked on another output must open menus on the *focused* monitor.
#[test]
fn focus_source_keeps_menu_on_focused_monitor_despite_cursor() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    // Focus a window on DP-1, then park the cursor on DP-2 (which also
    // flips the source to Pointer, as a stray mouse-move would).
    map_focused_window(&mut fx);
    assert_eq!(fx.server.state.clients[0].monitor, 0);
    move_pointer_onto(&mut fx, 1);
    assert_eq!(
        active_output(&fx),
        "DP-2",
        "precondition: with a pointer source the cursor's monitor wins",
    );

    // A keyboard keybind (tag switch, focus move, typing, …) makes the
    // focused monitor the active source.
    fx.server.state.active_output_source = ActiveOutputSource::Focus;

    assert_eq!(
        active_output(&fx),
        "DP-1",
        "focus-sourced active_output ignores the parked cursor and \
         follows keyboard focus",
    );
}

/// The cursor-priority case is preserved: with the pointer as the active
/// source, a menu opens under the cursor even when a window is focused on
/// another monitor (e.g. mouse onto an empty output → launcher there).
#[test]
fn pointer_source_keeps_menu_under_cursor_despite_focus() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    map_focused_window(&mut fx); // focus on DP-1
    move_pointer_onto(&mut fx, 1); // cursor + source → DP-2

    assert_eq!(
        fx.server.state.active_output_source,
        ActiveOutputSource::Pointer,
    );
    assert_eq!(
        active_output(&fx),
        "DP-2",
        "pointer source follows the cursor even with focus on DP-1",
    );
}
