#![allow(dead_code)]
/// Input handling: keyboard, pointer, touch, gesture, tablet.
/// This module contains the state types for input devices and the logic for
/// dispatching raw input events to compositor actions.
use margo_config::{Config, KeyBinding, Modifiers};

pub mod grabs;

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
    /// `write_state_file()` to *crossings* only — motion within
    /// the same monitor stays cheap; crossings rewrite state.json
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
}

// ── Gesture state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct GestureState {
    pub swipe_active: bool,
    pub pinch_active: bool,
    pub fingers: u32,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
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
    bindings.iter().find(|kb| {
        // mode check
        let mode_ok = kb.is_common_mode
            || kb.mode == mode
            || (kb.is_default_mode && mode == "default");

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
                keycode != 0
                    && (mc.code1 == keycode || mc.code2 == keycode || mc.code3 == keycode)
            }
        }
    })
}
