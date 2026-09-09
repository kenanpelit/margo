//! `focused_tiled_pos` — the anchor slot focus-following layouts
//! (scroller, vertical scroller, deck) centre on.
//!
//! Locks the fix for "a floating keyring / polkit dialog grabbing focus
//! snaps the scroller strip to window 1". The naive lookup returns `None`
//! when focus is off the tiled strip, and every consumer then falls back
//! to slot 0; `focused_tiled_pos` instead holds the strip on the
//! most-recently-focused window that is still tiled (per-monitor MRU
//! `focus_history`, most-recent first).

use margo_config::Config;

use super::fixture::Fixture;

/// Two mapped clients on one 1080p output. Returns the fixture and the two
/// windows' stable `MargoClient::id`s (clients land at state indices 0, 1).
fn two_clients() -> (Fixture, [u64; 2]) {
    let mut fx = Fixture::with_config(Config::default());
    fx.add_output("DP-1", (1920, 1080));
    for (app, title) in [("kitty", "one"), ("kitty", "two")] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    let ids = [fx.server.state.clients[0].id, fx.server.state.clients[1].id];
    (fx, ids)
}

#[test]
fn focused_tiled_window_uses_its_own_slot() {
    // When the focused window IS in the tiled strip, its own slot wins —
    // history is never consulted (no behaviour change from the old lookup).
    let (fx, _ids) = two_clients();
    let tiled = [0usize, 1];
    assert_eq!(
        fx.server.state.focused_tiled_pos(0, &tiled, Some(1)),
        Some(1)
    );
    assert_eq!(
        fx.server.state.focused_tiled_pos(0, &tiled, Some(0)),
        Some(0)
    );
}

#[test]
fn floating_focus_holds_last_tiled_position() {
    // Window 2 (idx 1) was focused, then a floating dialog (not in `tiled`)
    // grabbed focus and was pushed to the MRU front. The strip must stay on
    // window 2, not snap to slot 0.
    let (mut fx, ids) = two_clients();
    let tiled = [0usize, 1];
    let dialog_id = 9_999u64;
    let hist = &mut fx.server.state.monitors[0].focus_history;
    hist.clear();
    hist.push_front(ids[1]); // window 2, previously focused
    hist.push_front(dialog_id); // floating dialog, now focused (front)

    // Focus is on a client that isn't in the tiled strip (stand-in idx 99)…
    assert_eq!(
        fx.server.state.focused_tiled_pos(0, &tiled, Some(99)),
        Some(1)
    );
    // …and the same holds when focus is on a layer surface (no window focus).
    assert_eq!(fx.server.state.focused_tiled_pos(0, &tiled, None), Some(1));
}

#[test]
fn falls_back_to_none_when_no_history_entry_is_tiled() {
    // Nothing in history is tiled → `None`, so consumers keep their slot-0
    // default (e.g. a fresh tag). Never a panic on unknown ids.
    let (mut fx, _ids) = two_clients();
    let tiled = [0usize, 1];
    fx.server.state.monitors[0].focus_history.clear();
    assert_eq!(fx.server.state.focused_tiled_pos(0, &tiled, None), None);
    fx.server.state.monitors[0].focus_history.push_front(4242);
    assert_eq!(fx.server.state.focused_tiled_pos(0, &tiled, None), None);
}

#[test]
fn out_of_range_monitor_is_none_not_panic() {
    let (fx, _ids) = two_clients();
    let tiled = [0usize, 1];
    assert_eq!(fx.server.state.focused_tiled_pos(99, &tiled, None), None);
}

#[test]
fn tile_stack_member_with_a_large_min_height_no_longer_overflows_the_screen() {
    // A stacked client can declare its own xdg_toplevel minimum size (an
    // Electron app like Discord commonly does). Before this fix, the
    // per-client min-size clamp in `arrange_monitor` grew that client's
    // rect straight past its neighbours without shrinking anything else
    // to compensate — so it hung off the bottom of the work area
    // ("ekrandan çıkmış"), and in layouts with side-by-side columns
    // (center_tile) could overlap a neighbour outright.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    // One master + two stacked clients.
    for (app, title) in [
        ("kitty", "master"),
        ("kitty", "stack-a"),
        ("discord", "stack-b"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    // Force Tile explicitly (it's already the default — be explicit anyway).
    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Tile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // The bottom stack member declares a minimum height far bigger than
    // its fair-share slot (1080 split two ways ≈ 540 each).
    fx.server.state.clients[2].min_height = 900;

    fx.server.state.arrange_monitor(0);

    let work_area = fx.server.state.monitors[0].work_area;
    for c in &fx.server.state.clients {
        assert!(
            c.geom.y + c.geom.height <= work_area.y + work_area.height,
            "{} overflowed the bottom of the screen: y={} height={} work_area_bottom={}",
            c.app_id,
            c.geom.y,
            c.geom.height,
            work_area.y + work_area.height
        );
    }
    // The min-height client still got at least its declared minimum.
    assert!(
        fx.server.state.clients[2].geom.height >= 900,
        "min_height not honoured: {}",
        fx.server.state.clients[2].geom.height
    );

    // No two clients overlap.
    for i in 0..fx.server.state.clients.len() {
        for j in (i + 1)..fx.server.state.clients.len() {
            let a = fx.server.state.clients[i].geom;
            let b = fx.server.state.clients[j].geom;
            let overlap_x = a.x < b.x + b.width && b.x < a.x + a.width;
            let overlap_y = a.y < b.y + b.height && b.y < a.y + a.height;
            assert!(
                !(overlap_x && overlap_y),
                "clients {i} and {j} overlap: {a:?} vs {b:?}"
            );
        }
    }
}
