//! Keyboard input: key binds, the lock screen and keyboard-shortcuts
//! inhibitors, virtual keyboards and their keymaps, modifier helpers and
//! the exclusive-keyboard layer-shell focus.

use smithay::{
    backend::input::{InputBackend, InputEvent, KeyState, KeyboardKeyEvent},
    desktop::layer_map_for_output,
    input::keyboard::{FilterResult, ModifiersState},
    utils::{Point, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer},
};
use tracing::info;

use crate::{
    input::{find_keybinding, find_lock_binding},
    state::{FocusTarget, MargoState},
};

/// Put the configured xkb keymap back after a virtual keyboard's own
/// keymap was active.
fn restore_configured_keymap(state: &mut MargoState) {
    state.active_virtual_keymap = None;
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let rules = state.config.xkb_rules.clone();
    let config = smithay::input::keyboard::XkbConfig {
        rules: &rules.rules,
        model: &rules.model,
        layout: &rules.layout,
        variant: &rules.variant,
        options: rules.options_or_none(),
    };
    if let Err(e) = keyboard.set_xkb_config(state, config) {
        tracing::warn!("restoring the configured keymap failed: {e:?}");
    }
    state.apply_lock_keys();
}

/// Load `device`'s keymap into the seat if it isn't already; a device
/// without one (`no_keymap`) uses the configured keymap.
fn activate_virtual_keymap(
    state: &mut MargoState,
    device: &smithay::wayland::virtual_keyboard::VirtualKeyboardDevice,
) {
    match device.keymap() {
        Some(keymap) => {
            if state.active_virtual_keymap.as_ref() == Some(&keymap) {
                return;
            }
            let Some(keyboard) = state.seat.get_keyboard() else {
                return;
            };
            match keyboard.set_keymap_from_string(state, keymap.to_string()) {
                Ok(()) => state.active_virtual_keymap = Some(keymap),
                Err(e) => tracing::warn!("virtual keyboard keymap rejected: {e:?}"),
            }
        }
        None => {
            if state.active_virtual_keymap.is_some() {
                restore_configured_keymap(state);
            }
        }
    }
}

/// `zwp_virtual_keyboard_v1` input (wtype, wayvnc, mkeys, IMEs). smithay
/// delivers it as a regular input device, so keys go through the same
/// path as a physical keyboard — bindings included, minus anything that
/// would end or escape the session (see `handle_keyboard`).
pub(crate) fn handle_virtual_keyboard(
    state: &mut MargoState,
    event: InputEvent<smithay::wayland::virtual_keyboard::VirtualKeyboardBackend>,
) {
    use smithay::backend::input::{Device, Event};
    use smithay::wayland::input_method::InputMethodSeat;
    use smithay::wayland::virtual_keyboard::VirtualKeyboardSpecialEvent as Special;
    match event {
        InputEvent::Keyboard { event } => {
            let device = event.device();
            activate_virtual_keymap(state, &device);
            // An IME forwarding keys it didn't consume through its own
            // virtual keyboard must not get them back through its grab.
            let grab_client = state.seat.input_method().keyboard_grab_client();
            if grab_client.is_some() && grab_client == device.client() {
                if let Some(keyboard) = state.seat.get_keyboard() {
                    keyboard.input_forward_bypassing_grab(
                        state,
                        event.key_code(),
                        event.state(),
                        SERIAL_COUNTER.next_serial(),
                        event.time(),
                        true,
                    );
                }
                return;
            }
            state.keyboard_source = device.source();
            state.virtual_input = true;
            handle_keyboard(state, event);
            state.virtual_input = false;
            state.keyboard_source = smithay::input::keyboard::KeyboardSource::MAIN;
        }
        InputEvent::Special(Special::Modifiers {
            device,
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
        }) => {
            activate_virtual_keymap(state, &device);
            state.virtual_modifier_devices.insert(device.id());
            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.with_xkb_state(state, |mut context| {
                    context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group)
                });
            }
        }
        InputEvent::DeviceRemoved { device } => {
            let set_modifiers = state.virtual_modifier_devices.remove(&device.id());
            if device.keymap().is_some()
                && state.active_virtual_keymap.as_ref() == device.keymap().as_ref()
            {
                // Rebuilds the xkb state, which also drops its modifiers.
                restore_configured_keymap(state);
            } else if set_modifiers && let Some(keyboard) = state.seat.get_keyboard() {
                // Seat-keymap device that set modifiers directly: release
                // what it left held, keep the locks.
                let locked = keyboard.modifier_state().serialized.locked;
                keyboard.with_xkb_state(state, |mut context| {
                    context.set_modifier_mask(0, 0, locked, 0)
                });
            }
        }
        _ => {}
    }
}

