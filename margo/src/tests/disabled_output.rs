//! A soft-disabled output (`wlr-randr --output X --off`, a kanshi
//! `disable` profile) must go dark and drop out of everything that picks
//! an output: pointer hit-testing and clamping, the new-window / layer
//! fallback monitor, `focusmon` / `tagmon` cycling and the space.

use super::client::ClientId;
use super::fixture::Fixture;

fn centre_of(fx: &Fixture, mon: usize) -> (f64, f64) {
    let a = fx.server.state.monitors[mon].monitor_area;
    ((a.x + a.width / 2) as f64, (a.y + a.height / 2) as f64)
}

fn park_pointer_on(fx: &mut Fixture, mon: usize) {
    let (x, y) = centre_of(fx, mon);
    fx.server.state.input_pointer.x = x;
    fx.server.state.input_pointer.y = y;
}

fn three_outputs() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.add_output("DP-3", (1920, 1080));
    fx
}

fn map_focused_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

#[test]
fn disabling_an_output_blanks_its_panel_and_enabling_lights_it_again() {
    let mut fx = three_outputs();
    let dp2 = fx.server.state.monitors[1].output.clone();

    fx.server.state.disable_monitor(1);
    assert!(
        fx.server
            .state
            .output_pending
            .dpms
            .iter()
            .any(|(o, on)| *o == dp2 && !on),
        "the disabled panel must be cleared, not left showing its last frame"
    );
    assert!(
        !fx.server.state.space.outputs().any(|o| *o == dp2),
        "a disabled output leaves the space"
    );

    fx.server.state.output_pending.dpms.clear();
    fx.server.state.enable_monitor(1);
    assert!(
        fx.server
            .state
            .output_pending
            .dpms
            .iter()
            .any(|(o, on)| *o == dp2 && *on)
    );
    let geo = fx
        .server
        .state
        .space
        .output_geometry(&dp2)
        .expect("mapped again");
    assert_eq!(geo.loc.x, fx.server.state.monitors[1].monitor_area.x);
}

#[test]
fn the_pointer_can_not_stay_on_a_disabled_output() {
    let mut fx = three_outputs();
    park_pointer_on(&mut fx, 1);

    fx.server.state.disable_monitor(1);

    let mon = fx.server.state.pointer_monitor();
    assert!(
        mon.is_some_and(|m| m != 1),
        "pointer must be moved onto an enabled output, got {mon:?}"
    );

    // Motion back into the disabled area is clamped out of it again.
    park_pointer_on(&mut fx, 1);
    assert_eq!(fx.server.state.pointer_monitor(), None);
    fx.server.state.clamp_pointer_to_outputs();
    let mon = fx.server.state.pointer_monitor();
    assert!(mon.is_some_and(|m| m != 1), "clamped to {mon:?}");
}

#[test]
fn new_windows_never_land_on_a_disabled_output() {
    let mut fx = three_outputs();
    fx.add_keyboard();
    fx.server.state.disable_monitor(0);
    // Pointer resting on the disabled area, no focused client.
    park_pointer_on(&mut fx, 0);

    assert_ne!(fx.server.state.focused_monitor(), 0);
    map_focused_window(&mut fx);
    assert_ne!(fx.server.state.clients[0].monitor, 0);
}

#[test]
fn focusmon_and_tagmon_skip_a_disabled_output() {
    let mut fx = three_outputs();
    fx.add_keyboard();
    map_focused_window(&mut fx);
    assert_eq!(fx.server.state.clients[0].monitor, 0);
    fx.server.state.disable_monitor(1);

    fx.server.state.tag_mon(1);
    assert_eq!(
        fx.server.state.clients[0].monitor, 2,
        "tagmon must step over the disabled DP-2"
    );

    fx.server.state.tag_mon(-1);
    assert_eq!(fx.server.state.clients[0].monitor, 0);

    let mut fx = three_outputs();
    fx.server.state.disable_monitor(1);
    park_pointer_on(&mut fx, 0);
    fx.server.state.focus_mon(1);
    assert_eq!(fx.server.state.pointer_monitor(), Some(2));
}

#[test]
fn a_tag_pinned_to_a_disabled_output_has_no_home_there() {
    let mut config = margo_config::Config::default();
    config.tag_rules.push(margo_config::TagRule {
        id: 2,
        monitor_name: Some("DP-2".into()),
        ..Default::default()
    });
    let mut fx = Fixture::with_config(config);
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    assert_eq!(fx.server.state.tag_home_monitor(0b10), Some(1));

    fx.server.state.disable_monitor(1);
    assert_eq!(fx.server.state.tag_home_monitor(0b10), None);
}

/// Map a window on monitor `mon` (new windows open on the monitor under
/// the pointer when nothing is focused there); it takes focus. Returns
/// its client index.
fn map_window_on(fx: &mut Fixture, mon: usize) -> usize {
    fx.server.state.focus_surface(None);
    park_pointer_on(fx, mon);
    map_focused_window(fx);
    let idx = fx.server.state.clients.len() - 1;
    assert_eq!(fx.server.state.clients[idx].monitor, mon);
    idx
}

fn focused(fx: &Fixture) -> Option<usize> {
    fx.server.state.focused_client_idx()
}

#[test]
fn taking_out_an_unfocused_output_leaves_focus_where_you_type() {
    // You type on DP-3 while DP-2 is switched off (or becomes a mirror
    // target): focus used to jump to the migration target, DP-1.
    for mirror in [false, true] {
        let mut fx = three_outputs();
        fx.add_keyboard();
        let on_dp1 = map_window_on(&mut fx, 0);
        map_window_on(&mut fx, 1);
        let on_dp3 = map_window_on(&mut fx, 2);
        assert_eq!(focused(&fx), Some(on_dp3));

        if mirror {
            fx.server
                .state
                .start_mirror(Some("DP-1"), Some("DP-2"))
                .unwrap();
        } else {
            fx.server.state.disable_monitor(1);
        }
        assert_eq!(focused(&fx), Some(on_dp3), "mirror={mirror}");
        assert_eq!(
            fx.server.state.monitors[0].selected,
            Some(on_dp1),
            "mirror={mirror}: DP-1 remembers its window"
        );
    }
}

#[test]
fn taking_out_the_focused_output_moves_focus_and_keeps_other_selections() {
    let mut fx = three_outputs();
    fx.add_keyboard();
    let on_dp3 = map_window_on(&mut fx, 2);
    let on_dp2 = map_window_on(&mut fx, 1);
    assert_eq!(focused(&fx), Some(on_dp2));

    fx.server.state.disable_monitor(1);
    // DP-2's window moved to DP-1 and keeps focus there.
    assert_eq!(fx.server.state.clients[on_dp2].monitor, 0);
    assert_eq!(focused(&fx), Some(on_dp2));
    assert_eq!(
        fx.server.state.monitors[2].selected,
        Some(on_dp3),
        "DP-3's selection is not wiped"
    );
    assert_eq!(fx.server.state.monitors[1].selected, None);
}
