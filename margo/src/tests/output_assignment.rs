//! Multi-monitor output-assignment tests (first-wave integration
//! coverage — see `docs/protocol-matrix.md` "next targets").
//!
//! These pin the invariants the udev backend's `setup_connector`
//! path relies on but that no automated test guarded before: when
//! outputs come up they must land left-to-right with non-overlapping
//! geometry, each gets its own `MargoMonitor` slot with independent
//! pertag state, and per-output tag rules are applied to the right
//! monitor. The fixture's `add_output` mirrors that backend layout
//! (cumulative-width placement), so a regression in the placement
//! arithmetic or the per-monitor pertag wiring shows up here instead
//! of as "windows pile up on the wrong screen" on real hardware.

use margo_config::{Config, TagRule};

use super::fixture::Fixture;

/// Three same-width outputs must tile left-to-right at cumulative
/// x-offsets with no overlap — 0, 1920, 3840. This is the exact
/// placement the multi-monitor focus / tag-move math assumes; an
/// off-by-one (e.g. placing at index*width vs cumulative width)
/// would silently break `focus_mon` direction on mixed-width setups.
#[test]
fn outputs_tile_left_to_right_without_overlap() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.add_output("DP-3", (1920, 1080));

    let xs: Vec<i32> = fx
        .server
        .state
        .monitors
        .iter()
        .map(|m| m.monitor_area.x)
        .collect();
    assert_eq!(xs, vec![0, 1920, 3840], "outputs must tile cumulatively");

    // Each output owns a full, non-overlapping 1920-wide slice.
    for m in &fx.server.state.monitors {
        assert_eq!(m.monitor_area.width, 1920);
        assert_eq!(m.monitor_area.height, 1080);
    }
}

/// Mixed-width outputs still tile cumulatively: a 2560 ultrawide
/// followed by a 1920 panel puts the second at x=2560, not x=1920.
/// This is the case the naive `index * width` placement gets wrong.
#[test]
fn mixed_width_outputs_use_cumulative_offsets() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (2560, 1440));
    fx.add_output("DP-2", (1920, 1080));

    assert_eq!(fx.server.state.monitors[0].monitor_area.x, 0);
    assert_eq!(
        fx.server.state.monitors[1].monitor_area.x, 2560,
        "second output must start where the first ends, not at a fixed stride",
    );
}

/// Each output gets its own `MargoMonitor` with an independent
/// pertag snapshot. Mutating one monitor's tagset must not bleed
/// into another — the bug that the overview restore tests guard
/// from the other direction.
#[test]
fn each_output_has_independent_pertag_state() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    fx.server.state.monitors[0].tagset[0] = 0b0000_0001;
    fx.server.state.monitors[1].tagset[0] = 0b0000_1000;

    assert_eq!(fx.server.state.monitors[0].current_tagset(), 0b0000_0001);
    assert_eq!(
        fx.server.state.monitors[1].current_tagset(),
        0b0000_1000,
        "per-monitor tagsets must not share storage",
    );
}

/// A `tagrule` pinning tag 2 to "DP-2" with a custom nmaster must
/// land on DP-2's pertag and leave DP-1 untouched. `add_output`
/// runs `apply_tag_rules_to_monitor` for the new monitor, so this
/// verifies the name-matched rule reaches only the matching output.
#[test]
fn per_output_tag_rule_applies_to_named_monitor_only() {
    let mut config = Config::default();
    config.tag_rules.push(TagRule {
        id: 2,
        monitor_name: Some("DP-2".into()),
        nmaster: 3,
        ..Default::default()
    });

    let mut fx = Fixture::with_config(config);
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));

    // pertag is 1-indexed by tag id; tag 2 → index 2.
    assert_eq!(
        fx.server.state.monitors[1].pertag.nmasters[2], 3,
        "DP-2's tag-2 nmaster must come from the named tagrule",
    );
    assert_ne!(
        fx.server.state.monitors[0].pertag.nmasters[2], 3,
        "DP-1 must be untouched by a DP-2-scoped rule",
    );
}
