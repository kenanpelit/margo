//! Overview state-machine tests — pin the behaviour of
//! `open_overview` / `close_overview` / `toggle_overview` against
//! the fixture so the perf-driven refactor (cached
//! `overview_open_count`, monitor-targeted arrange instead of
//! full `arrange_all`, batched dwl-ipc broadcast) doesn't silently
//! drift the user-observable semantics.

use super::fixture::Fixture;

#[test]
fn overview_starts_closed() {
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    assert_eq!(fx.server.state.overview_open_count, 0);
    assert!(!fx.server.state.is_overview_open());
    assert!(!fx.server.state.monitors[0].is_overview);
}

#[test]
fn open_overview_flips_single_monitor_count() {
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));

    fx.server.state.open_overview();
    assert!(fx.server.state.monitors[0].is_overview);
    assert_eq!(
        fx.server.state.overview_open_count, 1,
        "overview_open_count must equal the number of monitors flipped",
    );
    assert!(fx.server.state.is_overview_open());
}

#[test]
fn open_overview_then_close_returns_to_clean_state() {
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));

    fx.server.state.open_overview();
    fx.server.state.close_overview(None);

    assert_eq!(fx.server.state.overview_open_count, 0);
    assert!(!fx.server.state.is_overview_open());
    assert!(!fx.server.state.monitors[0].is_overview);
}

#[test]
fn open_overview_is_idempotent() {
    // Calling open twice must not double the cached count — the
    // second call sees `is_overview` already true on every
    // monitor and is a no-op. Without the idempotency check the
    // count would drift past the actual monitor count, breaking
    // close_overview's flipped-detection.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    fx.add_output("HEADLESS-2", (1920, 1080));

    fx.server.state.open_overview();
    fx.server.state.open_overview();

    assert_eq!(
        fx.server.state.overview_open_count, 2,
        "second open_overview must NOT increment past the monitor count",
    );
}

#[test]
fn close_overview_with_no_overview_open_is_a_no_op() {
    // Pre-W*.Y this used `is_overview_open()` (an iter-every-
    // monitor scan) to early-return; the cached
    // `overview_open_count == 0` check produces the same effect
    // in O(1). Pin behaviour so the optimisation can't regress
    // into a stuck "phantom close" pass.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));

    let _before_clients_len = fx.server.state.clients.len();
    fx.server.state.close_overview(None);
    assert_eq!(fx.server.state.overview_open_count, 0);
    assert!(!fx.server.state.monitors[0].is_overview);
}

#[test]
fn toggle_overview_round_trips() {
    // Two toggles == clean state. A regression here would mean
    // the cached count drifted, so a "double-tap to dismiss"
    // user gesture leaves the compositor in a half-open state.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));

    fx.server.state.toggle_overview();
    assert!(fx.server.state.is_overview_open());
    assert_eq!(fx.server.state.overview_open_count, 1);

    fx.server.state.toggle_overview();
    assert!(!fx.server.state.is_overview_open());
    assert_eq!(fx.server.state.overview_open_count, 0);
}

#[test]
fn open_overview_flips_all_monitors_at_once() {
    // Multi-monitor: a single open_overview call should flip
    // EVERY monitor, not just the focused one. Pre-refactor
    // this was already the case (it iterated all monitors); the
    // refactor preserves it via the `flipped` Vec.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    fx.add_output("HEADLESS-2", (2560, 1440));

    fx.server.state.open_overview();
    assert_eq!(fx.server.state.overview_open_count, 2);
    for (idx, mon) in fx.server.state.monitors.iter().enumerate() {
        assert!(mon.is_overview, "monitor {idx} should be in overview");
    }
}

#[test]
fn close_overview_clears_all_monitors() {
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    fx.add_output("HEADLESS-2", (2560, 1440));

    fx.server.state.open_overview();
    fx.server.state.close_overview(None);

    assert_eq!(fx.server.state.overview_open_count, 0);
    for (idx, mon) in fx.server.state.monitors.iter().enumerate() {
        assert!(!mon.is_overview, "monitor {idx} should be out of overview");
    }
}

#[test]
fn open_overview_without_monitors_is_safe() {
    // Edge case: lock screen scenarios may briefly run with no
    // monitors. open_overview must not panic or push to
    // `overview_open_count`.
    let mut fx = Fixture::new();
    fx.server.state.open_overview();
    assert_eq!(fx.server.state.overview_open_count, 0);
}

#[test]
fn close_overview_preserves_pre_overview_tagset() {
    // open_overview snapshots the active tagset into
    // `overview_backup_tagset`. close_overview without an
    // activate-window argument must restore that snapshot — that's
    // the "press overview, glance, press again, you're back exactly
    // where you were" UX contract.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));

    // Switch to tag 4 (mask = 8) before opening.
    fx.server.state.monitors[0].tagset[0] = 8;
    let pre = fx.server.state.monitors[0].current_tagset();

    fx.server.state.open_overview();
    assert_eq!(
        fx.server.state.monitors[0].overview_backup_tagset, pre,
        "open_overview must snapshot the pre-overview tagset",
    );

    fx.server.state.close_overview(None);
    assert_eq!(
        fx.server.state.monitors[0].current_tagset(),
        pre,
        "close_overview without activate-window restores the snapshot",
    );
}
