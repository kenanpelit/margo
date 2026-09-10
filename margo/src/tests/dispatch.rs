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
