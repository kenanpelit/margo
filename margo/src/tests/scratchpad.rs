//! Scratchpads (`toggle_named_scratchpad`, `toggle_scratchpad_id`,
//! `toggle_scratchpad`, `unscratchpad`) and run-or-raise (`summon`,
//! `focusapp` with its `MatchOp`) against real Wayland clients.

use super::fixture::Fixture;
use crate::state::{FocusTarget, MatchOp};

/// One 1080p output with a keyboard and a mapped toplevel per
/// `(app_id, title)`; clients land at state indices 0, 1, ...
fn windows(apps: &[(&str, &str)], single_scratchpad: bool) -> Fixture {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.animations = false;
    fx.server.state.config.single_scratchpad = single_scratchpad;
    for (app, title) in apps {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id((*app).into());
        toplevel.set_title((*title).into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    fx
}

fn focus(fx: &mut Fixture, idx: usize) {
    let w = fx.server.state.clients[idx].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w)));
}

fn shown(fx: &Fixture, idx: usize) -> bool {
    let c = &fx.server.state.clients[idx];
    c.is_in_scratchpad && c.is_scratchpad_show && !c.is_minimized
}

fn hidden(fx: &Fixture, idx: usize) -> bool {
    let c = &fx.server.state.clients[idx];
    c.is_in_scratchpad && !c.is_scratchpad_show && c.is_minimized
}

fn mapped(fx: &Fixture, idx: usize) -> bool {
    let w = &fx.server.state.clients[idx].window;
    fx.server.state.space.elements().any(|e| e == w)
}

#[test]
fn match_op_parses_the_bind_field() {
    for (s, want) in [
        (None, "and"),
        (Some(""), "and"),
        (Some("and"), "and"),
        (Some("bogus"), "and"),
        (Some(" OR "), "or"),
        (Some("union"), "or"),
        (Some("difference"), "diff"),
        (Some("diff"), "diff"),
        (Some("Minus"), "diff"),
    ] {
        let got = match MatchOp::parse(s) {
            MatchOp::And => "and",
            MatchOp::Or => "or",
            MatchOp::Difference => "diff",
        };
        assert_eq!(got, want, "{s:?}");
    }
}

#[test]
fn named_scratchpad_toggles_hidden_then_shown_then_hidden() {
    let mut fx = windows(&[("kitty", "term"), ("clipse", "clip")], false);
    // First press on a running, ordinary window: promoted and shown.
    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), None, None);
    assert!(shown(&fx, 1));
    let c = &fx.server.state.clients[1];
    assert!(c.is_named_scratchpad && c.is_floating);
    assert_eq!(fx.server.state.focused_client_idx(), Some(1));
    assert!(mapped(&fx, 1));

    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), None, None);
    assert!(hidden(&fx, 1));
    assert!(!mapped(&fx, 1), "hidden scratchpad is unmapped");
    assert_ne!(
        fx.server.state.focused_client_idx(),
        Some(1),
        "focus leaves the hidden surface"
    );

    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), None, None);
    assert!(shown(&fx, 1));
    assert!(mapped(&fx, 1));
    // The other window was never touched.
    assert!(!fx.server.state.clients[0].is_in_scratchpad);
}

#[test]
fn named_scratchpad_pattern_is_a_regex_not_a_substring() {
    let mut fx = windows(&[("org.clipse.helper", "x"), ("clipse", "clip")], false);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), None, None);
    assert!(!fx.server.state.clients[0].is_in_scratchpad);
    assert!(shown(&fx, 1));
    // A title pattern narrows it further; no match → nothing changes.
    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), Some("^nope$"), None);
    assert!(shown(&fx, 1));
}

#[test]
fn single_scratchpad_hides_the_other_one() {
    let mut fx = windows(&[("a", "a"), ("b", "b")], true);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^a$"), None, None);
    assert!(shown(&fx, 0));
    fx.server
        .state
        .toggle_named_scratchpad(Some("^b$"), None, None);
    assert!(shown(&fx, 1));
    assert!(hidden(&fx, 0), "only one scratchpad visible");

    // Off: both may be visible.
    let mut fx = windows(&[("a", "a"), ("b", "b")], false);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^a$"), None, None);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^b$"), None, None);
    assert!(shown(&fx, 0) && shown(&fx, 1));
}

#[test]
fn toggle_by_id_never_promotes_an_ordinary_window() {
    let mut fx = windows(&[("a", "a"), ("b", "b")], true);
    let id_a = fx.server.state.clients[0].id;
    let id_b = fx.server.state.clients[1].id;
    fx.server.state.toggle_scratchpad_id(id_a);
    assert!(!fx.server.state.clients[0].is_in_scratchpad);
    assert!(mapped(&fx, 0));

    fx.server
        .state
        .toggle_named_scratchpad(Some("^a$"), None, None);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^b$"), None, None);
    assert!(hidden(&fx, 0) && shown(&fx, 1));
    // Showing `a` by id honours single_scratchpad too.
    fx.server.state.toggle_scratchpad_id(id_a);
    assert!(shown(&fx, 0) && hidden(&fx, 1));
    fx.server.state.toggle_scratchpad_id(id_a);
    assert!(hidden(&fx, 0));
    // Unknown id: no-op.
    fx.server.state.toggle_scratchpad_id(u64::MAX);
    assert!(hidden(&fx, 0) && hidden(&fx, 1));
    let _ = id_b;
}

