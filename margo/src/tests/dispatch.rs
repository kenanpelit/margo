//! `dispatch_action` — the compositor's action table, reached from
//! `bind =` lines, `mctl dispatch`, and Rhai hooks.
//!
//! Covers the outcome contract: a recognised action reports `Handled`,
//! an unrecognised one reports `Unknown` so the IPC layer can answer
//! `{"error": …}` instead of a lying `{"ok": true}`.

use margo_config::{Arg, Config};

use super::fixture::Fixture;
use crate::dispatch::{DispatchOutcome, dispatch_action};
use crate::state::FocusTarget;

fn state_with_one_window() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (toplevel, surface) = fx.client(id).create_toplevel();
    toplevel.set_app_id("kitty".into());
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    fx
}

#[test]
fn unknown_action_reports_unknown() {
    let mut fx = state_with_one_window();
    assert_eq!(
        dispatch_action(&mut fx.server.state, "focuslsat", &Arg::default()),
        DispatchOutcome::Unknown,
        "a typo'd action must not be reported as handled"
    );
}

#[test]
fn recognised_actions_report_handled() {
    let mut fx = state_with_one_window();
    for action in ["killclient", "togglefloating", "focuslast", "reload"] {
        assert_eq!(
            dispatch_action(&mut fx.server.state, action, &Arg::default()),
            DispatchOutcome::Handled,
            "`{action}` should be a recognised action"
        );
    }
}

#[test]
fn a_recognised_action_that_fails_internally_still_reports_handled() {
    // `session_load` with no saved session on disk logs its own error
    // and bails — the action was still recognised, so the CLI must not
    // treat it like a typo.
    let mut fx = state_with_one_window();
    assert_eq!(
        dispatch_action(&mut fx.server.state, "session_load", &Arg::default()),
        DispatchOutcome::Handled,
    );
}

/// An action whose handler does real host I/O even with a default
/// `Arg` (a file write, a theme rebuild, spawning `mscreenshot`).
/// Skipped by the sweeps so they stay hermetic; the handlers are
/// exercised in their own tests.
fn is_side_effecting(name: &str) -> bool {
    name.starts_with("screenshot") || matches!(name, "session_save" | "theme")
}

#[test]
fn every_catalogued_action_is_handled_by_the_dispatcher() {
    // The `margo-config` catalogue drives `mctl actions`, the shell's
    // layout menu, tab-completion, and (pending) the config validator's
    // "unknown bind action" check — so an entry the dispatcher's
    // `match action` doesn't actually cover is a silent lie in all of
    // them. This is the guard the missing `focuslast` arm slipped past.
    let mut fx = state_with_one_window();
    for a in margo_config::actions::ACTIONS {
        if is_side_effecting(a.name) {
            continue;
        }
        for name in std::iter::once(a.name).chain(a.aliases.iter().copied()) {
            assert_ne!(
                dispatch_action(&mut fx.server.state, name, &Arg::default()),
                DispatchOutcome::Unknown,
                "catalogued action `{name}` has no arm in dispatch_action",
            );
        }
    }
}

#[test]
fn every_dispatch_action_is_in_the_catalogue() {
    // The reverse guard: a `match action` arm the catalogue omits is
    // invisible to `mctl actions` / completions, and would make the
    // validator's action-name check false-flag a valid bind. Parses
    // the top-level arm heads out of the dispatcher source.
    let src = include_str!("../dispatch/mod.rs");
    let body = {
        let start = src.find("pub fn dispatch_action").expect("fn present");
        let after_match = src[start..]
            .find("match action {")
            .map(|o| start + o + "match action {".len())
            .expect("match present");
        &src[after_match..]
    };

    let mut depth: i32 = 0;
    let mut uncatalogued: Vec<&str> = Vec::new();
    for line in body.lines() {
        if depth == 0
            && let Some(arm) = line.strip_prefix("        ")
            && arm.starts_with('"')
            && let Some(fat) = arm.find("=>")
        {
            for tok in arm[..fat].split('|') {
                let name = tok.trim().trim_matches('"');
                // An action name is lowercase snake/kebab, 3+ chars —
                // filters out any stray short token an arm's LHS might
                // carry.
                if name.len() >= 3
                    && name.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
                    })
                    && !margo_config::actions::is_known(name)
                {
                    uncatalogued.push(name);
                }
            }
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth < 0 {
            break; // past the closing brace of `match action`
        }
    }

    assert!(
        uncatalogued.is_empty(),
        "dispatch_action arms missing from the margo-config catalogue: {uncatalogued:?}"
    );
}

