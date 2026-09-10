//! `arrange_monitor` behaviour tests.
//!
//! `focused_tiled_pos` — the anchor slot focus-following layouts
//! (scroller, vertical scroller, deck) centre on. Locks the fix for "a
//! floating keyring / polkit dialog grabbing focus snaps the scroller
//! strip to window 1": the naive lookup returns `None` when focus is off
//! the tiled strip, and every consumer then falls back to slot 0;
//! `focused_tiled_pos` instead holds the strip on the most-recently-
//! focused window that is still tiled (per-monitor MRU `focus_history`,
//! most-recent first).
//!
//! Tiled min/max-size — a tiled window gets exactly its layout slot; its
//! own `xdg_toplevel` min/max-size request never grows it past that slot
//! or eats the gaps around it (regression: Discord's 940×~500 minimum
//! collapsed center_tile / tgmix / dwindle to edge-to-edge).
//!
//! Deck z-order — the focused stack card is raised to the front of the
//! scene, since every deck card shares one rect.

use margo_config::Config;

use super::fixture::Fixture;
use crate::state::FocusTarget;

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
fn a_tiled_clients_min_size_is_ignored_and_the_gaps_survive() {
    // A stacked client can declare its own `xdg_toplevel` minimum size —
    // Discord/Electron routinely does (940×~500). It must not grow past
    // the slot the layout gave it, and the inter-window gap must not be
    // spent making room for it.
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

    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Tile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;

    // Pristine layout, before any client declares a minimum.
    fx.server.state.arrange_monitor(0);
    let pristine: Vec<_> = fx.server.state.clients.iter().map(|c| c.geom).collect();
    let gap_before = fx.server.state.clients[2].geom.y
        - (fx.server.state.clients[1].geom.y + fx.server.state.clients[1].geom.height);
    assert!(gap_before > 0, "fixture should tile with a real gap");

    // The bottom stack member now declares a minimum far bigger than its
    // ~half-column slot, on both axes.
    fx.server.state.clients[2].min_height = 900;
    fx.server.state.clients[2].min_width = 1800;
    fx.server.state.arrange_monitor(0);

    for (i, c) in fx.server.state.clients.iter().enumerate() {
        assert_eq!(
            c.geom, pristine[i],
            "{}'s slot changed after it declared a min-size — tiled min-size must be ignored",
            c.app_id
        );
    }
    let gap_after = fx.server.state.clients[2].geom.y
        - (fx.server.state.clients[1].geom.y + fx.server.state.clients[1].geom.height);
    assert_eq!(gap_after, gap_before, "the inter-window gap was eaten");
}

#[test]
fn deck_raises_the_focused_stack_card_to_the_front() {
    // `deck` gives every stack member the one identical rect, so the
    // card you actually see is whichever sits on top of the scene
    // z-order. Cycling focus through the stack must bring the focused
    // card to the front, not just move the border highlight.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = crate::layout::LayoutId::Deck;
    fx.server.state.monitors[0].pertag.user_picked_layout[curtag] = true;

    // 1 master + 2 stack cards.
    for _ in 0..3 {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id("kitty".into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    fx.server.state.arrange_monitor(0);

    assert_eq!(
        fx.server.state.clients[1].geom, fx.server.state.clients[2].geom,
        "deck stack cards should share one rect"
    );

    let stack_a = fx.server.state.clients[1].window.clone();
    let stack_b = fx.server.state.clients[2].window.clone();

    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(stack_a.clone())));
    {
        let els: Vec<_> = fx.server.state.space.elements().collect();
        let pa = els.iter().position(|&e| *e == stack_a);
        let pb = els.iter().position(|&e| *e == stack_b);
        assert!(
            pa > pb && pa.is_some(),
            "focusing stack card A must raise it above B (a={pa:?} b={pb:?})"
        );
    }

    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(stack_b.clone())));
    {
        let els: Vec<_> = fx.server.state.space.elements().collect();
        let pa = els.iter().position(|&e| *e == stack_a);
        let pb = els.iter().position(|&e| *e == stack_b);
        assert!(
            pb > pa && pb.is_some(),
            "focusing stack card B must raise it above A (a={pa:?} b={pb:?})"
        );
    }
}

#[test]
fn focus_last_toggles_between_the_two_most_recent_windows() {
    // `focuslast` (dwl): jump back to the window focused before the
    // current one; press again to return.
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));

    for title in ["a", "b", "c"] {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id("kitty".into());
        toplevel.set_title(title.into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    fx.server.state.arrange_monitor(0);

    let win_a = fx.server.state.clients[0].window.clone();
    let win_c = fx.server.state.clients[2].window.clone();

    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win_a)));
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win_c)));
    assert_eq!(fx.server.state.focused_client_idx(), Some(2));

    fx.server.state.focus_last();
    assert_eq!(
        fx.server.state.focused_client_idx(),
        Some(0),
        "focus_last returns to the prior window"
    );

    fx.server.state.focus_last();
    assert_eq!(
        fx.server.state.focused_client_idx(),
        Some(2),
        "focus_last again toggles back"
    );
}

#[test]
fn focus_last_is_a_noop_with_no_prior_window() {
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (toplevel, surface) = fx.client(id).create_toplevel();
    toplevel.set_app_id("kitty".into());
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    fx.server.state.arrange_monitor(0);

    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));
    let before = fx.server.state.focused_client_idx();
    fx.server.state.focus_last();
    assert_eq!(fx.server.state.focused_client_idx(), before);
}
