//! Tag-move-across-outputs tests (`tag_mon` / `tagmon`).
//!
//! `tag_mon(dir)` moves the *focused* window to the adjacent output
//! and re-homes it onto that output's active tagset, then follows it
//! with the pointer + keyboard focus. It's the highest-risk
//! multi-monitor path (`docs/protocol-matrix.md` lists it first)
//! because it mutates per-output `MargoMonitor` state and the
//! client's `monitor` / `tags` in one step — a partial update strands
//! a window off-screen or focuses an empty output.
//!
//! Unlike the pure-state overview tests, this drives a **real
//! xdg_toplevel** through the fixture so the window genuinely lives
//! in `state.clients`, is mapped, and holds keyboard focus — without
//! a focused client `tag_mon` is a documented no-op. The fixture's
//! `add_keyboard` is what makes `focused_client_idx()` resolve.

use super::client::ClientId;
use super::fixture::Fixture;

/// Map a single focused toplevel and return its `ClientId`. Drives
/// the deferred-map flow to completion: create role → commit →
/// round-trip so `finalize_initial_map` runs (maps + focuses).
fn map_focused_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

/// A freshly-mapped window starts on monitor 0 and holds keyboard
/// focus — the precondition every `tag_mon` assertion below depends
/// on. If this regresses, the moves can't be trusted.
#[test]
fn mapped_window_is_focused_on_first_output() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    let _id = map_focused_window(&mut fx);

    assert_eq!(fx.server.state.clients.len(), 1);
    assert_eq!(fx.server.state.clients[0].monitor, 0);
    assert_eq!(
        fx.server.state.focused_client_idx(),
        Some(0),
        "the mapped toplevel must hold keyboard focus",
    );
}

/// `tag_mon(+1)` moves the focused window to the next output and
/// re-tags it onto that output's active tagset. Both the client's
/// `monitor` and `tags` must update together.
#[test]
fn tag_mon_moves_focused_window_to_next_output() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    // Give DP-2 a distinct active tag so re-homing is observable.
    fx.server.state.monitors[1].tagset[0] = 0b0000_0100;

    map_focused_window(&mut fx);
    assert_eq!(fx.server.state.clients[0].monitor, 0);

    fx.server.state.tag_mon(1);

    assert_eq!(
        fx.server.state.clients[0].monitor, 1,
        "window must migrate to the next output",
    );
    assert_eq!(
        fx.server.state.clients[0].tags, 0b0000_0100,
        "window must adopt the destination output's active tagset",
    );
    assert_eq!(
        fx.server.state.monitors[1].selected,
        Some(0),
        "the destination output must select the migrated window",
    );
}

/// `tag_mon` wraps: `-1` from output 0 lands the window on the last
/// output (1 on a 2-output desk).
#[test]
fn tag_mon_backward_wraps_to_last_output() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    map_focused_window(&mut fx);
    fx.server.state.tag_mon(-1);

    assert_eq!(
        fx.server.state.clients[0].monitor, 1,
        "-1 from the first output must wrap to the last",
    );
}

/// Single-output desk: `tag_mon` is a guarded no-op (returns early
/// when `monitors.len() <= 1`), so the window stays where it is.
#[test]
fn tag_mon_is_noop_with_single_output() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));

    map_focused_window(&mut fx);
    let tags_before = fx.server.state.clients[0].tags;

    fx.server.state.tag_mon(1);

    assert_eq!(fx.server.state.clients[0].monitor, 0);
    assert_eq!(
        fx.server.state.clients[0].tags, tags_before,
        "single-output tag_mon must not re-tag the window",
    );
}