// ── move_focused / resize_focused / toggle_floating: float_geom staleness ──
//
// Regression coverage: `movewin` / `resizewin` / `togglefloating` always
// float the focused window, seeding `float_geom` from the current tiled
// `geom` only the *first* time (the old guard was `float_geom.width ==
// 0`). A window once shrunk to a sliver by `resizewin`, tiled again
// (looking perfectly normal), then floated a second time by any of the
// three actions silently snapped back to that invisible sliver — the
// stale, nonzero `float_geom` from the earlier episode was never
// invalidated. Reported live: a browser window `super+ctrl+shift,h/j/k/l`
// had been resized down to 1x1, `togglefloating` correctly re-tiled it
// (visible again), but a later `movewin` on the same client made it
// vanish again.

fn tiled_two_window_fixture() -> Fixture {
    let mut fx = Fixture::with_config(Config {
        animations: false,
        ..Config::default()
    });
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    for _ in 0..2 {
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id("kitty".into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
    fx.server.state.monitors[0].pertag.ltidxs[1] = crate::layout::LayoutId::Tile;
    fx.server.state.monitors[0].pertag.user_picked_layout[1] = true;
    fx.server.state.arrange_monitor(0);
    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));
    fx
}

/// Poke a degenerate `float_geom` onto client 0 as if a much earlier
/// floating episode had shrunk it there, without touching `is_floating`
/// (the client is still tiled, exactly as observed live).
fn leave_a_stale_sliver_float_geom(fx: &mut Fixture) {
    fx.server.state.clients[0].float_geom = crate::layout::Rect::new(-999, -999, 1, 1);
    assert!(!fx.server.state.clients[0].is_floating);
}

#[test]
fn move_focused_reseeds_a_stale_float_geom_from_the_tiled_slot() {
    let mut fx = tiled_two_window_fixture();
    let tiled = fx.server.state.clients[0].geom;
    leave_a_stale_sliver_float_geom(&mut fx);

    fx.server.state.move_focused(10, 20);

    let c = &fx.server.state.clients[0];
    assert!(c.is_floating);
    assert_eq!(
        (c.float_geom.width, c.float_geom.height),
        (tiled.width, tiled.height),
        "must reseed from the tiled slot's real size, not the stale 1x1"
    );
    assert_eq!(
        (c.float_geom.x, c.float_geom.y),
        (tiled.x + 10, tiled.y + 20)
    );
}

#[test]
fn resize_focused_reseeds_a_stale_float_geom_from_the_tiled_slot() {
    let mut fx = tiled_two_window_fixture();
    let tiled = fx.server.state.clients[0].geom;
    leave_a_stale_sliver_float_geom(&mut fx);

    fx.server.state.resize_focused(100, 50);

    let c = &fx.server.state.clients[0];
    assert!(c.is_floating);
    assert_eq!(
        (c.float_geom.width, c.float_geom.height),
        (tiled.width + 100, tiled.height + 50),
        "must grow from the tiled slot's real size, not 1x1 + delta"
    );
}

#[test]
fn toggle_floating_reseeds_a_stale_float_geom_from_the_tiled_slot() {
    let mut fx = tiled_two_window_fixture();
    let tiled = fx.server.state.clients[0].geom;
    leave_a_stale_sliver_float_geom(&mut fx);

    fx.server.state.toggle_floating();

    assert!(fx.server.state.clients[0].is_floating);
    assert_eq!(
        fx.server.state.clients[0].geom, tiled,
        "floating on the real tiled size must render at that size, not 1x1"
    );
}

#[test]
fn move_focused_preserves_an_already_floating_windows_real_geometry() {
    // No-regression: a window the user deliberately floated and placed
    // (not a stale/degenerate leftover) must not get snapped back to its
    // tiled slot just because it's being nudged.
    let mut fx = tiled_two_window_fixture();
    let placed = crate::layout::Rect::new(300, 250, 640, 480);
    fx.server.state.clients[0].is_floating = true;
    fx.server.state.clients[0].float_geom = placed;

    fx.server.state.move_focused(10, 20);

    let c = &fx.server.state.clients[0];
    assert_eq!((c.float_geom.width, c.float_geom.height), (640, 480));
    assert_eq!((c.float_geom.x, c.float_geom.y), (310, 270));
}