/// Swallow a pressed key and remember it, so its release is swallowed
/// too. (Portal global shortcuts track their own releases.)
fn swallow_key(state: &mut MargoState, keycode: u32) -> FilterResult<()> {
    state.input_keyboard.swallowed_keys.insert(keycode);
    FilterResult::Intercept(())
}

pub(super) fn handle_keyboard<B: InputBackend, E: KeyboardKeyEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time();
    let key_state = event.state();
    let keycode = event.key_code();

    // A physical key after a virtual keyboard swapped in its own keymap:
    // put the configured one back first.
    if !state.virtual_input && state.active_virtual_keymap.is_some() {
        restore_configured_keymap(state);
    }

    if let Some(keyboard) = state.seat.get_keyboard() {
        // While the session is locked, the lock surface MUST keep
        // keyboard focus — never let an exclusive layer surface
        // (noctalia bar / launcher / OSD with `keyboard-interactivity:
        // exclusive`) hijack focus, otherwise the user can't type the
        // password into the lock screen.
        if !state.session_locked {
            if let Some(focus) = exclusive_keyboard_layer(state) {
                let current_focus = keyboard.current_focus();
                if current_focus.as_ref() != Some(&focus) {
                    state.set_keyboard_focus(Some(focus), serial);
                }
            }
        }

        let source = state.keyboard_source;
        keyboard.input_from_source(
            source,
            state,
            keycode,
            key_state,
            serial,
            time,
            |state, modifiers, handle| {
                // A key whose press a binding swallowed: swallow its release
                // too (see `KeyboardState::swallowed_keys`). Checked before
                // everything else so a lock / focus change in between
                // can't leak it either.
                if key_state == KeyState::Released
                    && state.input_keyboard.swallowed_keys.remove(&keycode.raw())
                {
                    return FilterResult::Intercept(());
                }

                // While the session is locked, no compositor keybindings —
                // EVERY key (press or release) goes straight through to the
                // focused lock surface so the user can type their password.
                // Without `Forward` here the lock screen never sees a single
                // keystroke and there's no way to unlock.
                if state.session_locked {
                    // Whitelisted recovery bind: a `force_unlock`
                    // keybind is matched even while the session is
                    // locked, so the user can bring a crashed locker
                    // back without a TTY. It restarts the locker and
                    // never unlocks (see `state::lock_guard`).
                    // Everything else is forwarded to the lock surface.
                    // Synthetic keys never reach a binding over the lock:
                    // a client with a virtual keyboard must not be able to
                    // `force_unlock` or run lock-safe actions.
                    if key_state == KeyState::Pressed && !state.virtual_input {
                        let mods = smithay_mods_to_margo(modifiers);
                        let latin = handle.raw_latin_sym_or_raw_current_sym();
                        let syms: Vec<u32> = std::iter::once(handle.modified_sym())
                            .chain(handle.raw_syms())
                            .chain(latin)
                            .map(|sym| sym.raw())
                            .collect();
                        let matched = find_lock_binding(
                            &state.config.key_bindings,
                            mods,
                            &syms,
                            keycode.raw(),
                            &state.input_keyboard.mode,
                        );
                        if let Some(kb) = matched {
                            if matches!(kb.action.as_str(), "force_unlock" | "force-unlock") {
                                tracing::warn!(
                                    "lock: force_unlock keybind hit, restarting a dead locker"
                                );
                                let action = kb.action.clone();
                                let arg = kb.arg.clone();
                                crate::dispatch::dispatch_action(state, &action, &arg);
                                return swallow_key(state, keycode.raw());
                            }
                            // Binds the user explicitly tagged `l` (lock_apply) —
                            // volume / brightness / media keys marked safe to run
                            // over the lock screen — are dispatched here and the key
                            // is swallowed so it doesn't also reach the lock surface.
                            // Every other bind falls through to the forward below, so
                            // the lock screen stays secure (no spawning terminals over
                            // it). Without this the `l` suffix was a silent no-op.
                            if kb.lock_apply {
                                let action = kb.action.clone();
                                let arg = kb.arg.clone();
                                crate::dispatch::dispatch_action(state, &action, &arg);
                                return swallow_key(state, keycode.raw());
                            }
                        }
                    }
                    // Never log WHICH key: with `log_to_file` on, keycodes
                    // typed here are the unlock password. Focus target at
                    // trace is enough to debug "lock screen gets no keys".
                    let focus = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
                    let lock_has_focus =
                        matches!(focus, Some(crate::state::FocusTarget::SessionLock(_)));
                    tracing::trace!(
                        "lock: key state={:?} focus={} action={}",
                        key_state,
                        match &focus {
                            Some(crate::state::FocusTarget::SessionLock(_)) => "SessionLock",
                            Some(crate::state::FocusTarget::LayerSurface(_)) => "LayerSurface",
                            Some(crate::state::FocusTarget::Window(_)) => "Window",
                            Some(crate::state::FocusTarget::Popup(_)) => "Popup",
                            None => "None",
                        },
                        if lock_has_focus { "forward" } else { "drop" },
                    );
                    return if lock_has_focus {
                        FilterResult::Forward
                    } else {
                        FilterResult::Intercept(())
                    };
                }

                // `zwp_keyboard_shortcuts_inhibit_v1`: if the focused
                // surface has an active inhibitor, the client (VNC /
                // RDP / VirtualBox / browser remote-desktop) wants
                // every key — including margo's own Super / Alt+Tab —
                // forwarded straight to it. Skip the keybinding match
                // entirely so the keys reach the guest. Session-lock
                // path above already returned, so this only fires in
                // the normal unlocked state.
                let inhibited = state
                    .seat
                    .get_keyboard()
                    .and_then(|kb| kb.current_focus())
                    .and_then(|f| f.wl_surface().map(|cow| cow.into_owned()))
                    .and_then(|surface| {
                        state
                            .keyboard_shortcuts_inhibiting_surfaces
                            .get(&surface)
                            .map(|i| i.is_active())
                    })
                    .unwrap_or(false);
                if inhibited {
                    return FilterResult::Forward;
                }

                // Check for compositor keybindings when key is pressed
                if key_state == KeyState::Pressed {
                    let keysym = handle.modified_sym();
                    let mods = smithay_mods_to_margo(modifiers);
                    let mode = state.input_keyboard.mode.clone();
                    // Never log WHICH key (no keysym / keycode / utf8):
                    // with `log_to_file` at debug every typed character,
                    // passwords included, would land in the log file. A
                    // matched bind is logged below.

                    let mut matched = find_keybinding(
                        &state.config.key_bindings,
                        mods,
                        keysym.raw(),
                        keycode.raw(),
                        &mode,
                        false,
                    );

                    // Fallback to raw unshifted symbols if modified_sym didn't match.
                    // This fixes bindings like `super+shift,1` where modified_sym is `!` but raw is `1`.
                    // Then the layout-agnostic latin sym: with a non-latin layout
                    // active (ru, gr, …) the raw sym is e.g. `й`, so a `super+q`
                    // bind would never fire — resolve the key through the first
                    // configured layout that has a latin keysym for it instead.
                    if matched.is_none() {
                        let latin = handle.raw_latin_sym_or_raw_current_sym();
                        for sym in handle.raw_syms().into_iter().chain(latin) {
                            matched = find_keybinding(
                                &state.config.key_bindings,
                                mods,
                                sym.raw(),
                                keycode.raw(),
                                &mode,
                                false,
                            );
                            if matched.is_some() {
                                break;
                            }
                        }
                    }

                    // Virtual keyboards drive normal bindings (that is the
                    // point of smithay routing them as input devices), but
                    // never ones that end or escape the session.
                    if state.virtual_input
                        && matched.as_ref().is_some_and(|kb| {
                            matches!(
                                kb.action.as_str(),
                                "quit" | "force_unlock" | "force-unlock" | "chvt"
                            )
                        })
                    {
                        return FilterResult::Forward;
                    }

                    if let Some(kb) = matched {
                        info!(
                            action = ?kb.action,
                            arg = ?kb.arg,
                            "keybinding match",
                        );
                        let action = kb.action.clone();
                        let arg = kb.arg.clone();
                        // Alt+Tab muscle memory: when an overview-cycle
                        // action fires, snapshot the modifier set the user
                        // is currently holding. The release dispatch
                        // below watches for any of those modifiers being
                        // let go and auto-commits the cycle's pick. This
                        // makes alt+Tab behave like Win/GNOME/Hypr —
                        // hold modifier, tap Tab, release modifier to
                        // confirm.
                        if matches!(
                            action.as_str(),
                            "overview_focus_next" | "overview_focus_prev"
                        ) {
                            let snapshot = smithay_mods_to_margo(modifiers);
                            state.overview_cycle_pending = true;
                            state.overview_cycle_modifier_mask = snapshot;
                        }
                        // Same muscle-memory for the MRU switcher (Super+Tab):
                        // snapshot the held modifier so the release handler
                        // knows when to commit the pick.
                        if matches!(action.as_str(), "mru_next" | "mru_prev" | "mruwindow") {
                            state.mru_open_mask = smithay_mods_to_margo(modifiers);
                        }
                        crate::dispatch::dispatch_action(state, &action, &arg);
                        // A keyboard keybind means the user is acting on
                        // the focused monitor, so menu opens should target
                        // it — *unless* this bind is a `spawn` (every
                        // `mshellctl menu …` open is a spawn, plus app
                        // launches). Those are neutral: they read the
                        // current active-output source instead of redefining
                        // it, so a fresh "mouse onto monitor B → open menu"
                        // still lands on B. Stamped after dispatch so
                        // `focusmon`/tag changes are already reflected.
                        if action != "spawn" {
                            state.active_output_source = crate::state::ActiveOutputSource::Focus;
                        }
                        return swallow_key(state, keycode.raw());
                    }

                    // Scroller-overview modal keys. Runs only when no
                    // keybind claimed the press, so user binds and the
                    // toggle key still win: Esc closes, Up/Down move the
                    // selection, Enter activates the selected tag.
                    // MRU switcher: Esc cancels (re-focus the original window).
                    if state.is_mru_open() && keysym.raw() == 0xff1b {
                        state.mru_cancel();
                        return swallow_key(state, keycode.raw());
                    }

                    if state.is_scroller_overview_open() {
                        const ESC: u32 = 0xff1b;
                        const UP: u32 = 0xff52;
                        const DOWN: u32 = 0xff54;
                        const RETURN: u32 = 0xff0d;
                        const KP_ENTER: u32 = 0xff8d;
                        let handled = match keysym.raw() {
                            ESC => {
                                state.close_scroller_overview();
                                true
                            }
                            UP => {
                                state.scroller_overview_select(-1);
                                true
                            }
                            DOWN => {
                                state.scroller_overview_select(1);
                                true
                            }
                            RETURN | KP_ENTER => {
                                state.scroller_overview_activate();
                                true
                            }
                            _ => false,
                        };
                        if handled {
                            return swallow_key(state, keycode.raw());
                        }
                    }

                    // xdg-desktop-portal GlobalShortcuts: consulted only
                    // after config binds and modal keys pass, so an app
                    // can never shadow a user bind or a modal Esc.
                    if state.global_shortcut_press(mods, keysym.raw(), keycode.raw()) {
                        return FilterResult::Intercept(());
                    }

                    // Reached here = a pressed key that no keybind or modal
                    // claimed, so it's forwarded to the focused client —
                    // i.e. the user is typing on the focused monitor. Make
                    // that monitor the active-output source so a later menu
                    // open targets it even across a plain typing session
                    // with the cursor parked on another output. Modifiers
                    // are skipped so the Super/Alt of a menu-open chord
                    // stays neutral (the open itself doesn't move focus).
                    if !is_modifier_keysym(keysym.raw()) {
                        state.active_output_source = crate::state::ActiveOutputSource::Focus;
                    }
                }

                // GlobalShortcuts release: the press was swallowed, so the
                // matching release must be too (a client seeing half a key
                // thinks it's stuck). Fires the portal's Deactivated.
                if key_state == KeyState::Released && state.global_shortcut_release(keycode.raw()) {
                    return FilterResult::Intercept(());
                }

                // Modifier-release auto-commit for alt+Tab cycle. We can't
                // rely on the `modifiers` parameter on a release event
                // because xkbcommon updates its modifier state AFTER the
                // filter callback runs — `modifiers.alt` is still `true`
                // when the Alt_L release reaches us. Instead, identify
                // *which* modifier was released by looking at the
                // released keysym(s) directly, subtract that bit from
                // the pending-cycle snapshot, and commit when the
                // snapshot empties.
                //
                // Alt+Tab: snap = {ALT}. Alt release → snap = {} → commit.
                // Alt+Shift+Tab walking back: snap = {ALT, SHIFT}. Shift
                // release alone → snap = {ALT} → no commit. Then Alt
                // release → snap = {} → commit. Releasing modifiers in
                // any order still confirms the pick.
                if key_state == KeyState::Released
                    && state.overview_cycle_pending
                    && (state.is_overview_open() || state.is_scroller_overview_open())
                    && !state.overview_cycle_modifier_mask.is_empty()
                {
                    let released_bit = handle
                        .raw_syms()
                        .iter()
                        .find_map(|s| released_modifier_bit(s.raw()));
                    if let Some(bit) = released_bit {
                        if state.overview_cycle_modifier_mask.contains(bit) {
                            state.overview_cycle_modifier_mask.remove(bit);
                            if state.overview_cycle_modifier_mask.is_empty() {
                                state.overview_cycle_pending = false;
                                state.overview_activate_styled();
                                return FilterResult::Intercept(());
                            }
                        }
                    }
                }

                // Same release-to-commit for the MRU window switcher. When the
                // last held modifier is let go, lock in the highlighted window.
                if key_state == KeyState::Released
                    && state.is_mru_open()
                    && state
                        .mru_switcher
                        .as_ref()
                        .is_some_and(|s| !s.modifier_mask.is_empty())
                {
                    let released_bit = handle
                        .raw_syms()
                        .iter()
                        .find_map(|s| released_modifier_bit(s.raw()));
                    if let Some(bit) = released_bit {
                        let empty = if let Some(sw) = state.mru_switcher.as_mut() {
                            if sw.modifier_mask.contains(bit) {
                                sw.modifier_mask.remove(bit);
                            }
                            sw.modifier_mask.is_empty()
                        } else {
                            false
                        };
                        if empty {
                            // Commit, but FORWARD the modifier release (don't
                            // Intercept): the just-focused window must receive
                            // the Alt/Super release too, or it thinks the
                            // modifier is still held (stuck-Alt). xkb updates
                            // its own modifier state after this filter anyway.
                            state.mru_confirm();
                        }
                    }
                }

                FilterResult::Forward
            },
        );

        // The key event may have toggled the xkb layout group (e.g.
        // a `grp:` toggle). Re-cache the active layout name so the
        // shell's keyboard-layout pill reflects the switch.
        state.refresh_keyboard_layout();
    }
}