#[test]
fn unscratchpad_recovers_a_normal_tile() {
    let mut fx = windows(&[("a", "a"), ("b", "b")], false);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^b$"), None, None);
    assert_eq!(fx.server.state.focused_client_idx(), Some(1));
    fx.server.state.unscratchpad_focused();
    let c = &fx.server.state.clients[1];
    assert!(!c.is_in_scratchpad && !c.is_named_scratchpad && !c.is_scratchpad_show);
    assert!(!c.is_floating && !c.is_minimized);
    assert!(mapped(&fx, 1));
    // The next toggle press promotes it again rather than finding a
    // half-scratchpad.
    fx.server
        .state
        .toggle_named_scratchpad(Some("^b$"), None, None);
    assert!(shown(&fx, 1));
}

#[test]
fn focusapp_cycles_through_matching_windows() {
    let mut fx = windows(
        &[
            ("firefox", "one"),
            ("kitty", "k"),
            ("firefox", "two"),
            ("firefox", "three"),
        ],
        false,
    );
    focus(&mut fx, 1);
    let mut order = Vec::new();
    for _ in 0..4 {
        fx.server
            .state
            .focus_or_raise(Some("^firefox$"), None, MatchOp::And, None);
        order.push(fx.server.state.focused_client_idx().unwrap());
    }
    assert_eq!(order, [0, 2, 3, 0], "first, then the next on each press");
}

#[test]
fn focusapp_match_ops_combine_app_id_and_title() {
    let apps = [
        ("msedge", "Outlook Calendar"),
        ("msedge", "News"),
        ("kitty", "Calendar notes"),
    ];
    let pick = |name: Option<&str>, title: Option<&str>, op| {
        let mut fx = windows(&apps, false);
        focus(&mut fx, 2);
        fx.server.state.focus_or_raise(name, title, op, None);
        fx.server.state.focused_client_idx()
    };
    assert_eq!(
        pick(Some("^msedge$"), Some("Calendar"), MatchOp::And),
        Some(0)
    );
    assert_eq!(
        pick(Some("^msedge$"), Some("Calendar"), MatchOp::Difference),
        Some(1),
        "same app, not that window"
    );
    // Or: kitty's "Calendar notes" also matches; it is focused, so the
    // next match after it wraps to the first.
    assert_eq!(pick(Some("^nope$"), Some("Calendar"), MatchOp::Or), Some(0));
    // Nothing matches and no spawn command: focus stays.
    assert_eq!(pick(Some("^nope$"), None, MatchOp::And), Some(2));
}

#[test]
fn summon_brings_the_window_to_the_current_tag() {
    let mut fx = windows(&[("kitty", "k"), ("spotify", "s")], false);
    // Move spotify to tag 2 and stay on tag 1.
    fx.server.state.clients[1].tags = 1 << 1;
    fx.server.state.arrange_monitor(0);
    focus(&mut fx, 0);
    let here = fx.server.state.monitors[0].current_tagset();
    assert!(!fx.server.state.clients[1].is_visible_on(0, here));

    fx.server
        .state
        .summon(Some("^spotify$"), None, MatchOp::And, None);
    assert_eq!(fx.server.state.clients[1].tags, here);
    assert_eq!(fx.server.state.focused_client_idx(), Some(1));
    assert_eq!(
        fx.server.state.monitors[0].current_tagset(),
        here,
        "summon does not switch tags"
    );
}

#[test]
fn summon_skips_hidden_scratchpads() {
    let mut fx = windows(&[("kitty", "k"), ("kitty", "dropdown")], false);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^kitty$"), Some("^dropdown$"), None);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^kitty$"), Some("^dropdown$"), None);
    assert!(hidden(&fx, 1));
    fx.server
        .state
        .focus_or_raise(Some("^kitty$"), None, MatchOp::And, None);
    assert_eq!(fx.server.state.focused_client_idx(), Some(0));
    focus(&mut fx, 0);
    fx.server
        .state
        .summon(Some("^kitty$"), Some("^dropdown$"), MatchOp::And, None);
    assert!(hidden(&fx, 1), "summon leaves the hidden scratchpad alone");
}

/// Anonymous scratchpads (restored from a session snapshot) toggle with
/// `toggle_scratchpad`; with `single_scratchpad` a shown NAMED one is
/// hidden at the same time (mango's toggle_scratchpad).
#[test]
fn anonymous_toggle_honours_single_scratchpad() {
    let mut fx = windows(&[("kitty", "k"), ("notes", "n"), ("clipse", "c")], true);
    // `notes`: an anonymous, hidden scratchpad.
    {
        let c = &mut fx.server.state.clients[1];
        c.is_in_scratchpad = true;
        c.is_scratchpad_show = false;
        c.is_minimized = true;
    }
    let w = fx.server.state.clients[1].window.clone();
    fx.server.state.space.unmap_elem(&w);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^clipse$"), None, None);
    assert!(shown(&fx, 2));

    fx.server.state.toggle_scratchpad();
    assert!(shown(&fx, 1));
    assert!(
        hidden(&fx, 2),
        "the named one is hidden by single_scratchpad"
    );

    fx.server.state.toggle_scratchpad();
    assert!(hidden(&fx, 1));
}

/// Hiding the only window leaves nothing focused; the toggle must still
/// bring the anonymous scratchpad back.
#[test]
fn anonymous_toggle_works_with_nothing_focused() {
    let mut fx = windows(&[("notes", "n")], false);
    {
        let c = &mut fx.server.state.clients[0];
        c.is_in_scratchpad = true;
        c.is_scratchpad_show = true;
    }
    fx.server.state.toggle_scratchpad();
    assert!(hidden(&fx, 0));
    assert_eq!(fx.server.state.focused_client_idx(), None);
    fx.server.state.toggle_scratchpad();
    assert!(shown(&fx, 0), "shown again from an empty tag");
}
