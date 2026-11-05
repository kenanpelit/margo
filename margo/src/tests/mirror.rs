//! Output mirroring on the compositor: the target leaves the layout but
//! stays lit, either end unplugging or being switched by hand ends it,
//! and the session is published over IPC.

use margo_config::Arg;

use super::fixture::Fixture;
use crate::dispatch::{DispatchOutcome, dispatch_action};

/// Owner's setup: DP-3 2560×1440 + the ThinkPad's eDP-1 1920×1200.
fn laptop_and_monitor() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("DP-3", (2560, 1440));
    fx.add_output("eDP-1", (1920, 1200));
    fx
}

fn idx(fx: &Fixture, name: &str) -> usize {
    fx.server
        .state
        .monitors
        .iter()
        .position(|m| m.name == name)
        .unwrap()
}

fn strings(v: &str, v2: &str) -> Arg {
    Arg {
        v: Some(v.to_string()),
        v2: Some(v2.to_string()),
        ..Arg::default()
    }
}

#[test]
fn mirroring_takes_the_target_out_of_the_layout_but_keeps_it_lit() {
    let mut fx = laptop_and_monitor();
    let dp3 = fx.server.state.monitors[idx(&fx, "DP-3")].output.clone();

    assert_eq!(
        dispatch_action(
            &mut fx.server.state,
            "mirror_output",
            &strings("eDP-1", "DP-3")
        ),
        DispatchOutcome::Handled
    );
    let st = &fx.server.state;
    assert!(st.mirror.is_target("DP-3") && st.mirror.is_source("eDP-1"));
    assert!(!st.monitors[idx(&fx, "DP-3")].enabled);
    assert!(
        !st.space.outputs().any(|o| *o == dp3),
        "the target leaves the space: no pointer, windows or layers on it"
    );
    assert!(
        !st.output_pending
            .dpms
            .iter()
            .any(|(o, on)| *o == dp3 && !on),
        "the target panel must not be powered down — it shows the mirror"
    );

    dispatch_action(&mut fx.server.state, "stop_mirror", &Arg::default());
    let st = &fx.server.state;
    assert!(st.mirror.active().is_none());
    assert!(st.monitors[idx(&fx, "DP-3")].enabled);
    assert!(st.space.outputs().any(|o| *o == dp3));
}

#[test]
fn a_bare_toggle_mirrors_the_laptop_onto_the_monitor_and_back() {
    let mut fx = laptop_and_monitor();
    dispatch_action(&mut fx.server.state, "toggle_mirror", &Arg::default());
    assert!(fx.server.state.mirror.is_target("DP-3"));
    assert!(fx.server.state.mirror.is_source("eDP-1"));
    dispatch_action(&mut fx.server.state, "toggle_mirror", &Arg::default());
    assert!(fx.server.state.mirror.active().is_none());
    assert!(fx.server.state.monitors.iter().all(|m| m.enabled));
}

#[test]
fn unplugging_the_source_stops_and_restores_the_target() {
    let mut fx = laptop_and_monitor();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    let edp = fx.server.state.monitors[idx(&fx, "eDP-1")].output.clone();
    fx.server.state.remove_output(&edp);
    let st = &fx.server.state;
    assert!(st.mirror.active().is_none());
    assert!(
        st.monitors[idx(&fx, "DP-3")].enabled,
        "DP-3 is the desktop again"
    );
}

#[test]
fn unplugging_the_target_stops_mirroring() {
    let mut fx = laptop_and_monitor();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    let dp3 = fx.server.state.monitors[idx(&fx, "DP-3")].output.clone();
    fx.server.state.remove_output(&dp3);
    assert!(fx.server.state.mirror.active().is_none());
    assert!(fx.server.state.monitors.iter().all(|m| m.name != "DP-3"));
}

#[test]
fn switching_either_end_by_hand_ends_the_mirror() {
    // Enabling the target (wlr-randr --on, enable_output) puts it back.
    let mut fx = laptop_and_monitor();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    let dp3 = idx(&fx, "DP-3");
    fx.server.state.enable_monitor(dp3);
    assert!(fx.server.state.mirror.active().is_none());
    assert!(fx.server.state.monitors[dp3].enabled);

    // Disabling the source would leave nothing to show: the mirror ends
    // and the source is then switched off as asked.
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    let edp = idx(&fx, "eDP-1");
    fx.server.state.disable_monitor(edp);
    let st = &fx.server.state;
    assert!(st.mirror.active().is_none());
    assert!(st.monitors[dp3].enabled);
    assert!(!st.monitors[edp].enabled);
}

