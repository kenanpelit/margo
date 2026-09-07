//! `mosaic` layout — content-aware auto-float / re-pack reconciliation.
//!
//! Mirrors `floating_layout.rs`'s structure and fixture usage closely
//! (both auto-float via `is_floating`/`float_geom`, both dissolve tabbed
//! groups, both leave hand-floated clients alone), plus the two properties
//! specific to Mosaic: repeated packing passes must not ratchet a window
//! smaller (`mosaic_ideal_width`/`height` exist precisely to prevent that),
//! and switching a tag between `Floating` and `Mosaic` must not let either
//! reconcile pass reclaim/un-float the other's clients (`auto_float_owner`).

use super::client::ClientId;
use super::fixture::Fixture;
use crate::layout::LayoutId;

fn map_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

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
fn mosaic_layout_auto_floats_every_tiled_client() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    assert!(fx.server.state.clients.iter().all(|c| !c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(c.is_floating, "client not auto-floated under Mosaic");
        assert!(c.floated_by_layout, "floated_by_layout flag not set");
        assert_eq!(c.auto_float_owner, Some(LayoutId::Mosaic));
        assert!(c.float_geom.width > 0 && c.float_geom.height > 0);
        assert_eq!(c.geom, c.float_geom, "geom not applied from float_geom");
    }
}

#[test]
fn mosaic_layout_dissolves_tabbed_groups() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    let w0 = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(crate::state::FocusTarget::Window(w0)));
    fx.server.state.toggle_group();
    assert!(fx.server.state.clients[0].group_id.is_some());

    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "mosaic layout must dissolve tabbed groups"
    );
}

#[test]
fn switching_away_from_mosaic_re_tiles_auto_floated_clients() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    assert!(fx.server.state.clients.iter().all(|c| c.is_floating));

    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);

    for c in &fx.server.state.clients {
        assert!(!c.is_floating, "auto-floated client not re-tiled");
        assert!(!c.floated_by_layout);
        assert_eq!(c.auto_float_owner, None);
        assert_eq!(
            c.mosaic_ideal_width, 0,
            "ideal size must not survive leaving the Mosaic tag"
        );
    }
}

#[test]
fn hand_floated_client_survives_switch_to_mosaic() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // User floats window 0 by hand (togglefloating semantics) before this
    // tag is ever Mosaic.
    fx.server.state.clients[0].is_floating = true;
    fx.server.state.clients[0].floated_by_layout = false;
    let hand_geom = fx.server.state.clients[0].float_geom;

    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    assert!(fx.server.state.clients[0].is_floating, "hand-float lost");
    assert!(!fx.server.state.clients[0].floated_by_layout);
    assert_eq!(
        fx.server.state.clients[0].float_geom, hand_geom,
        "mosaic repacked a window the user placed by hand"
    );
    assert!(fx.server.state.clients[1].floated_by_layout);
    assert!(fx.server.state.clients[2].floated_by_layout);
}

#[test]
fn reconcile_is_idempotent_and_does_not_ratchet_size() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    let first: Vec<_> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| c.float_geom)
        .collect();

    // Several more passes with nothing else changing must be a no-op —
    // in particular, must not shrink anything further (the bug
    // `mosaic_ideal_width`/`height` exist to prevent: re-reading a
    // possibly-already-shrunk `float_geom` back as next pass's "ideal").
    for _ in 0..3 {
        fx.server.state.arrange_monitor(0);
    }
    let after: Vec<_> = fx
        .server
        .state
        .clients
        .iter()
        .map(|c| c.float_geom)
        .collect();

    assert_eq!(first, after, "repeated arrange passes drifted the packing");
}

#[test]
fn switching_between_floating_and_mosaic_does_not_corrupt_ownership() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    for c in &fx.server.state.clients {
        assert!(c.is_floating);
        assert_eq!(c.auto_float_owner, Some(LayoutId::Floating));
    }

    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    for c in &fx.server.state.clients {
        assert!(
            c.is_floating,
            "mosaic's reconcile must not have been skipped/blocked by \
             Floating's leftover ownership"
        );
        assert_eq!(
            c.auto_float_owner,
            Some(LayoutId::Mosaic),
            "ownership must transfer to Mosaic, not stay stuck on Floating"
        );
        assert!(c.floated_by_layout);
    }

    pin_layout(&mut fx, 1, LayoutId::Floating);
    fx.server.state.arrange_monitor(0);
    for c in &fx.server.state.clients {
        assert!(c.is_floating);
        assert_eq!(
            c.auto_float_owner,
            Some(LayoutId::Floating),
            "ownership must transfer back to Floating"
        );
    }

    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);
    for c in &fx.server.state.clients {
        assert!(
            !c.is_floating,
            "leaving both auto-float layouts must re-tile every client"
        );
        assert!(!c.floated_by_layout);
        assert_eq!(c.auto_float_owner, None);
    }
}

#[test]
fn opening_a_fourth_window_repacks_without_orphaning_geometry() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    let _fourth = map_window(&mut fx);
    fx.server.state.arrange_monitor(0);

    assert_eq!(fx.server.state.clients.len(), 4);
    for c in &fx.server.state.clients {
        assert!(c.is_floating);
        assert_eq!(c.auto_float_owner, Some(LayoutId::Mosaic));
        assert!(
            c.float_geom.width > 0 && c.float_geom.height > 0,
            "every client must keep valid geometry after a new window joins"
        );
    }
}