/// Map a released modifier keysym to the matching `margo_config::Modifiers`
/// bit. We need this because xkbcommon updates the keyboard's modifier
/// state *after* the filter callback runs for a release event — so a
/// `KeyState::Released` event for `Alt_L` is delivered with
/// `ModifiersState::alt` still set to `true`. Reading the keysym we just
/// got the release for is unambiguous; we then subtract its bit from
/// our pending-cycle snapshot and commit when the snapshot empties.
fn released_modifier_bit(keysym: u32) -> Option<margo_config::Modifiers> {
    // X11 keysym constants. These are stable across xkbcommon versions
    // and avoid pulling a dedicated keysym module into this hot path.
    const SHIFT_L: u32 = 0xffe1;
    const SHIFT_R: u32 = 0xffe2;
    const CONTROL_L: u32 = 0xffe3;
    const CONTROL_R: u32 = 0xffe4;
    const META_L: u32 = 0xffe7;
    const META_R: u32 = 0xffe8;
    const ALT_L: u32 = 0xffe9;
    const ALT_R: u32 = 0xffea;
    const SUPER_L: u32 = 0xffeb;
    const SUPER_R: u32 = 0xffec;
    const HYPER_L: u32 = 0xffed;
    const HYPER_R: u32 = 0xffee;
    match keysym {
        SHIFT_L | SHIFT_R => Some(margo_config::Modifiers::SHIFT),
        CONTROL_L | CONTROL_R => Some(margo_config::Modifiers::CTRL),
        ALT_L | ALT_R | META_L | META_R => Some(margo_config::Modifiers::ALT),
        SUPER_L | SUPER_R | HYPER_L | HYPER_R => Some(margo_config::Modifiers::LOGO),
        _ => None,
    }
}

