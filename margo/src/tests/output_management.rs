//! Integration tests for `OutputManagementHandler` (W4.2 Phase 4
//! extracted impl at `state/handlers/output_management.rs`).
//!
//! `zwlr_output_manager_v1` is what `wlr-randr` / `kanshi` bind
//! to apply runtime topology / scale / transform / mode / disable
//! changes. The handler:
//!
//! * Refuses to apply a config that would leave zero enabled
//!   outputs (would strand the user with a dark screen and no
//!   recovery short of TTY login).
//! * Updates Output::change_current_state for scale / transform
//!   / position synchronously, defers mode change to the udev
//!   backend.
//! * Re-publishes topology after a successful apply so kanshi
//!   watchers see the new state.
//!
//! Headless tests cover the global advertisement + the
//! default-state assertion. Driving an actual configuration apply
//! through `zwlr_output_configuration_v1` is bigger than this
//! round; that's a follow-up that exercises the
//! `apply_output_pending` branch end-to-end.

use super::fixture::Fixture;

#[test]
fn output_management_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwlr_output_manager_v1"),
        "wlr_output_manager_v1 must be advertised; saw {:?}",
        names,
    );
}

#[test]
fn xdg_output_manager_advertises_per_output_metadata() {
    // `zxdg_output_manager_v1` is the read-only side of the
    // output story — gives clients access to logical position +
    // logical size + display name. xdg-desktop-portal-wlr binds
    // it; if we forget to advertise it on a regression, the
    // portal silently skips outputs.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zxdg_output_manager_v1"),
        "zxdg_output_manager_v1 must be advertised; saw {:?}",
        names,
    );
}

#[test]
fn pointer_clamp_survives_extreme_output_geometry() {
    // An output moved next to i32::MAX made `x + width` overflow (a panic:
    // release builds keep overflow checks), and a zero-sized output made
    // `f64::clamp(min, max)` panic with min > max.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let state = &mut fx.server.state;

    state.monitors[0].monitor_area.x = i32::MAX - 10;
    state.monitors[0].monitor_area.y = i32::MAX - 10;
    state.clamp_pointer_to_outputs();
    assert!(state.input_pointer.x.is_finite() && state.input_pointer.y.is_finite());

    state.monitors[0].monitor_area.x = 0;
    state.monitors[0].monitor_area.y = 0;
    state.monitors[0].monitor_area.width = 0;
    state.monitors[0].monitor_area.height = 0;
    state.input_pointer.x = 500.0;
    state.input_pointer.y = 500.0;
    state.clamp_pointer_to_outputs();
    assert_eq!((state.input_pointer.x, state.input_pointer.y), (0.0, 0.0));
}

#[test]
fn state_snapshot_reports_live_logical_output_size() {
    // `width`/`height` are the mode size; on a scaled/rotated output the
    // layout-space size differs and must be reported separately, from the
    // live output state (a runtime scale change included).
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1600));
    let output = fx.server.state.monitors[0].output.clone();
    output.change_current_state(
        None,
        Some(smithay::utils::Transform::_90),
        Some(smithay::output::Scale::Fractional(2.0)),
        None,
    );
    fx.server.state.space.map_output(&output, (0, 0));

    let snapshot = fx.server.state.build_state_snapshot();
    let o = &snapshot["outputs"][0];
    assert_eq!(o["width"], 2560, "mode width stays physical");
    assert_eq!(o["height"], 1600);
    assert_eq!(o["logical_width"], 800, "rotated, then / scale");
    assert_eq!(o["logical_height"], 1280);
    assert_eq!(o["scale"], 2.0, "the live scale, not the configured one");
}
