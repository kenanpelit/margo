//! Overview state-transition tests.
//!
//! Guards against the regression that 8c58b20 introduced (and
//! 953caf2 reverted): targeted-arrange optimizations were
//! corrupting per-monitor tagset state across rapid toggles.
//! These tests pin the open / close / rapid-toggle invariants so
//! a future "perf" pass can't silently drop them.

use super::fixture::Fixture;

/// open_overview must flip every monitor's `is_overview` flag to
/// true and remember the pre-overview tagset. This is the contract
/// `close_overview` relies on to restore the right tagset later.
#[test]
fn open_overview_flips_every_monitor_and_records_backup() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    // Set distinct pre-overview tagsets so a "wrong tagset on
    // restore" regression actually has signal.
    fx.server.state.monitors[0].tagset[0] = 0b0000_0001;
    fx.server.state.monitors[1].tagset[0] = 0b0000_0100;

    fx.server.state.open_overview();

    assert!(fx.server.state.is_overview_open());
    for (i, mon) in fx.server.state.monitors.iter().enumerate() {
        assert!(mon.is_overview, "mon[{i}] should be in overview");
    }
    assert_eq!(fx.server.state.monitors[0].overview_backup_tagset, 0b0000_0001);
    assert_eq!(fx.server.state.monitors[1].overview_backup_tagset, 0b0000_0100);
}

/// close_overview must restore each monitor's pre-overview tagset
/// independently — DP-1's tag-1 stays tag-1 even though DP-2 was
/// on tag-3. The earlier broken implementation was collapsing all
/// monitors to the same tagset; this test pins the invariant.
#[test]
fn close_overview_restores_each_monitor_independently() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.server.state.monitors[0].tagset[0] = 0b0000_0001;
    fx.server.state.monitors[1].tagset[0] = 0b0000_0100;

    fx.server.state.open_overview();
    fx.server.state.close_overview(None);

    assert!(!fx.server.state.is_overview_open());
    assert_eq!(
        fx.server.state.monitors[0].current_tagset(),
        0b0000_0001,
        "mon[0] tagset must be restored to its pre-overview value",
    );
    assert_eq!(
        fx.server.state.monitors[1].current_tagset(),
        0b0000_0100,
        "mon[1] tagset must be restored to its pre-overview value",
    );
}

/// open_overview is idempotent — calling it while already in
/// overview must not clobber the original `overview_backup_tagset`
/// (which would happen if the second call overwrote it with the
/// in-overview "all tags" tagset). This is the exact bug that
/// caused "all windows collapse onto a single tag after the second
/// toggle" in the previously-reverted attempt.
#[test]
fn double_open_overview_does_not_clobber_backup() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.monitors[0].tagset[0] = 0b0000_0010;

    fx.server.state.open_overview();
    fx.server.state.open_overview();
    fx.server.state.close_overview(None);

    assert_eq!(fx.server.state.monitors[0].current_tagset(), 0b0000_0010);
}

/// Rapid toggle (open / close ×5) must leave the state machine
/// coherent — no panics, tagsets restored, all monitors out of
/// overview. The reverted `8c58b20` was crashing after 2-3
/// toggles; this is the regression test for that crash.
#[test]
fn rapid_toggle_is_stable_across_many_iterations() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.add_output("DP-3", (1920, 1080));
    fx.server.state.monitors[0].tagset[0] = 0b0000_0001;
    fx.server.state.monitors[1].tagset[0] = 0b0000_0010;
    fx.server.state.monitors[2].tagset[0] = 0b0000_0100;

    for _ in 0..5 {
        fx.server.state.toggle_overview();
        assert!(fx.server.state.is_overview_open());
        fx.server.state.toggle_overview();
        assert!(!fx.server.state.is_overview_open());
    }

    assert_eq!(fx.server.state.monitors[0].current_tagset(), 0b0000_0001);
    assert_eq!(fx.server.state.monitors[1].current_tagset(), 0b0000_0010);
    assert_eq!(fx.server.state.monitors[2].current_tagset(), 0b0000_0100);
}

/// While in overview, the per-arrange animation duration override
/// is set; once close_overview returns, it must be back to None so
/// later arranges (focus shifts, gestures) use the user's
/// configured `animation_duration_move`. A leaked override would
/// silently shrink every move animation site-wide to the snappy
/// 180 ms — surprising and hard to track down.
#[test]
fn overview_animation_override_clears_after_close() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));

    fx.server.state.open_overview();
    fx.server.state.close_overview(None);

    assert!(
        fx.server.state.overview_transition_animation_ms.is_none(),
        "transition override must be cleared after close_overview",
    );
}
