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

#[test]
fn right_tile_stack_member_with_a_large_min_height_no_longer_overflows_the_screen() {
    // right_tile is tile mirrored (stack on the left, master on the
    // right) — same vertical-stack shape, same bug, same fix.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

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

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::RightTile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;
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
    assert!(
        fx.server.state.clients[2].geom.height >= 900,
        "min_height not honoured: {}",
        fx.server.state.clients[2].geom.height
    );

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

#[test]
fn center_tile_stack_member_with_a_large_min_height_no_longer_overlaps_a_neighbour() {
    // Same bug, center_tile's shape: a centred master column flanked by
    // a left AND a right stack. With 5 clients and nmaster=1, the split
    // is left_count=2, right_count=2 — two independent 2-member vertical
    // stacks, either of which can hit the same overflow the plain
    // `tile` test covers. Here the min-height member sits at the
    // BOTTOM of the RIGHT stack, right where it would visibly overlap
    // the master column (or spill off-screen) if nothing shrank to
    // compensate.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [
        ("kitty", "master"),
        ("kitty", "left-a"),
        ("kitty", "left-b"),
        ("kitty", "right-a"),
        ("discord", "right-b"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::CenterTile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // Bottom-right stack member declares a minimum far bigger than its
    // fair-share slot (1080 split two ways ≈ 540 each).
    fx.server.state.clients[4].min_height = 950;

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
    assert!(
        fx.server.state.clients[4].geom.height >= 950,
        "min_height not honoured: {}",
        fx.server.state.clients[4].geom.height
    );

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

#[test]
fn tgmix_stack_member_with_a_large_min_height_no_longer_overflows_the_screen() {
    // tgmix's stack half isn't a plain vertical stack — it hands its
    // stack clients to `grid`, which produces a genuine matrix (rows
    // AND columns). But `apply_min_height_floors` doesn't special-case
    // top-level layouts: it groups the *final* rects by (x, width)
    // regardless of which sub-algorithm produced them, so a grid
    // column reached through tgmix should get exactly the same
    // redistribution as a plain tile/center_tile stack column.
    //
    // 1 master + 3 stack clients -> grid arranges the 3 stack clients
    // as a 2x2 matrix with one empty cell: column 0 gets two rows
    // (clients "stack-a" top, "stack-c" bottom), column 1 gets one.
    // Force the bottom-left cell's min_height far past its share.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [
        ("kitty", "master"),
        ("kitty", "stack-a"),
        ("kitty", "stack-b"),
        ("discord", "stack-c"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::TgMix;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // "stack-c" (bottom-left grid cell) declares a minimum far bigger
    // than its fair-share (1080 split two ways ≈ 540).
    fx.server.state.clients[3].min_height = 950;

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
    assert!(
        fx.server.state.clients[3].geom.height >= 950,
        "min_height not honoured: {}",
        fx.server.state.clients[3].geom.height
    );

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

#[test]
fn dwindle_stack_member_with_a_large_min_height_stays_on_screen() {
    // dwindle's spiral splits the work area on an *alternating* axis
    // each step, so a stacked client's height-overflow neighbour isn't
    // always a rect it shares a column with — outside what
    // `apply_min_height_floors`'s column grouping can redistribute
    // into. `clamp_to_work_area` is the safety net for exactly this
    // shape: the client must never end up hanging off the bottom of
    // the screen, even if repositioning it can't help but touch an
    // immediate neighbour in this one irreducible case.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [
        ("kitty", "a"),
        ("kitty", "b"),
        ("kitty", "c"),
        ("discord", "d"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Dwindle;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // The final spiral leaf declares a minimum far bigger than the
    // sliver dwindle's split naturally leaves it.
    fx.server.state.clients[3].min_height = 950;

    fx.server.state.arrange_monitor(0);

    let work_area = fx.server.state.monitors[0].work_area;
    for c in &fx.server.state.clients {
        assert!(
            c.geom.x >= work_area.x
                && c.geom.y >= work_area.y
                && c.geom.x + c.geom.width <= work_area.x + work_area.width
                && c.geom.y + c.geom.height <= work_area.y + work_area.height,
            "{} escaped the work area: {:?} (work area: {:?})",
            c.app_id,
            c.geom,
            work_area
        );
    }
    assert!(
        fx.server.state.clients[3].geom.height >= 950,
        "min_height not honoured: {}",
        fx.server.state.clients[3].geom.height
    );
}

#[test]
fn deck_stack_members_with_a_large_min_height_do_not_shrink_each_other() {
    // deck's stack members deliberately share the *exact same* rect —
    // it's a tabbed "deck of cards", only one shown at a time, not a
    // vertical stack that must divide a span between its members. They
    // all land in one `stack_columns` group (identical x/width, and
    // trivially "overlapping" since they're the same rect), but that
    // group must NOT be treated as height-competing siblings: shrinking
    // "stack-a"/"stack-b" to make room for "discord"'s inflated
    // min_height would be actively wrong, since none of them are
    // actually sharing space with anyone.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [
        ("kitty", "master"),
        ("kitty", "stack-a"),
        ("kitty", "stack-b"),
        ("discord", "stack-c"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Deck;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;
    fx.server.state.clients[3].min_height = 950;

    let natural_stack_height = {
        // stack-a's height before the min-height client is introduced —
        // the height every deck stack member (this one included) should
        // keep, since they aren't sharing space with one another.
        fx.server.state.clients[3].min_height = 0;
        fx.server.state.arrange_monitor(0);
        let h = fx.server.state.clients[1].geom.height;
        fx.server.state.clients[3].min_height = 950;
        h
    };

    fx.server.state.arrange_monitor(0);

    assert_eq!(
        fx.server.state.clients[1].geom.height, natural_stack_height,
        "stack-a shrank to make room for stack-c's min_height, but they don't share space"
    );
    assert_eq!(
        fx.server.state.clients[2].geom.height, natural_stack_height,
        "stack-b shrank to make room for stack-c's min_height, but they don't share space"
    );
    assert!(
        fx.server.state.clients[3].geom.height >= 950,
        "min_height not honoured: {}",
        fx.server.state.clients[3].geom.height
    );

    let work_area = fx.server.state.monitors[0].work_area;
    assert!(
        fx.server.state.clients[3].geom.y + fx.server.state.clients[3].geom.height
            <= work_area.y + work_area.height,
        "stack-c overflowed the bottom of the screen: {:?}",
        fx.server.state.clients[3].geom
    );
}

#[test]
fn tgmix_grid_cell_with_a_large_min_width_no_longer_overlaps_its_row_mate() {
    // Width's twin of the height bug, reproduced from a real overlap:
    // Discord's own declared minimum WIDTH (its chat UI needs real
    // horizontal room) grew its grid cell past its column boundary in
    // place, riding over the cell next to it in the same row — while
    // the master column and the row below were untouched, so it read
    // as "some windows overlap, others don't" rather than an obvious
    // uniform bug.
    //
    // 1 master + 4 stack clients -> grid arranges the stack as a clean
    // 2x2 matrix: "top-left"/"top-right" share row 0, "bottom-left"/
    // "bottom-right" share row 1. Force top-left's min_width past its
    // column's natural share, but well within what the row can still
    // provide once its row-mate shrinks to make room — see
    // `apply_min_width_floors`'s doc comment for the separate,
    // genuinely irreducible case (a floor bigger than the *entire*
    // row) that this test isn't about.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [
        ("kitty", "master"),
        ("discord", "top-left"),
        ("kitty", "top-right"),
        ("kitty", "bottom-left"),
        ("kitty", "bottom-right"),
    ] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::TgMix;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // "top-left" declares a minimum far bigger than its column's fair
    // share of the stack half's width — comfortably less than the
    // row's whole span, so its row-mate has room to shrink and make
    // way for it.
    fx.server.state.clients[1].min_width = 700;

    fx.server.state.arrange_monitor(0);

    let work_area = fx.server.state.monitors[0].work_area;
    for c in &fx.server.state.clients {
        assert!(
            c.geom.x + c.geom.width <= work_area.x + work_area.width,
            "{} overflowed the right edge of the screen: {:?}",
            c.app_id,
            c.geom
        );
    }
    assert!(
        fx.server.state.clients[1].geom.width >= 700,
        "min_width not honoured: {}",
        fx.server.state.clients[1].geom.width
    );

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

#[test]
fn scroller_ignores_min_width_row_redistribution() {
    // Scroller's whole point is that a column can grow past its
    // neighbours (the strip pans horizontally) -- unlike every other
    // contained layout, a min-width member here must NOT shrink its
    // row-mates to make room. All three tiled clients naturally share
    // the same y/height in scroller (one full-height row), which is
    // exactly the shape `apply_min_width_floors` would otherwise
    // redistribute -- confirming the Scroller exception actually
    // takes effect, not just that it compiles.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_output("DP-1", (1920, 1080));

    for (app, title) in [("kitty", "one"), ("discord", "two"), ("kitty", "three")] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id(app.into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Scroller;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    let natural_width = {
        fx.server.state.arrange_monitor(0);
        fx.server.state.clients[0].geom.width
    };

    fx.server.state.clients[1].min_width = 1800;
    fx.server.state.arrange_monitor(0);

    assert_eq!(
        fx.server.state.clients[0].geom.width, natural_width,
        "scroller column 'one' shrank to make room for a wider neighbour"
    );
    assert_eq!(
        fx.server.state.clients[2].geom.width, natural_width,
        "scroller column 'three' shrank to make room for a wider neighbour"
    );
    assert!(fx.server.state.clients[1].geom.width >= 1800);
}