#[test]
fn a_refused_request_is_published_and_cleared_by_the_next_good_one() {
    let mut fx = laptop_and_monitor();
    dispatch_action(
        &mut fx.server.state,
        "mirror_output",
        &strings("eDP-1", "DP-9"),
    );
    let snap = fx.server.state.ipc_topic("mirror", &[]);
    assert_eq!(snap["active"], serde_json::Value::Null);
    assert_eq!(snap["error"], "no output named DP-9");

    dispatch_action(
        &mut fx.server.state,
        "mirror_output",
        &strings("eDP-1", "DP-3"),
    );
    let snap = fx.server.state.ipc_topic("mirror", &[]);
    assert_eq!(snap["active"]["source"], "eDP-1");
    assert_eq!(snap["active"]["target"], "DP-3");
    assert_eq!(snap["error"], serde_json::Value::Null);
    let outputs = fx.server.state.ipc_topic("monitors", &[]);
    let dp3 = outputs["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "DP-3")
        .unwrap();
    assert_eq!(dp3["mirroring"], true);
}

#[test]
fn reversing_the_direction_swaps_which_output_is_out_of_the_layout() {
    let mut fx = laptop_and_monitor();
    fx.server
        .state
        .start_mirror(Some("eDP-1"), Some("DP-3"))
        .unwrap();
    fx.server
        .state
        .start_mirror(Some("DP-3"), Some("eDP-1"))
        .unwrap();
    let st = &fx.server.state;
    assert!(st.mirror.is_target("eDP-1"));
    assert!(st.monitors[idx(&fx, "DP-3")].enabled);
    assert!(!st.monitors[idx(&fx, "eDP-1")].enabled);
}

#[test]
fn naming_one_output_while_mirroring_finds_the_pair() {
    // The target reads as disabled while mirroring; the default pair used
    // to see one enabled output and refuse with "needs two enabled outputs".
    let mut fx = laptop_and_monitor();
    let st = &mut fx.server.state;
    st.start_mirror(Some("eDP-1"), Some("DP-3")).unwrap();

    // Repeating the source: already mirroring exactly that.
    assert_eq!(st.start_mirror(Some("eDP-1"), None), Ok(()));
    assert!(st.mirror.is_source("eDP-1") && st.mirror.is_target("DP-3"));
    assert_eq!(st.start_mirror(None, Some("DP-3")), Ok(()));
    assert!(st.mirror.is_source("eDP-1") && st.mirror.is_target("DP-3"));

    // Naming the current target as source swaps the direction.
    assert_eq!(st.start_mirror(Some("DP-3"), None), Ok(()));
    assert!(st.mirror.is_source("DP-3") && st.mirror.is_target("eDP-1"));
}

fn wl_outputs(fx: &mut Fixture, id: super::client::ClientId) -> usize {
    fx.roundtrip(id);
    fx.client(id)
        .global_names()
        .iter()
        .filter(|n| *n == "wl_output")
        .count()
}

/// A mirror target is out of the layout but lit with the mirror: it keeps
/// its wl_output (screencopy, casts and output-bound clients use it).
/// Switching it off by hand ends the mirror and withdraws it like any
/// disabled output.
#[test]
fn a_mirror_target_keeps_its_wl_output_until_it_is_switched_off() {
    let mut fx = laptop_and_monitor();
    let id = fx.add_client();
    assert_eq!(wl_outputs(&mut fx, id), 2);
    dispatch_action(
        &mut fx.server.state,
        "mirror_output",
        &strings("eDP-1", "DP-3"),
    );
    assert!(fx.server.state.mirror.is_target("DP-3"));
    assert_eq!(wl_outputs(&mut fx, id), 2, "the target stays advertised");
    dispatch_action(&mut fx.server.state, "stop_mirror", &Arg::default());
    assert_eq!(wl_outputs(&mut fx, id), 2);

    dispatch_action(
        &mut fx.server.state,
        "mirror_output",
        &strings("eDP-1", "DP-3"),
    );
    let dp3 = idx(&fx, "DP-3");
    assert!(fx.server.state.disable_monitor(dp3));
    assert!(fx.server.state.mirror.active().is_none());
    assert_eq!(wl_outputs(&mut fx, id), 1, "switched off for real");
}
