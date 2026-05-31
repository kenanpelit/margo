//! Multi-monitor focus-movement tests (`focus_mon` / `focusmon`).
//!
//! `focus_mon(dir)` is the keybind path behind `focusmon left|right`.
//! On a multi-output desk it must cycle the *active* monitor in the
//! requested direction (wrapping at the ends) and warp the pointer to
//! the target so sloppy-focus doesn't immediately snap back — the
//! "Super+A bastım hiçbir şey olmuyor" symptom the implementation
//! comment calls out. With no keyboard attached in the fixture,
//! `focused_monitor()` resolves via `pointer_monitor()`, so these
//! tests track the active monitor through the pointer position the
//! warp leaves behind — exactly the observable a user perceives.

use super::fixture::Fixture;

/// Centre the pointer on a monitor so `focused_monitor()` resolves
/// to it before the first `focus_mon` call.
fn park_pointer_on(fx: &mut Fixture, mon: usize) {
    let area = fx.server.state.monitors[mon].monitor_area;
    fx.server.state.input_pointer.x = (area.x + area.width / 2) as f64;
    fx.server.state.input_pointer.y = (area.y + area.height / 2) as f64;
}

/// Forward then wrap: on a 2-output desk, `focus_mon(+1)` from
/// monitor 0 lands on 1; a second `+1` wraps back to 0.
#[test]
fn focus_mon_forward_cycles_and_wraps() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    park_pointer_on(&mut fx, 0);
    assert_eq!(fx.server.state.focused_monitor(), 0);

    fx.server.state.focus_mon(1);
    assert_eq!(
        fx.server.state.focused_monitor(),
        1,
        "+1 must advance to the next output",
    );

    fx.server.state.focus_mon(1);
    assert_eq!(
        fx.server.state.focused_monitor(),
        0,
        "+1 at the last output must wrap to the first",
    );
}

/// Backward wrap: `focus_mon(-1)` from monitor 0 wraps to the last
/// output (1 on a 2-output desk).
#[test]
fn focus_mon_backward_wraps_to_last() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    park_pointer_on(&mut fx, 0);

    fx.server.state.focus_mon(-1);
    assert_eq!(
        fx.server.state.focused_monitor(),
        1,
        "-1 at the first output must wrap to the last",
    );
}

/// The pointer must actually move onto the target output — without
/// the warp, sloppy-focus stays put. Assert the cursor lands inside
/// the target monitor's geometry after `focus_mon`.
#[test]
fn focus_mon_warps_pointer_onto_target_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    park_pointer_on(&mut fx, 0);

    fx.server.state.focus_mon(1);

    let target = fx.server.state.monitors[1].monitor_area;
    let px = fx.server.state.input_pointer.x;
    assert!(
        px >= target.x as f64 && px < (target.x + target.width) as f64,
        "pointer x={px} must be within DP-2 [{}, {})",
        target.x,
        target.x + target.width,
    );
}

/// Single-output desk: `focus_mon` is a guarded no-op (the
/// implementation returns early when `monitors.len() <= 1`), so the
/// active monitor and pointer stay put.
#[test]
fn focus_mon_is_noop_with_single_output() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    park_pointer_on(&mut fx, 0);
    let before = fx.server.state.input_pointer.x;

    fx.server.state.focus_mon(1);
    fx.server.state.focus_mon(-1);

    assert_eq!(fx.server.state.focused_monitor(), 0);
    assert_eq!(
        fx.server.state.input_pointer.x, before,
        "single-output focus_mon must not move the pointer",
    );
}