/// Is this keysym a modifier (Shift/Ctrl/Alt/Super/Hyper)? Used to keep
/// the modifier half of a menu-open chord (e.g. the Super of Super+D)
/// from counting as "the user is typing on this monitor".
fn is_modifier_keysym(keysym: u32) -> bool {
    released_modifier_bit(keysym).is_some()
}

pub(super) fn smithay_mods_to_margo(m: &ModifiersState) -> margo_config::Modifiers {
    let mut out = margo_config::Modifiers::empty();
    if m.shift {
        out |= margo_config::Modifiers::SHIFT;
    }
    if m.ctrl {
        out |= margo_config::Modifiers::CTRL;
    }
    if m.alt {
        out |= margo_config::Modifiers::ALT;
    }
    if m.logo {
        out |= margo_config::Modifiers::LOGO;
    }
    if m.caps_lock {
        out |= margo_config::Modifiers::CAPS;
    }
    out
}

fn exclusive_keyboard_layer(state: &MargoState) -> Option<FocusTarget> {
    if state.session_locked {
        // Find the lock surface on the currently focused monitor/output.
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        let output = state
            .space
            .output_under(pos)
            .next()
            .or_else(|| state.monitors.first().map(|m| &m.output));

        if let Some(output) = output {
            if let Some((_, surface)) = state
                .lock_surfaces
                .iter()
                .find(|(o, surface)| o == output && MargoState::lock_surface_ready(surface))
            {
                return Some(FocusTarget::SessionLock(surface.clone()));
            }
        }
        // Fallback to the first available lock surface.
        return state
            .lock_surfaces
            .iter()
            .find(|(_, surface)| MargoState::lock_surface_ready(surface))
            .map(|(_, s)| FocusTarget::SessionLock(s.clone()));
    }

    for layer in state.protocols.layer_shell.layer_surfaces().rev() {
        let exclusive = layer.with_cached_state(|data| {
            data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && matches!(data.layer, WlrLayer::Top | WlrLayer::Overlay)
        });

        if !exclusive {
            continue;
        }

        let mapped = state.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            map.layers()
                .find(|mapped| mapped.layer_surface() == &layer)
                .filter(|mapped| state.layer_accepts_input_on_output(output, mapped.layer()))
                .map(|mapped| mapped.layer_surface().clone())
        });

        if let Some(surface) = mapped {
            return Some(FocusTarget::LayerSurface(surface));
        }
    }

    None
}

