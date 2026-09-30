use tracing::info;
use wayland_client::{Connection, EventQueue, protocol::wl_keyboard::KeyState};
use xkbcommon::xkb;

use crate::service::host::KeyboardHandle;

use super::session::SessionState;

pub struct VirtualKeyboard {
    session_state: SessionState,
    event_queue: EventQueue<SessionState>,
    modifiers: u32,
    locks: u32,
    /// The xkb layout group to report on every `modifiers()` request —
    /// resolved once from `keymap` at construction. Previously hardcoded
    /// to 0: on a multi-layout config with a non-zero group active,
    /// tapping any on-screen modifier/lock key would silently reset the
    /// focused client's active layout group back to 0.
    group: u32,
}

impl VirtualKeyboard {
    /// `keymap` is margo's active keymap (already compiled once by the
    /// caller — see `service::host::run`, which also feeds it to
    /// `layout::live_legends`), or `None` if it couldn't be read; `None`
    /// falls back to a bare "us" keymap and group 0 in
    /// `session::get_keymap_as_file`.
    pub fn new(keymap: Option<xkb::Keymap>) -> Self {
        let conn = Connection::connect_to_env().unwrap();
        let display = conn.display();

        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();

        let _registry = display.get_registry(&qh, ());

        let group = keymap
            .as_ref()
            .map(crate::xkb_config::active_layout_group)
            .unwrap_or(0);

        let mut state = SessionState {
            keyboard_manager: None,
            keyboard: None,
            seat: None,
            keymap,
        };

        //bind seat and virtual keyboard manager
        event_queue.roundtrip(&mut state).unwrap();
        //create virtual keyboard by seat and manager
        event_queue.roundtrip(&mut state).unwrap();

        Self {
            session_state: state,
            event_queue,
            modifiers: 0,
            locks: 0,
            group,
        }
    }
}

impl KeyboardHandle for VirtualKeyboard {
    fn key_press(&mut self, key: evdev::KeyCode) {
        if let Some(keyboard) = &self.session_state.keyboard {
            info!("Key Pressed: {:?}", key);
            keyboard.key(0, key.code().into(), KeyState::Pressed.into());
            self.event_queue.roundtrip(&mut self.session_state).unwrap();
        }
    }

    fn key_release(&mut self, key: evdev::KeyCode) {
        if let Some(keyboard) = &self.session_state.keyboard {
            info!("Key Released: {:?}", key);
            keyboard.key(0, key.code().into(), KeyState::Released.into());
            self.event_queue.roundtrip(&mut self.session_state).unwrap();
        }
    }

    fn append_mod(&mut self, key: evdev::KeyCode) {
        info!("Mod Appended: {:?}", key);
        let mod_code = Self::map_mod_key(key);
        self.modifiers |= mod_code;

        self.update_state();
    }

    fn remove_mod(&mut self, key: evdev::KeyCode) {
        info!("Mod Removed: {:?}", key);
        let mod_code = Self::map_mod_key(key);
        self.modifiers &= !mod_code;

        self.update_state();
    }

    fn append_lock(&mut self, key: evdev::KeyCode) {
        info!("Lock Appended: {:?}", key);
        let lock_code = Self::map_lock_key(key);
        self.locks |= lock_code;

        self.update_state();
    }

    fn remove_lock(&mut self, key: evdev::KeyCode) {
        info!("Lock Removed: {:?}", key);
        let lock_code = Self::map_lock_key(key);
        self.locks &= !lock_code;

        self.update_state();
    }

    fn destroy(&mut self) {
        // `take()`, not a borrow: clears the field so a second `destroy()`
        // call (e.g. the close button and the IPC "quit" listener firing
        // within the same tick) is a safe no-op instead of resending a
        // destroy request on an already-destroyed proxy, which the
        // roundtrip below would otherwise turn into a panic on the
        // resulting protocol error.
        if let Some(keyboard) = self.session_state.keyboard.take() {
            info!("Destroying Virtual Keyboard.");
            keyboard.destroy();
            self.event_queue.roundtrip(&mut self.session_state).unwrap();
        }
    }
}

impl VirtualKeyboard {
    fn update_state(&mut self) {
        if let Some(keyboard) = &self.session_state.keyboard {
            keyboard.modifiers(self.modifiers, 0, self.locks, self.group);
            self.event_queue.roundtrip(&mut self.session_state).unwrap();
        }
    }

    fn map_mod_key(key: evdev::KeyCode) -> u32 {
        match key {
            evdev::KeyCode::KEY_LEFTCTRL | evdev::KeyCode::KEY_RIGHTCTRL => 4,
            evdev::KeyCode::KEY_LEFTMETA | evdev::KeyCode::KEY_RIGHTMETA => 64,
            evdev::KeyCode::KEY_LEFTSHIFT | evdev::KeyCode::KEY_RIGHTSHIFT => 1,
            evdev::KeyCode::KEY_LEFTALT | evdev::KeyCode::KEY_RIGHTALT => 8,
            _ => 0,
        }
    }

    fn map_lock_key(key: evdev::KeyCode) -> u32 {
        match key {
            evdev::KeyCode::KEY_CAPSLOCK => 2,
            evdev::KeyCode::KEY_NUMLOCK => 256,
            evdev::KeyCode::KEY_SCROLLLOCK => 32768,
            _ => 0,
        }
    }
}
