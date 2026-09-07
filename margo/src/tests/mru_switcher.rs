//! The niri-style MRU window switcher (Super+Tab) — `mru_advance`/
//! `mru_confirm`, and `activate_window_idx` (which they, and
//! wlr-foreign-toplevel activation, both go through).
//!
//! Regression coverage for a bug reported against `scroller`: cycling
//! visibly worked (the overlay/focus highlight moves), but releasing the
//! modifier committed real keyboard focus to the target window *without
//! ever scrolling it into view* — every other focus-changing dispatch
//! (`focus_stack`, directional focus, ...) calls `arrange_monitor` right
//! after `focus_surface`, since `scroller` only repositions its viewport
//! to reveal the focused column as a side effect of that arrange pass
//! reading focus back out. `activate_window_idx` was the one path that
//! skipped it — invisible on layouts where every window is always fully
//! on screen, but on `scroller`, with `workspace`-scoped candidates
//! (same tag throughout, so `view_tag`'s own internal arrange never
//! fires either), it meant Tab could change *who* has focus without ever
//! bringing them into view.

use super::fixture::Fixture;
use crate::layout::LayoutId;
use crate::state::FocusTarget;
use crate::state::mru_switcher::{MruDirection, MruFilter, MruScope};

/// `n` windows on one 1920×1080 output, pinned to `scroller` on the
/// current tag. `scroller`'s default column width (a large fraction of
/// the work area) means even a handful of windows don't all fit at once,
/// so cycling to a later one genuinely requires the viewport to scroll.
/// Animations off: `.geom` is otherwise the *animated, mid-flight* slot
/// (see `arrange_monitor`'s doc comment on `old`/`already_animating_to_
/// target`), not the target — these tests care about the settled target.
fn n_scroller_windows(fx: &mut Fixture, n: usize) {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.animations = false;
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = LayoutId::Scroller;
    for _ in 0..n {
        let id = fx.add_client();
        let (_toplevel, surface) = fx.client(id).create_toplevel();
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
}

#[test]
fn activate_window_idx_scrolls_scroller_to_reveal_the_target() {
    let mut fx = Fixture::new();
    n_scroller_windows(&mut fx, 5);

    // Focus window 0 for real (sets keyboard focus + `monitors[].selected`
    // + arranges), establishing a known starting scroll position.
    let w0 = fx.server.state.clients[0].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w0)));
    fx.server.state.arrange_monitor(0);
    let before: Vec<crate::layout::Rect> = fx.server.state.clients.iter().map(|c| c.geom).collect();

    // The last window is far enough from window 0 that scroller cannot
    // have both fully on screen at once at its default column width —
    // reaching it requires an actual scroll, not just a focus change.
    let last = fx.server.state.clients.len() - 1;
    fx.server.state.activate_window_idx(last);

    assert_eq!(
        fx.server.state.focused_client_idx(),
        Some(last),
        "activate_window_idx must move real keyboard focus"
    );

    let after: Vec<crate::layout::Rect> = fx.server.state.clients.iter().map(|c| c.geom).collect();
    assert_ne!(
        before, after,
        "scroller's viewport never scrolled — activate_window_idx changed \
         focus without arranging, so the newly focused column stayed \
         wherever it was before"
    );

    let target = fx.server.state.clients[last].geom;
    let wa = fx.server.state.monitors[0].work_area;
    assert!(
        target.x >= wa.x && target.x + target.width <= wa.x + wa.width,
        "the newly focused window should be scrolled fully into view, got {target:?} vs work area {wa:?}"
    );
}

#[test]
fn mru_confirm_scrolls_scroller_to_reveal_the_committed_window() {
    // End-to-end through the actual switcher (open, advance repeatedly,
    // confirm) rather than calling `activate_window_idx` directly, since
    // that's the real Super+Tab path (`super,Tab,mru_next,workspace`).
    let mut fx = Fixture::new();
    n_scroller_windows(&mut fx, 5);

    let w0 = fx.server.state.clients[0].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w0)));
    fx.server.state.arrange_monitor(0);

    // `mru_advance`'s candidates are recency-ordered, not creation-index
    // order (see `mru_candidate_windows`), so which client index ends up
    // highlighted after `n - 1` forward steps isn't something to
    // hardcode here — only that stepping through every other candidate
    // once lands on a *different* window than the one we started on.
    let n = fx.server.state.clients.len();
    for _ in 0..(n - 1) {
        fx.server
            .state
            .mru_advance(MruScope::Workspace, MruFilter::All, MruDirection::Forward);
    }
    fx.server.state.mru_confirm();

    let committed = fx
        .server
        .state
        .focused_client_idx()
        .expect("mru_confirm must focus something");
    assert_ne!(
        committed, 0,
        "cycling through every other candidate must land somewhere other than where we started"
    );

    let target = fx.server.state.clients[committed].geom;
    let wa = fx.server.state.monitors[0].work_area;
    assert!(
        target.x >= wa.x && target.x + target.width <= wa.x + wa.width,
        "the committed window should be scrolled fully into view, got {target:?} vs work area {wa:?}"
    );
}
