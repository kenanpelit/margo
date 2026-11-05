//! `zwlr_output_configuration_v1` `test` / `apply` with outputs that are
//! off or mirrored — what Settings → Monitors (`wlr-randr --dryrun`, then
//! `wlr-randr`) sends. wlr-randr puts every head it read as off into the
//! configuration with `disable_head`, so an output being off (or a mirror
//! target, which reads as off) must not fail the configuration.

use smithay::reexports::wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1,
    zwlr_output_head_v1::AdaptiveSyncState, zwlr_output_manager_v1::ZwlrOutputManagerV1,
};

use super::client::ClientId;
use super::fixture::Fixture;
use super::wlr_output_client::ConfigOutcome;

/// What a configuration says about one head.
enum Want {
    /// `disable_head`.
    Off,
    /// `enable_head`, optionally with `set_position`.
    On(Option<(i32, i32)>),
}

/// Laptop panel + monitor, as the owner's desk; a wlr-output-management
/// client that has read the heads.
fn desk() -> (Fixture, ClientId, ZwlrOutputManagerV1) {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (1920, 1200));
    fx.add_output("DP-3", (2560, 1440));
    fx.server.state.publish_output_topology();
    let id = fx.add_client();
    fx.roundtrip(id);
    let manager = fx.client(id).bind_output_manager();
    fx.roundtrip(id);
    (fx, id, manager)
}

/// The client re-reads the topology after the compositor changed it, as a
/// fresh `wlr-randr` run would.
fn reread(fx: &mut Fixture, id: ClientId) {
    fx.server.state.publish_output_topology();
    fx.roundtrip(id);
}

fn idx(fx: &Fixture, name: &str) -> usize {
    fx.server
        .state
        .monitors
        .iter()
        .position(|m| m.name == name)
        .unwrap()
}

fn enabled(fx: &Fixture, name: &str) -> bool {
    fx.server.state.monitors[idx(fx, name)].enabled
}

/// Build a configuration from `heads`, send `test` (`dry_run`) or `apply`,
/// and return the reply.
fn configure(
    fx: &mut Fixture,
    id: ClientId,
    manager: &ZwlrOutputManagerV1,
    heads: &[(&str, Want)],
    dry_run: bool,
) -> Option<ConfigOutcome> {
    let cfg = build(fx, id, manager, heads, |_| {});
    send(fx, id, &cfg, dry_run)
}

fn build(
    fx: &mut Fixture,
    id: ClientId,
    manager: &ZwlrOutputManagerV1,
    heads: &[(&str, Want)],
    extra: impl Fn(&smithay::reexports::wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1),
) -> ZwlrOutputConfigurationV1 {
    let c = fx.client(id);
    let serial = c.state.output_management.serial.unwrap();
    let cfg = manager.create_configuration(serial, &c.qh, ());
    for (name, want) in heads {
        let head = c.state.output_management.head(name);
        match want {
            Want::Off => cfg.disable_head(&head),
            Want::On(pos) => {
                let h = cfg.enable_head(&head, &c.qh, ());
                if let Some((x, y)) = pos {
                    h.set_position(*x, *y);
                }
                extra(&h);
            }
        }
    }
    cfg
}

fn send(
    fx: &mut Fixture,
    id: ClientId,
    cfg: &ZwlrOutputConfigurationV1,
    dry_run: bool,
) -> Option<ConfigOutcome> {
    if dry_run {
        cfg.test();
    } else {
        cfg.apply();
    }
    fx.client(id).flush();
    fx.roundtrip(id);
    fx.client(id).state.output_management.outcome(cfg)
}

#[test]
fn a_configuration_with_an_off_head_passes_the_test() {
    // Settings → Monitors with the laptop panel switched off: the dry run
    // carries `disable_head(eDP-1)` and failed, so nothing could be applied.
    let (mut fx, id, manager) = desk();
    let edp = idx(&fx, "eDP-1");
    fx.server.state.disable_monitor(edp);
    reread(&mut fx, id);

    let heads = [("eDP-1", Want::Off), ("DP-3", Want::On(Some((0, 0))))];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Succeeded)
    );
    assert!(!enabled(&fx, "eDP-1"), "a test changes nothing");
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
}

#[test]
fn switching_an_output_off_passes_the_test_and_applies() {
    let (mut fx, id, manager) = desk();
    let heads = [("eDP-1", Want::Off), ("DP-3", Want::On(None))];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Succeeded)
    );
    assert!(enabled(&fx, "eDP-1"), "a test changes nothing");
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
    assert!(!enabled(&fx, "eDP-1"));
    assert!(enabled(&fx, "DP-3"));
}

#[test]
fn a_configuration_that_leaves_every_output_off_fails_test_and_apply() {
    let (mut fx, id, manager) = desk();
    let edp = idx(&fx, "eDP-1");
    fx.server.state.disable_monitor(edp);
    reread(&mut fx, id);
    let heads = [("eDP-1", Want::Off), ("DP-3", Want::Off)];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Failed)
    );
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Failed)
    );
    assert!(enabled(&fx, "DP-3"), "the last output stays on");
}

