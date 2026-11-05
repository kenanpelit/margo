//! The built-in keyboard toggle (`libinput_config::internal_keyboard`)
//! through dispatch and the state snapshot. The fixture has no libinput
//! devices, which is exactly the "no internal keyboard found" case; the
//! device decisions are unit-tested against a captured device list.

use super::fixture::Fixture;
use crate::dispatch::{DispatchOutcome, dispatch_action};
use margo_config::Arg;

fn arg(v: Option<&str>) -> Arg {
    Arg {
        v: v.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn toggle_without_a_built_in_keyboard_changes_nothing() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    for v in [None, Some("off"), Some("toggle"), Some("on"), Some("bogus")] {
        assert_eq!(
            dispatch_action(&mut fx.server.state, "internal_keyboard", &arg(v)),
            DispatchOutcome::Handled
        );
        assert!(
            !fx.server.state.input_keyboard.internal_keyboard_off,
            "{v:?}"
        );
    }
    let snap = fx.server.state.build_state_snapshot();
    let ikb = &snap["internal_keyboard"];
    assert!(ikb["name"].is_null(), "{ikb}");
    assert_eq!(ikb["disabled"], false);
    assert_eq!(ikb["external"], false);
    assert_eq!(ikb["auto"], false);
}

#[test]
fn reload_with_auto_and_no_keyboards_keeps_it_on() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.internal_keyboard_auto = true;
    fx.server.state.internal_keyboard_after_reload(false);
    assert!(!fx.server.state.input_keyboard.internal_keyboard_off);
}
