//! DPMS (panel power) is per output: switching one panel off or on, or
//! toggling it, leaves the others' state — and what wlr-output-power
//! reports for them — alone.

use smithay::output::Output;

use super::fixture::Fixture;
use crate::protocols::output_power::OutputPowerHandler;

fn two_outputs() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (1920, 1200));
    fx.add_output("DP-3", (2560, 1440));
    fx
}

fn output(fx: &Fixture, name: &str) -> Output {
    fx.server
        .state
        .monitors
        .iter()
        .find(|m| m.name == name)
        .unwrap()
        .output
        .clone()
}

fn power_on(fx: &mut Fixture, name: &str) -> bool {
    let o = output(fx, name);
    fx.server.state.output_power_is_on(&o)
}

/// The last power request queued for `name` (what the backend applies).
fn last_request(fx: &Fixture, name: &str) -> Option<bool> {
    let o = output(fx, name);
    fx.server
        .state
        .output_pending
        .dpms
        .iter()
        .rev()
        .find(|(out, _)| *out == o)
        .map(|(_, on)| *on)
}

#[test]
fn one_panel_off_reports_only_that_panel_off() {
    let mut fx = two_outputs();
    fx.server.state.request_dpms(Some(false), Some("eDP-1"));
    assert!(!power_on(&mut fx, "eDP-1"));
    assert!(power_on(&mut fx, "DP-3"), "DP-3 was never switched off");
}

#[test]
fn toggling_a_lit_panel_switches_it_off_while_another_is_off() {
    let mut fx = two_outputs();
    fx.server.state.request_dpms(Some(false), Some("eDP-1"));
    fx.server.state.output_pending.dpms.clear();
    fx.server.state.request_dpms(None, Some("DP-3"));
    assert_eq!(last_request(&fx, "DP-3"), Some(false));
    assert!(!power_on(&mut fx, "DP-3"));
}

#[test]
fn waking_one_panel_keeps_the_other_off_and_input_still_wakes_it() {
    let mut fx = two_outputs();
    fx.server.state.request_dpms(Some(false), None);
    fx.server.state.request_dpms(Some(true), Some("eDP-1"));
    assert!(power_on(&mut fx, "eDP-1"));
    assert!(!power_on(&mut fx, "DP-3"));
    assert!(fx.server.state.any_dpms_off(), "DP-3 is still dark");

    fx.server.state.dpms_off_at = None; // past the grace period
    fx.server.state.output_pending.dpms.clear();
    assert!(fx.server.state.wake_dpms_on_input());
    assert_eq!(last_request(&fx, "DP-3"), Some(true));
    assert!(power_on(&mut fx, "DP-3"));
    assert!(!fx.server.state.any_dpms_off());
}

#[test]
fn a_bare_toggle_wakes_everything_when_any_panel_is_off() {
    let mut fx = two_outputs();
    fx.server.state.request_dpms(Some(false), Some("eDP-1"));
    fx.server.state.request_dpms(None, None);
    assert!(power_on(&mut fx, "eDP-1") && power_on(&mut fx, "DP-3"));
    fx.server.state.request_dpms(None, None);
    assert!(!power_on(&mut fx, "eDP-1") && !power_on(&mut fx, "DP-3"));
}

#[test]
fn switching_off_a_dark_output_does_not_leave_input_swallowed() {
    // A panel DPMS-off and then disabled is out of DPMS' reach: if it
    // still counted as "dark", every key press would be eaten as a wake.
    let mut fx = two_outputs();
    fx.server.state.request_dpms(Some(false), Some("eDP-1"));
    let edp = fx
        .server
        .state
        .monitors
        .iter()
        .position(|m| m.name == "eDP-1")
        .unwrap();
    fx.server.state.disable_monitor(edp);
    assert!(!fx.server.state.any_dpms_off());
    fx.server.state.dpms_off_at = None;
    assert!(!fx.server.state.wake_dpms_on_input());
}
