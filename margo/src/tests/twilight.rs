//! Twilight gamma: an unchanged sample costs nothing, and a gamma change
//! never dirties an output (GAMMA_LUT is a CRTC property, not a frame).

use margo_config::{Config, TwilightMode};

use super::fixture::Fixture;
use crate::protocols::gamma_control::GammaControlHandler;

fn fixture() -> Fixture {
    let config = Config {
        twilight: true,
        twilight_mode: TwilightMode::Static,
        ..Default::default()
    };
    let mut fx = Fixture::with_config(config);
    fx.add_output_full("DP-1", (1920, 1080), 256);
    fx.add_output_full("DP-2", (1920, 1080), 1024);
    settle(&mut fx);
    fx
}

/// Drain the gamma queue and every repaint request.
fn settle(fx: &mut Fixture) {
    let state = &mut fx.server.state;
    state.output_pending.gamma.clear();
    let tokens: Vec<_> = state
        .per_output_clocks
        .values_mut()
        .filter_map(|clock| clock.timer_token.take())
        .collect();
    for token in tokens {
        state.loop_handle.remove(token);
    }
    for clock in state.per_output_clocks.values_mut() {
        clock.dirty = false;
    }
    state.take_repaint_request();
}

fn queued(fx: &Fixture) -> Vec<(String, bool)> {
    fx.server
        .state
        .output_pending
        .gamma
        .iter()
        .map(|(o, ramp)| (o.name(), ramp.is_some()))
        .collect()
}

fn assert_nothing_dirty(fx: &mut Fixture) {
    let state = &mut fx.server.state;
    assert!(
        state.per_output_clocks.values().all(|c| !c.dirty),
        "a gamma change must not dirty an output clock"
    );
    assert!(
        !state.take_repaint_request(),
        "a gamma change must not request a repaint"
    );
}

#[test]
fn a_twilight_ramp_is_queued_without_dirtying_outputs() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    assert_eq!(
        queued(&fx),
        vec![("DP-1".to_string(), true), ("DP-2".to_string(), true)]
    );
    assert_nothing_dirty(&mut fx);
}

#[test]
fn an_unchanged_twilight_sample_queues_nothing() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    settle(&mut fx);
    fx.server.state.tick_twilight();
    assert!(queued(&fx).is_empty());
    assert_nothing_dirty(&mut fx);
}

#[test]
fn a_changed_sample_is_queued_again() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    settle(&mut fx);
    fx.server.state.config.twilight_static_temp += 500;
    fx.server.state.tick_twilight();
    assert_eq!(queued(&fx).len(), 2);
}

#[test]
fn a_new_output_gets_the_current_ramp() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    fx.add_output_full("DP-3", (1920, 1080), 256);
    settle(&mut fx);
    fx.server.state.tick_twilight();
    assert_eq!(queued(&fx), vec![("DP-3".to_string(), true)]);
}

#[test]
fn disabling_resets_once_and_reenabling_reapplies() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    settle(&mut fx);
    fx.server.state.config.twilight = false;
    fx.server.state.tick_twilight();
    assert_eq!(
        queued(&fx),
        vec![("DP-1".to_string(), false), ("DP-2".to_string(), false)]
    );
    assert_nothing_dirty(&mut fx);
    settle(&mut fx);
    fx.server.state.config.twilight = true;
    fx.server.state.tick_twilight();
    assert_eq!(queued(&fx).len(), 2);
}

#[test]
fn a_client_ramp_makes_twilight_reapply_its_own() {
    let mut fx = fixture();
    fx.server.state.tick_twilight();
    settle(&mut fx);
    let output = fx.server.state.monitors[0].output.clone();
    fx.server.state.set_gamma(&output, Some(vec![0; 256 * 3]));
    assert_nothing_dirty(&mut fx);
    settle(&mut fx);
    fx.server.state.tick_twilight();
    assert_eq!(queued(&fx), vec![("DP-1".to_string(), true)]);
}
