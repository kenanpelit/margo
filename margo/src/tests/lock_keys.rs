//! Caps / Num / Scroll lock state in the IPC snapshot.
//!
//! mshell's lock-key pill and toasts read `lock_keys` from `watch state`
//! instead of polling `/sys/class/leds` on a timer, so a toggle must mark
//! the state dirty (push a frame) and nothing else may.

use super::fixture::Fixture;
use serde_json::json;
use smithay::backend::input::{InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::utils::SERIAL_COUNTER;

/// evdev `KEY_CAPSLOCK` / `KEY_NUMLOCK` / `KEY_A` (+8 for xkb).
const CAPS: u32 = 58 + 8;
const NUM: u32 = 69 + 8;
const A: u32 = 30 + 8;

/// Press and release `code`, then run the post-key refresh the input
/// handler runs.
fn tap(fx: &mut Fixture, code: u32) {
    let kbd = fx.server.state.seat.get_keyboard().expect("keyboard");
    for st in [KeyState::Pressed, KeyState::Released] {
        kbd.input::<(), _>(
            &mut fx.server.state,
            Keycode::new(code),
            st,
            SERIAL_COUNTER.next_serial(),
            InputTime::now(),
            |_, _, _| FilterResult::Forward,
        );
    }
    fx.server.state.refresh_keyboard_layout();
}

fn lock_keys(fx: &Fixture) -> serde_json::Value {
    fx.server.state.build_state_snapshot()["lock_keys"].clone()
}

#[test]
fn lock_key_toggles_are_published_and_push_a_frame() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    assert_eq!(
        lock_keys(&fx),
        json!({ "caps": false, "num": false, "scroll": false })
    );

    fx.server.state.state_dirty.set(false);
    tap(&mut fx, CAPS);
    assert!(
        fx.server.state.state_dirty.get(),
        "caps toggle pushes a frame"
    );
    assert_eq!(
        lock_keys(&fx),
        json!({ "caps": true, "num": false, "scroll": false })
    );

    fx.server.state.state_dirty.set(false);
    tap(&mut fx, NUM);
    assert_eq!(
        lock_keys(&fx),
        json!({ "caps": true, "num": true, "scroll": false })
    );

    // Ordinary typing changes no lock state and must not push frames.
    fx.server.state.state_dirty.set(false);
    tap(&mut fx, A);
    assert!(!fx.server.state.state_dirty.get(), "plain keys stay quiet");

    tap(&mut fx, CAPS);
    assert_eq!(
        lock_keys(&fx),
        json!({ "caps": false, "num": true, "scroll": false })
    );
}
