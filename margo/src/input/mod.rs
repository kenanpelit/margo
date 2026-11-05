#![allow(dead_code)]
/// Input handling: keyboard, pointer, touch, gesture, tablet.
/// This module contains the state types for input devices and the logic for
/// dispatching raw input events to compositor actions.
use margo_config::{Config, KeyBinding, Modifiers};

pub mod grabs;
pub mod touch;

// ── Keyboard state ────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct KeyboardState {
    /// Currently pressed modifier mask.
    pub modifiers: Modifiers,
    /// Current key mode name (e.g. "default", "resize").
    pub mode: String,
    /// Repeat rate (keys/sec).
    pub repeat_rate: i32,
    /// Repeat delay (ms before first repeat).
    pub repeat_delay: i32,
    /// Keycodes whose press a compositor binding swallowed. Their
    /// release is swallowed too, so a client never gets a release for a
    /// key it never saw pressed (mango `6a73450b`: Alt+Space's Space
    /// release leaking into Chrome).
    pub swallowed_keys: std::collections::HashSet<u32>,
}

impl KeyboardState {
    pub fn new(config: &Config) -> Self {
        KeyboardState {
            mode: "default".to_string(),
            repeat_rate: config.repeat_rate,
            repeat_delay: config.repeat_delay,
            ..Default::default()
        }
    }
}

// ── Pointer state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    #[default]
    Normal,
    Pressed,
    Move,
    Resize,
    Pan,
}

#[derive(Debug, Default)]
pub struct PointerState {
    pub x: f64,
    pub y: f64,
    pub motion_events: u64,
    pub mode: CursorMode,
    pub grab_x: f64,
    pub grab_y: f64,
    /// Monitor index the pointer was last on. Used to gate
    /// `mark_state_dirty()` to *crossings* only — motion within
    /// the same monitor stays cheap; crossings rewrite state snapshot
    /// so mshell's `active_monitor_name()` sees the new output
    /// even when no client is focused (empty monitor / sloppy
    /// focus into empty space).
    pub last_monitor: Option<usize>,
}

// ── Touch state ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct TouchPoint {
    pub id: i32,
    pub x: f64,
    pub y: f64,
    pub start_x: f64,
    pub start_y: f64,
    pub start_time: u32,
}

/// One finger that has been lifted off the screen mid-gesture. Held in
/// `TouchState::releases` until every finger is up — at that point
/// `handle_touch_up` averages the displacement across releases and
/// derives a swipe motion code, the same way the touchpad path does.
#[derive(Debug, Clone)]
pub struct TouchRelease {
    pub start_x: f64,
    pub start_y: f64,
    pub end_x: f64,
    pub end_y: f64,
}

#[derive(Debug, Default)]
pub struct TouchState {
    pub points: Vec<TouchPoint>,
    /// Set as soon as ≥ 2 fingers are simultaneously down — locks in
    /// "this is going to be a multi-finger gesture, accumulate
    /// releases for end-of-gesture analysis". Cleared when every
    /// finger is up.
    pub gesture_armed: bool,
    pub releases: Vec<TouchRelease>,
    /// margo took this touch sequence for a `gesturebind` (the client got
    /// `wl_touch.cancel`); nothing reaches clients until every finger is
    /// up. See `input::touch`.
    pub claimed: bool,
}

// ── Gesture state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct GestureState {
    /// A touchpad swipe margo keeps for its `gesturebind`s (a bind uses
    /// its finger count): accumulated here, resolved at the end.
    pub swipe_active: bool,
    pub fingers: u32,
    pub dx: f64,
    pub dy: f64,
    /// Gestures whose begin went to the client under the pointer through
    /// `zwp_pointer_gestures_v1`; their updates and end follow it.
    pub swipe_forwarded: bool,
    pub pinch_forwarded: bool,
    pub hold_forwarded: bool,
}

// ── Keybinding matching ───────────────────────────────────────────────────────

/// Find a key binding matching the given modifiers, keysym, and mode.
pub fn find_keybinding<'a>(
    bindings: &'a [KeyBinding],
    mods: Modifiers,
    keysym: u32,
    keycode: u32,
    mode: &str,
    is_locked: bool,
) -> Option<&'a KeyBinding> {
    bindings
        .iter()
        .find(|kb| keybinding_matches(kb, mods, keysym, keycode, mode, is_locked))
}

/// Whether a bind may run while the session is locked: the `force_unlock`
/// recovery bind and the binds the user tagged `l` (lock_apply).
pub fn usable_over_lock(kb: &KeyBinding) -> bool {
    kb.lock_apply || matches!(kb.action.as_str(), "force_unlock" | "force-unlock")
}

/// The bind a key press runs over the lock screen, if any. `keysyms` are
/// tried in order (the handler passes the modified sym first, then the
/// raw / latin syms). Only [`usable_over_lock`] binds count, so a mode's
/// ordinary bind can't shadow a lock bind on the same keys. Binds of the
/// current keymode come first, then the `default` mode's: the lock screen
/// is not part of a keymode, and a non-default mode ("passthrough" for a
/// VM, "resize") must not leave the volume keys or `force_unlock` dead
/// while locked.
pub fn find_lock_binding<'a>(
    bindings: &'a [KeyBinding],
    mods: Modifiers,
    keysyms: &[u32],
    keycode: u32,
    mode: &str,
) -> Option<&'a KeyBinding> {
    let modes: &[&str] = if mode == "default" {
        &["default"]
    } else {
        &[mode, "default"]
    };
    modes.iter().find_map(|mode| {
        keysyms.iter().find_map(|&sym| {
            bindings.iter().find(|kb| {
                usable_over_lock(kb) && keybinding_matches(kb, mods, sym, keycode, mode, false)
            })
        })
    })
}

/// Whether `setkeymode` may enter `mode`: `default`, or a mode the config
/// declares binds for. Any other mode has no binds of its own, so only the
/// `common` binds would stay live and no key could leave it again.
pub fn keymode_exists(bindings: &[KeyBinding], mode: &str) -> bool {
    mode == "default" || bindings.iter().any(|kb| kb.mode == mode)
}

fn keybinding_matches(
    kb: &KeyBinding,
    mods: Modifiers,
    keysym: u32,
    keycode: u32,
    mode: &str,
    is_locked: bool,
) -> bool {
    // mode check
    let mode_ok = kb.is_common_mode || kb.mode == mode || (kb.is_default_mode && mode == "default");

    // lock check
    let lock_ok = if is_locked { kb.lock_apply } else { true };

    if !mode_ok || !lock_ok {
        return false;
    }

    // modifier check
    if kb.modifiers != mods {
        return false;
    }

    // key check
    use margo_config::KeyType;
    match kb.key.key_type {
        KeyType::Sym => kb.key.keysym == keysym,
        KeyType::Code => {
            let mc = &kb.key.keycode;
            keycode != 0 && (mc.code1 == keycode || mc.code2 == keycode || mc.code3 == keycode)
        }
    }
}
