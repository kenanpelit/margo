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

/// `n` mapped windows on one 1080p output — for the overflow tests, which
/// need more windows than `three_windows` gives.
fn n_windows(fx: &mut Fixture, n: usize) -> Vec<ClientId> {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    (0..n).map(|_| map_window(fx)).collect()
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

// ── Phase 2: drag-tile-to-tile extended to Mosaic ───────────────────────────

#[test]
fn interactive_grab_excludes_a_client_from_mosaic_repacking() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    // Simulate an in-progress drag the way `MoveSurfaceGrab::motion` drives
    // it: move the window and flag it grabbed, *without* going through the
    // real pointer grab machinery.
    fx.server.state.clients[0].interactive_grab = true;
    let dragged_to = crate::layout::Rect::new(900, 700, 250, 180);
    fx.server.state.clients[0].float_geom = dragged_to;
    fx.server.state.clients[0].geom = dragged_to;

    fx.server.state.arrange_monitor(0);

    assert_eq!(
        fx.server.state.clients[0].float_geom, dragged_to,
        "reconcile_mosaic_layout repacked a client mid-drag"
    );
}

// `resolve_drag_tile_drop`'s swap-target *decision* (same-kind check) is
// covered directly, without a Space/pointer fixture, by
// `input::grabs::drag_tile_target_tests` — the compositor's real hit-
// testing needs a client with an actually-committed buffer, which this
// harness's synthetic toplevels don't have.

#[test]
fn dropping_a_mosaic_client_on_empty_space_stays_floating_and_owned() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);

    let dragged_window = fx.server.state.clients[0].window.clone();
    let original_float_geom = fx.server.state.clients[0].float_geom;

    // Cursor far off in empty space — no client under it.
    fx.server.state.input_pointer.x = -5000.0;
    fx.server.state.input_pointer.y = -5000.0;

    crate::input::grabs::resolve_drag_tile_drop(
        &mut fx.server.state,
        &dragged_window,
        original_float_geom,
    );

    assert!(fx.server.state.clients[0].is_floating);
    assert_eq!(
        fx.server.state.clients[0].auto_float_owner,
        Some(LayoutId::Mosaic)
    );
}

#[test]
fn dropping_a_classically_tiled_client_restores_original_float_geom_when_no_target() {
    // For a classic-tile source, the trailing `arrange_monitor` recomputes
    // *tiled* `geom` via the normal grid algorithms, which never touch
    // `float_geom` — so the restored value sticks. (Unlike Mosaic, whose
    // trailing repack overwrites `float_geom` again regardless of what
    // `resolve_drag_tile_drop` just wrote — see
    // `dropping_a_mosaic_client_on_empty_space_stays_floating_and_owned`
    // above, which asserts what actually holds for that case.)
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    pin_layout(&mut fx, 1, LayoutId::Tile);
    fx.server.state.arrange_monitor(0);
    assert!(!fx.server.state.clients[0].is_floating);

    let dragged_window = fx.server.state.clients[0].window.clone();
    let original_float_geom = crate::layout::Rect::new(42, 42, 111, 222);

    fx.server.state.input_pointer.x = -5000.0;
    fx.server.state.input_pointer.y = -5000.0;
    crate::input::grabs::resolve_drag_tile_drop(
        &mut fx.server.state,
        &dragged_window,
        original_float_geom,
    );

    assert_eq!(fx.server.state.clients[0].float_geom, original_float_geom);
    assert!(!fx.server.state.clients[0].is_floating);
}

// ── Phase 3: auto-move to an empty tag on genuine overflow ──────────────────

/// Force every client's own floor (`min_height`) high enough that ten of
/// them genuinely cannot fit a 1080p output no matter how much
/// `mosaic_arrange` shrinks toward it. `n_windows` maps clients in order
/// and nothing before this point reorders `self.clients`, so positional
/// indexing lines up 1:1 with the returned `ClientId`s (same convention
/// every other test in this file/`floating_layout.rs` already relies on).
fn force_overflow(fx: &mut Fixture, count: usize) {
    // `pack_rows` caps each client's width toward roughly sqrt(count)
    // columns (see its doc comment), so 10 clients land in ~3 rows, not
    // one-per-row — min_height alone has to clear each row's floor
    // (200 wasn't enough once that cap landed; 400 comfortably is).
    for i in 0..count {
        fx.server.state.clients[i].min_height = 400;
    }
}

#[test]
fn mosaic_moves_the_newest_overflowing_client_to_an_empty_tag() {
    let mut fx = Fixture::new();
    let windows = n_windows(&mut fx, 10);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    force_overflow(&mut fx, windows.len());

    // The client created *last* has the highest `id` — it's the one that
    // must get moved.
    let newest_window = fx.server.state.clients.last().unwrap().window.clone();
    let newest_id = fx.server.state.clients.last().unwrap().id;

    fx.server.state.arrange_monitor(0);

    let newest = fx
        .server
        .state
        .clients
        .iter()
        .find(|c| c.window == newest_window)
        .expect("newest client still exists");
    assert_eq!(newest.id, newest_id);
    assert_eq!(
        newest.tags,
        1u32 << 1,
        "newest client should have moved to tag 2 (the first empty tag)"
    );

    let remaining_on_tag1 = fx
        .server
        .state
        .clients
        .iter()
        .filter(|c| c.tags & 1 != 0)
        .count();
    assert_eq!(
        remaining_on_tag1, 9,
        "exactly the evicted client should leave tag 1"
    );
}

#[test]
fn mosaic_does_not_evict_when_the_config_flag_is_off() {
    let mut fx = Fixture::new();
    let windows = n_windows(&mut fx, 10);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    force_overflow(&mut fx, windows.len());
    fx.server.state.config.mosaic_auto_overflow_tag = false;

    fx.server.state.arrange_monitor(0);

    let remaining_on_tag1 = fx
        .server
        .state
        .clients
        .iter()
        .filter(|c| c.tags & 1 != 0)
        .count();
    assert_eq!(
        remaining_on_tag1, 10,
        "nobody should be evicted while the config flag is off"
    );
}

#[test]
fn mosaic_does_not_evict_when_every_tag_is_already_occupied() {
    let mut fx = Fixture::new();
    let windows = n_windows(&mut fx, 10);
    pin_layout(&mut fx, 1, LayoutId::Mosaic);
    fx.server.state.arrange_monitor(0);
    force_overflow(&mut fx, windows.len());

    // Occupy every other tag (2..=9) with one of the ten windows each, so
    // `find_empty_tag` has nowhere left to send the overflow to. Leave at
    // least two on tag 1 so it's still the one that's overflowing.
    let _ = &windows; // only the count mattered — see force_overflow above
    for i in 0..8 {
        fx.server.state.clients[i].tags = 1u32 << (i + 1); // tags 2..=9
    }

    fx.server.state.arrange_monitor(0);

    let untouched_tags: Vec<u32> = fx.server.state.clients.iter().map(|c| c.tags).collect();
    // Re-arranging again must be a no-op: nothing left to evict *to*.
    fx.server.state.arrange_monitor(0);
    let after: Vec<u32> = fx.server.state.clients.iter().map(|c| c.tags).collect();
    assert_eq!(
        untouched_tags, after,
        "no empty tag exists — nothing should move"
    );
}
