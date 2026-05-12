//! Keyboard input — xkbcommon for layout + key translation, password
//! buffer with zeroize.

use std::os::fd::OwnedFd;
use tracing::{debug, warn};
use wayland_client::{
    QueueHandle, WEnum,
    protocol::{wl_keyboard, wl_keyboard::KeymapFormat},
};
use xkbcommon::xkb;
use zeroize::Zeroize;

use crate::state::MlockState;

pub struct SeatState {
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub xkb_context: xkb::Context,
    pub xkb_keymap: Option<xkb::Keymap>,
    pub xkb_state: Option<xkb::State>,
    pub password: String,
    pub fail_message: Option<String>,
    /// True when Caps Lock is engaged — shown in the UI so the user
    /// notices before submitting a wrong password.
    pub caps_lock: bool,
    /// Number of failed auth attempts in this session. Visible to
    /// the user as a deterrent + warning.
    pub fail_count: u32,
    /// While `Some(deadline)`, the card is shaking. The renderer
    /// uses Instant::now() vs deadline to compute the offset.
    pub shake_until: Option<std::time::Instant>,
}

impl SeatState {
    pub fn new() -> Self {
        Self {
            keyboard: None,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_keymap: None,
            xkb_state: None,
            password: String::new(),
            fail_message: None,
            caps_lock: false,
            fail_count: 0,
            shake_until: None,
        }
    }

    pub fn is_shaking(&self) -> bool {
        match self.shake_until {
            Some(t) => std::time::Instant::now() < t,
            None => false,
        }
    }
}

impl Drop for SeatState {
    fn drop(&mut self) {
        // Defence in depth: scrub the password buffer when the seat
        // state is dropped (process exit on auth success).
        self.password.zeroize();
    }
}

pub fn handle_keyboard_event(
    state: &mut MlockState,
    event: wl_keyboard::Event,
    qh: &QueueHandle<MlockState>,
) {
    match event {
        wl_keyboard::Event::Keymap { format, fd, size } => {
            if !matches!(format, WEnum::Value(KeymapFormat::XkbV1)) {
                warn!("unsupported keymap format: {format:?}");
                return;
            }
            load_keymap(&mut state.seat_state, fd, size);
        }
        wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Leave { .. } => {
            // Focus changes aren't actionable for the locker — we
            // grab all input via session_lock anyway.
        }
        wl_keyboard::Event::Key {
            key,
            state: key_state,
            ..
        } => {
            if !matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed)) {
                return;
            }
            handle_keypress(state, key, qh);
        }
        wl_keyboard::Event::Modifiers {
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
            ..
        } => {
            if let Some(xkb_state) = state.seat_state.xkb_state.as_mut() {
                xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                // Re-read caps lock effective state.
                let caps = xkb_state
                    .mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_EFFECTIVE);
                if state.seat_state.caps_lock != caps {
                    state.seat_state.caps_lock = caps;
                    state.request_redraw_all();
                }
            }
        }
        _ => {}
    }
}

fn load_keymap(seat: &mut SeatState, fd: OwnedFd, size: u32) {
    let keymap = unsafe {
        xkb::Keymap::new_from_fd(
            &seat.xkb_context,
            fd,
            size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    };
    match keymap {
        Ok(Some(km)) => {
            let xkb_state = xkb::State::new(&km);
            seat.xkb_state = Some(xkb_state);
            seat.xkb_keymap = Some(km);
            debug!("keymap loaded ({size} bytes)");
        }
        Ok(None) | Err(_) => warn!("failed to parse compositor keymap"),
    }
}

fn handle_keypress(state: &mut MlockState, key: u32, qh: &QueueHandle<MlockState>) {
    // Wayland's keycodes are evdev (+8 from xkb). xkbcommon expects
    // the evdev-+8 form already.
    let keycode = xkb::Keycode::new(key + 8);

    // Read the keysym BEFORE we mutate state — borrow checker.
    let (keysym, utf8) = {
        let Some(xkb_state) = state.seat_state.xkb_state.as_ref() else {
            return;
        };
        let sym = xkb_state.key_get_one_sym(keycode);
        let text = xkb_state.key_get_utf8(keycode);
        (sym, text)
    };

    // Special keys: Backspace, Enter, Escape, Ctrl+U.
    match keysym {
        xkb::Keysym::BackSpace => {
            state.seat_state.password.pop();
            state.seat_state.fail_message = None;
            state.request_redraw_all();
            return;
        }
        xkb::Keysym::Return | xkb::Keysym::KP_Enter => {
            state.try_authenticate(qh);
            state.request_redraw_all();
            return;
        }
        xkb::Keysym::Escape => {
            state.seat_state.password.clear();
            state.seat_state.fail_message = None;
            state.request_redraw_all();
            return;
        }
        _ => {}
    }

    // Normal text input.
    if !utf8.is_empty() && !utf8.chars().all(|c| c.is_control()) {
        state.seat_state.password.push_str(&utf8);
        state.seat_state.fail_message = None;
        state.request_redraw_all();
    }

    let _ = qh;
}
