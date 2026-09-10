//! `dispatch_action` — the compositor's action table, reached from
//! `bind =` lines, `mctl dispatch`, and Rhai hooks.
//!
//! Covers the outcome contract: a recognised action reports `Handled`,
//! an unrecognised one reports `Unknown` so the IPC layer can answer
//! `{"error": …}` instead of a lying `{"ok": true}`.

use margo_config::Arg;

use super::fixture::Fixture;
use crate::dispatch::{DispatchOutcome, dispatch_action};

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

/// Catalogue entries whose handler does real host I/O with a default
/// `Arg` — a file write, a real theme apply. Skipped here so the sweep
/// stays hermetic; their handlers are exercised elsewhere.
const SKIP_SIDE_EFFECTING: &[&str] = &[
    "session_save",
    "theme",
    "screenshot",
    "screenshot-window",
    "screenshot-region",
    "screenshot-output",
];

#[test]
fn every_catalogued_action_is_handled_by_the_dispatcher() {
    // The `margo-config` catalogue drives `mctl actions`, the shell's
    // layout menu, tab-completion, and the config validator's W005
    // "unknown bind action" check — so an entry the dispatcher's
    // `match action` doesn't actually cover is a silent lie in all of
    // them. This is the guard the missing `focuslast` arm slipped past.
    let mut fx = state_with_one_window();
    for a in margo_config::actions::ACTIONS {
        for name in std::iter::once(a.name).chain(a.aliases.iter().copied()) {
            if SKIP_SIDE_EFFECTING.contains(&name) {
                continue;
            }
            assert_ne!(
                dispatch_action(&mut fx.server.state, name, &Arg::default()),
                DispatchOutcome::Unknown,
                "catalogued action `{name}` has no arm in dispatch_action",
            );
        }
    }
}