impl smithay::wayland::virtual_keyboard::VirtualKeyboardHandler for MargoState {
    fn process_virtual_keyboard_event(
        &mut self,
        event: InputEvent<smithay::wayland::virtual_keyboard::VirtualKeyboardBackend>,
    ) {
        handle_virtual_keyboard(self, event);
    }
}

impl smithay::wayland::text_input::TextInputActivation for MargoState {}

#[cfg(test)]
mod key_log_tests {
    /// Every tracing call in the keyboard paths, with its arguments.
    fn log_calls(src: &str) -> Vec<String> {
        let mut calls = Vec::new();
        for mac in ["trace!(", "debug!(", "info!(", "warn!(", "error!("] {
            let mut from = 0;
            while let Some(at) = src[from..].find(mac) {
                let start = from + at;
                let mut depth = 0usize;
                let mut end = start;
                for (i, ch) in src[start..].char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = start + i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                calls.push(src[start..=end].to_string());
                from = end.max(start + 1);
            }
        }
        calls
    }

    /// With `log_to_file` and debug level on (typical when filing a bug),
    /// a log line per key press put every typed character, passwords
    /// included, into a world-readable log. Only modifiers, the key mode
    /// and matched binds may be logged.
    #[test]
    fn key_presses_never_log_which_key() {
        // Every file of `input_handler` (they were one file when this was
        // written); each is cut at its first test module.
        let sources = [
            include_str!("../input_handler.rs"),
            include_str!("gestures.rs"),
            include_str!("hit.rs"),
            include_str!("hot_corner.rs"),
            include_str!("keyboard.rs"),
            include_str!("pointer.rs"),
        ];
        let leaks: Vec<String> = sources
            .iter()
            .flat_map(|src| log_calls(&src[..src.find("#[cfg(test)]").unwrap_or(src.len())]))
            .filter(|call| {
                [
                    "keysym",
                    "keycode",
                    "key_code",
                    "utf8",
                    "modified_sym",
                    "raw_syms",
                ]
                .iter()
                .any(|word| call.contains(word))
            })
            .collect();
        assert!(leaks.is_empty(), "key identity in logs: {leaks:#?}");
    }
}