#[test]
fn swapping_which_output_is_on_succeeds() {
    // One apply: eDP-1 on → off and DP-3 off → on. The guard counted the
    // outputs on *before* the configuration and refused it.
    let (mut fx, id, manager) = desk();
    let dp3 = idx(&fx, "DP-3");
    fx.server.state.disable_monitor(dp3);
    reread(&mut fx, id);
    let heads = [("eDP-1", Want::Off), ("DP-3", Want::On(None))];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Succeeded)
    );
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
    assert!(!enabled(&fx, "eDP-1"));
    assert!(enabled(&fx, "DP-3"));
}

#[test]
fn moving_an_output_while_mirroring_passes_the_test_and_keeps_the_mirror() {
    // The mirror target reads as off, so wlr-randr sends `disable_head`
    // for it with every configuration.
    let (mut fx, id, manager) = desk();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    reread(&mut fx, id);
    let heads = [("eDP-1", Want::On(Some((100, 0)))), ("DP-3", Want::Off)];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Succeeded)
    );
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
    let st = &fx.server.state;
    assert!(st.mirror.is_target("DP-3"), "still mirroring");
    assert_eq!(st.monitors[idx(&fx, "eDP-1")].monitor_area.x, 100);
}

#[test]
fn switching_the_mirror_source_off_ends_the_mirror_and_succeeds() {
    // Like `disable_output eDP-1`: the mirror stops, DP-3 is the desktop
    // again and eDP-1 goes off. The guard counted DP-3 as off and refused.
    let (mut fx, id, manager) = desk();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    reread(&mut fx, id);
    let heads = [("eDP-1", Want::Off), ("DP-3", Want::Off)];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, true),
        Some(ConfigOutcome::Succeeded)
    );
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
    let st = &fx.server.state;
    assert!(st.mirror.active().is_none());
    assert!(!enabled(&fx, "eDP-1"));
    assert!(enabled(&fx, "DP-3"));
}

#[test]
fn switching_the_mirror_target_on_ends_the_mirror_and_places_it() {
    let (mut fx, id, manager) = desk();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    reread(&mut fx, id);
    let heads = [
        ("eDP-1", Want::On(None)),
        ("DP-3", Want::On(Some((1920, 0)))),
    ];
    assert_eq!(
        configure(&mut fx, id, &manager, &heads, false),
        Some(ConfigOutcome::Succeeded)
    );
    let st = &fx.server.state;
    assert!(st.mirror.active().is_none());
    assert!(enabled(&fx, "DP-3"));
    assert_eq!(st.monitors[idx(&fx, "DP-3")].monitor_area.x, 1920);
}

#[test]
fn variable_refresh_rate_is_refused_not_ignored() {
    // margo has no VRR: `set_adaptive_sync(enabled)` was dropped and the
    // configuration still answered `succeeded`. `disabled` is what every
    // output already does.
    let (mut fx, id, manager) = desk();
    for (state, dry_run, want) in [
        (AdaptiveSyncState::Enabled, true, ConfigOutcome::Failed),
        (AdaptiveSyncState::Enabled, false, ConfigOutcome::Failed),
        (AdaptiveSyncState::Disabled, true, ConfigOutcome::Succeeded),
        (AdaptiveSyncState::Disabled, false, ConfigOutcome::Succeeded),
    ] {
        let heads = [("eDP-1", Want::On(None)), ("DP-3", Want::On(None))];
        let cfg = build(&mut fx, id, &manager, &heads, |h| {
            h.set_adaptive_sync(state)
        });
        assert_eq!(
            send(&mut fx, id, &cfg, dry_run),
            Some(want),
            "{state:?} dry_run={dry_run}"
        );
    }
}

/// Settings → Monitors end to end: with DP-3 off, the page moves eDP-1.
/// It runs `wlr-randr --dryrun <args>` then `wlr-randr <args>`, where the
/// args give every head: `--off` for one that is off, else `--on --mode
/// --pos --transform --scale --adaptive-sync disabled`.
#[test]
fn the_monitors_page_applies_with_an_output_off() {
    let (mut fx, id, manager) = desk();
    fx.server.state.output_pending.mode_changes_supported = true;
    let dp3 = idx(&fx, "DP-3");
    fx.server.state.disable_monitor(dp3);
    reread(&mut fx, id);

    let om = &fx.client(id).state.output_management;
    let mode = om.mode(&om.head("eDP-1"), (1920, 1200, 60_000));
    for dry_run in [true, false] {
        let heads = [("eDP-1", Want::On(Some((0, 40)))), ("DP-3", Want::Off)];
        let cfg = build(&mut fx, id, &manager, &heads, |h| {
            h.set_mode(&mode);
            h.set_transform(wayland_client::protocol::wl_output::Transform::Normal);
            h.set_scale(1.0);
            h.set_adaptive_sync(AdaptiveSyncState::Disabled);
        });
        let outcome = send(&mut fx, id, &cfg, dry_run);
        if dry_run {
            assert_eq!(outcome, Some(ConfigOutcome::Succeeded), "dry run");
        } else {
            // The reply waits for the backend to set the mode.
            assert_eq!(outcome, None);
            for change in std::mem::take(&mut fx.server.state.output_pending.mode_changes) {
                change.reply.finish(true);
            }
            fx.roundtrip(id);
            assert_eq!(
                fx.client(id).state.output_management.outcome(&cfg),
                Some(ConfigOutcome::Succeeded),
                "apply"
            );
        }
    }
    assert_eq!(
        fx.server.state.monitors[idx(&fx, "eDP-1")].monitor_area.y,
        40
    );
    assert!(!enabled(&fx, "DP-3"));
}
