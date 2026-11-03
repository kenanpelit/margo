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
    // What every scale / transform / mode path does after changing the
    // output: `monitor_area` follows.
    fx.server.state.refresh_output_work_area(&output);

    let snapshot = fx.server.state.build_state_snapshot();
    let o = &snapshot["outputs"][0];
    assert_eq!(o["width"], 2560, "mode width stays physical");
    assert_eq!(o["height"], 1600);
    assert_eq!(o["logical_width"], 800, "rotated, then / scale");
    assert_eq!(o["logical_height"], 1280);
    assert_eq!(o["scale"], 2.0, "the live scale, not the configured one");
}

// ── SetMode through a real zwlr_output_configuration_v1 ──────────────────────

use smithay::output::Mode as OutputMode;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::client::zwlr_output_manager_v1::ZwlrOutputManagerV1;

use super::client::ClientId;
use super::wlr_output_client::ConfigOutcome;

/// DP-1 (2560×1440@60, plus 1920×1080@120) and DP-2 (1280×1024@60), with
/// a client that has read the heads. `mode_changes` says whether the
/// backend can change modes (udev can; winit / headless can't).
fn wlr_desk(mode_changes: bool) -> (Fixture, ClientId, ZwlrOutputManagerV1) {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1440));
    fx.add_output("DP-2", (1280, 1024));
    fx.server.state.monitors[0].output.add_mode(OutputMode {
        size: (1920, 1080).into(),
        refresh: 120_000,
    });
    fx.server.state.output_pending.mode_changes_supported = mode_changes;
    fx.server.state.publish_output_topology();
    let id = fx.add_client();
    fx.roundtrip(id);
    let manager = fx.client(id).bind_output_manager();
    fx.roundtrip(id);
    (fx, id, manager)
}

/// `enable_head(head)` + `set_mode(mode)` on a fresh configuration, then
/// apply it; returns the outcome.
fn apply_set_mode(
    fx: &mut Fixture,
    id: ClientId,
    manager: &ZwlrOutputManagerV1,
    head_name: &str,
    mode: &smithay::reexports::wayland_protocols_wlr::output_management::v1::client::zwlr_output_mode_v1::ZwlrOutputModeV1,
) -> Option<ConfigOutcome> {
    let c = fx.client(id);
    let serial = c.state.output_management.serial.unwrap();
    let head = c.state.output_management.head(head_name);
    let cfg = manager.create_configuration(serial, &c.qh, ());
    let cfg_head = cfg.enable_head(&head, &c.qh, ());
    cfg_head.set_mode(mode);
    cfg.apply();
    c.flush();
    fx.roundtrip(id);
    fx.client(id).state.output_management.outcome(&cfg)
}

fn queued_modes(fx: &Fixture) -> Vec<(String, i32, i32, i32)> {
    fx.server
        .state
        .output_pending
        .mode_changes
        .iter()
        .map(|m| (m.output_name.clone(), m.width, m.height, m.refresh_mhz))
        .collect()
}

#[test]
fn set_mode_queues_the_advertised_mode_the_client_picked() {
    let (mut fx, id, manager) = wlr_desk(true);
    let om = &fx.client(id).state.output_management;
    let mode = om.mode(&om.head("DP-1"), (1920, 1080, 120_000));
    let outcome = apply_set_mode(&mut fx, id, &manager, "DP-1", &mode);
    assert_eq!(outcome, Some(ConfigOutcome::Succeeded));
    assert_eq!(
        queued_modes(&fx),
        vec![("DP-1".to_string(), 1920, 1080, 120_000)]
    );
}

#[test]
fn set_mode_with_another_heads_mode_fails() {
    let (mut fx, id, manager) = wlr_desk(true);
    let om = &fx.client(id).state.output_management;
    let dp2_mode = om.mode(&om.head("DP-2"), (1280, 1024, 60_000));
    let outcome = apply_set_mode(&mut fx, id, &manager, "DP-1", &dp2_mode);
    assert_eq!(outcome, Some(ConfigOutcome::Failed));
    assert!(queued_modes(&fx).is_empty());
}

#[test]
fn set_mode_with_a_stale_mode_fails() {
    let (mut fx, id, manager) = wlr_desk(true);
    let c = fx.client(id);
    let om = &c.state.output_management;
    let head = om.head("DP-1");
    let stale = om.mode(&head, (1920, 1080, 120_000));
    let serial = om.serial.unwrap();
    let cfg = manager.create_configuration(serial, &c.qh, ());
    let cfg_head = cfg.enable_head(&head, &c.qh, ());
    c.flush();
    fx.roundtrip(id);
    // The topology changes under the configuration: every head and mode
    // is re-announced and the old mode objects are finished.
    fx.server.state.monitors[0].output.add_mode(OutputMode {
        size: (1280, 720).into(),
        refresh: 60_000,
    });
    fx.server.state.publish_output_topology();
    fx.roundtrip(id);

    let c = fx.client(id);
    cfg_head.set_mode(&stale);
    cfg.apply();
    c.flush();
    fx.roundtrip(id);
    assert_eq!(
        fx.client(id).state.output_management.outcome(&cfg),
        Some(ConfigOutcome::Failed)
    );
    assert!(queued_modes(&fx).is_empty());
}

#[test]
fn set_mode_fails_when_the_backend_cannot_change_modes() {
    // winit / headless: nothing drains `mode_changes`.
    let (mut fx, id, manager) = wlr_desk(false);
    let om = &fx.client(id).state.output_management;
    let mode = om.mode(&om.head("DP-1"), (1920, 1080, 120_000));
    let outcome = apply_set_mode(&mut fx, id, &manager, "DP-1", &mode);
    assert_eq!(outcome, Some(ConfigOutcome::Failed));
    assert!(queued_modes(&fx).is_empty());
}

#[test]
fn custom_modes_are_refused() {
    let (mut fx, id, manager) = wlr_desk(true);
    let c = fx.client(id);
    let serial = c.state.output_management.serial.unwrap();
    let head = c.state.output_management.head("DP-1");
    let cfg = manager.create_configuration(serial, &c.qh, ());
    let cfg_head = cfg.enable_head(&head, &c.qh, ());
    cfg_head.set_custom_mode(1920, 1080, 120_000);
    cfg_head.set_scale(2.0);
    cfg.apply();
    c.flush();
    fx.roundtrip(id);
    assert_eq!(
        fx.client(id).state.output_management.outcome(&cfg),
        Some(ConfigOutcome::Failed)
    );
    assert!(queued_modes(&fx).is_empty());
}
