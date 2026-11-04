//! `focused_opacity` / `unfocused_opacity` follow a config reload.

use margo_config::{Config, WindowRule};

use super::fixture::Fixture;
use crate::state::WindowRuleReason;

fn map(fx: &mut Fixture, app_id: &str) {
    let id = fx.add_client();
    let (toplevel, surface) = fx.client(id).create_toplevel();
    toplevel.set_app_id(app_id.into());
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

fn opacity(fx: &Fixture, app_id: &str) -> (f32, f32) {
    let c = fx
        .server
        .state
        .clients
        .iter()
        .find(|c| c.app_id == app_id)
        .expect("client mapped");
    (c.focused_opacity, c.unfocused_opacity)
}

#[test]
fn a_reload_moves_open_windows_to_the_new_global_opacity_but_keeps_rule_overrides() {
    let config = Config {
        window_rules: vec![WindowRule {
            id: Some("ruled".into()),
            focused_opacity: Some(0.3),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut fx = Fixture::with_config(config);
    fx.add_output("DP-1", (1920, 1080));
    map(&mut fx, "plain");
    map(&mut fx, "ruled");
    assert_eq!(opacity(&fx, "plain"), (1.0, 1.0));
    assert_eq!(opacity(&fx, "ruled"), (0.3, 1.0));

    // What `reload_config` does after swapping in the new config.
    let state = &mut fx.server.state;
    state.config.focused_opacity = 0.8;
    state.config.unfocused_opacity = 0.5;
    state.reset_client_opacity_to_config();
    for idx in 0..state.clients.len() {
        state.reapply_rules(idx, WindowRuleReason::Reload);
    }

    assert_eq!(opacity(&fx, "plain"), (0.8, 0.5));
    assert_eq!(opacity(&fx, "ruled"), (0.3, 0.5));
}
