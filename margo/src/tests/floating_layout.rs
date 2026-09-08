//! `floating` layout — auto-float / re-tile reconciliation (issue #1).
//!
//! Drives real xdg_toplevels through the fixture so the windows live in
//! `state.clients`, are mapped, and past their deferred initial map —
//! then flips the current tag's layout and asserts what
//! `reconcile_floating_layout` (run inside `arrange_monitor`) does.

use super::client::ClientId;
use super::fixture::Fixture;
use crate::layout::LayoutId;

/// Map one focused toplevel and drive the deferred-map flow to
/// completion (so `is_initial_map_pending` clears).
fn map_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

/// Three mapped windows on one 1080p output.
fn three_windows(fx: &mut Fixture) -> [ClientId; 3] {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    [map_window(fx), map_window(fx), map_window(fx)]
}

fn pin_layout(fx: &mut Fixture, tag: usize, layout: LayoutId) {
    fx.server.state.monitors[0].pertag.ltidxs[tag] = layout;
    fx.server.state.monitors[0].pertag.user_picked_layout[tag] = true;
}

#[test]
fn floating_layout_auto_floats_every_tiled_client() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    assert!(fx.server.state.clients.iter().all(|c| !c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(c.is_floating, "client not auto-floated under Floating");
        assert!(c.floated_by_layout, "floated_by_layout flag not set");
        assert!(c.float_geom.width > 0 && c.float_geom.height > 0);
        assert_eq!(c.geom, c.float_geom, "geom not applied from float_geom");
    }
}

#[test]
fn floating_layout_cascades_placement() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    let mut origins: Vec<(i32, i32)> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| (c.float_geom.x, c.float_geom.y))
        .collect();
    origins.sort();
    origins.dedup();
    assert_eq!(origins.len(), 3, "cascade left windows stacked at one spot");
}

#[test]
fn switching_away_from_floating_re_tiles_auto_floated_clients() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    assert!(fx.server.state.clients.iter().all(|c| c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(!c.is_floating, "auto-floated client not re-tiled");
        assert!(!c.floated_by_layout);
    }
}

#[test]
fn hand_floated_client_survives_switch_to_tiling() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // User floats window 0 by hand (togglefloating semantics).
    fx.server.state.clients[0].is_floating = true;
    fx.server.state.clients[0].floated_by_layout = false;

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);

    assert!(fx.server.state.clients[0].is_floating, "hand-float lost");
    assert!(!fx.server.state.clients[0].floated_by_layout);
    assert!(!fx.server.state.clients[1].is_floating);
    assert!(!fx.server.state.clients[2].is_floating);
}

#[test]
fn reconcile_is_idempotent() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    let first: Vec<_> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| c.float_geom)
        .collect();

    fx.server.state.arrange_monitor(0);
    fx.server.state.arrange_monitor(0);
    let after: Vec<_> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| c.float_geom)
        .collect();

    assert_eq!(first, after, "repeated arrange passes drifted the cascade");
}

#[test]
fn layout_switch_during_toggleview_does_not_leak_into_other_tag_combos() {
    // Reproduces a real user-reported bug: windows overlapping under
    // tile/center_tile/grid after using `toggleview` to look at more
    // than one tag at once, then trying a layout switch.
    //
    // `Pertag::ltidxs` reserves index 0 for Overview
    // (`ltidxs: Vec<LayoutId>` doc: "indexed 0 = overview, 1..=MAXTAGS").
    // `update_pertag_for_tagset` used to collapse `curtag` to 0 for
    // *any* multi-tag view — not just Overview — so a layout switch
    // made while a toggleview combo was showing landed in that one
    // shared slot instead of a real tag. Every *other*, completely
    // unrelated toggleview combo then read the same slot back, so a
    // client on a tag that was never touched by hand ended up
    // auto-floated (and overlapping the tiled windows around it) just
    // because *some* earlier combo had picked mosaic/floating.
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));

    // Window 0 on tag 1, window 1 on tag 2.
    let _w0 = map_window(&mut fx);
    let _w1 = map_window(&mut fx);
    fx.server.state.clients[1].tags = 1 << 1; // tag 2

    // Exactly what `super,1` then `super+ctrl,2,toggleview,2` does:
    // view tag 1 alone, then toggle tag 2 in alongside it.
    fx.server.state.view_tag(1);
    fx.server.state.toggle_view_tag(1 << 1);

    // While that combo is showing, the user tries mosaic (or floating —
    // same bug either way) via a layout-switch bind.
    fx.server.state.set_layout("mosaic");

    // Leave the combo. Window 2 lives on tag 3, which nothing above
    // ever touched — its own layout is whatever `Fixture::new()`
    // defaults to (tile), never mosaic.
    let _w2 = map_window(&mut fx);
    fx.server.state.clients[2].tags = 1 << 2; // tag 3
    fx.server.state.view_tag(1 << 2);
    // A second, disjoint toggleview combo that shares no tag with the
    // first one.
    fx.server.state.toggle_view_tag(1 << 3);

    assert!(
        !fx.server.state.clients[2].is_floating,
        "layout switch made during one toggleview combo leaked into an unrelated combo"
    );
}

#[test]
fn floating_layout_dissolves_tabbed_groups() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // Group windows 0 + 1.
    let w0 = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(crate::state::FocusTarget::Window(w0)));
    fx.server.state.toggle_group();
    assert!(fx.server.state.clients[0].group_id.is_some());

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);

    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "floating layout must dissolve tabbed groups"
    );
}
