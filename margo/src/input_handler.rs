#![allow(dead_code)]
//! Translates raw libinput / winit events into compositor actions.

use smithay::{
    backend::input::{
        Axis, ButtonState, GestureBeginEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        PointerMotionAbsoluteEvent,
    },
    desktop::{layer_map_for_output, WindowSurfaceType},
    input::{
        keyboard::{FilterResult, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer},
};
use tracing::{debug, info};

use crate::{
    input::find_keybinding,
    state::{FocusTarget, MargoState},
};

pub fn handle_input<B: InputBackend>(state: &mut MargoState, event: InputEvent<B>) {
    // Every input event resets the idle timers so swayidle / noctalia
    // see the seat as "active". `notify_activity` is a no-op when
    // there are no listeners, so this is essentially free.
    if matches!(
        event,
        InputEvent::Keyboard { .. }
            | InputEvent::PointerMotion { .. }
            | InputEvent::PointerMotionAbsolute { .. }
            | InputEvent::PointerButton { .. }
            | InputEvent::PointerAxis { .. }
            | InputEvent::GestureSwipeBegin { .. }
            | InputEvent::GestureSwipeUpdate { .. }
            | InputEvent::GestureSwipeEnd { .. }
            | InputEvent::TouchDown { .. }
            | InputEvent::TouchMotion { .. }
            | InputEvent::TouchUp { .. }
    ) {
        let seat = state.seat.clone();
        state.idle_notifier_state.notify_activity(&seat);
    }

    match event {
        InputEvent::Keyboard { event } => handle_keyboard(state, event),
        InputEvent::PointerMotion { event } => handle_pointer_motion(state, event),
        InputEvent::PointerMotionAbsolute { event } => handle_pointer_motion_abs(state, event),
        InputEvent::PointerButton { event } => handle_pointer_button(state, event),
        InputEvent::PointerAxis { event } => handle_pointer_axis(state, event),
        InputEvent::GestureSwipeBegin { event } => handle_swipe_begin(state, event),
        InputEvent::GestureSwipeUpdate { event } => handle_swipe_update(state, event),
        InputEvent::GestureSwipeEnd { event: _ } => handle_swipe_end(state),
        _ => {}
    }
}

// ── Touchpad swipe gestures ──────────────────────────────────────────────────
//
// We accumulate dx/dy across the gesture, then on End we resolve a cardinal
// direction (or diagonal) and look up `gesturebind` from the config. This
// mirrors how mango (C) and niri handle touchpad swipes.

const SWIPE_MIN_DISTANCE: f64 = 30.0;

fn handle_swipe_begin<B: InputBackend, E: GestureBeginEvent<B>>(state: &mut MargoState, event: E) {
    state.input_gesture.swipe_active = true;
    state.input_gesture.fingers = event.fingers();
    state.input_gesture.dx = 0.0;
    state.input_gesture.dy = 0.0;
}

fn handle_swipe_update<B: InputBackend, E: GestureSwipeUpdateEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    if !state.input_gesture.swipe_active {
        return;
    }
    state.input_gesture.dx += event.delta_x();
    state.input_gesture.dy += event.delta_y();
}

fn handle_swipe_end(state: &mut MargoState) {
    let g = std::mem::take(&mut state.input_gesture);
    if !g.swipe_active {
        return;
    }
    let total = (g.dx * g.dx + g.dy * g.dy).sqrt();
    if total < SWIPE_MIN_DISTANCE {
        return; // ignored, too short
    }
    // Map dx/dy to a motion code matching margo-config's parse_motion()
    // (UP=0, DOWN=1, RIGHT=2, LEFT=3, UP_RIGHT=4, UP_LEFT=5, DOWN_LEFT=6, DOWN_RIGHT=7).
    // Diagonal threshold: each axis must contribute > 40% of total magnitude.
    let ax = g.dx.abs();
    let ay = g.dy.abs();
    let diag = ax > 0.4 * total && ay > 0.4 * total;
    let motion = if diag {
        match (g.dx.is_sign_positive(), g.dy.is_sign_positive()) {
            (true, false) => 4,  // UP_RIGHT
            (false, false) => 5, // UP_LEFT
            (false, true) => 6,  // DOWN_LEFT
            (true, true) => 7,   // DOWN_RIGHT
        }
    } else if ax > ay {
        if g.dx > 0.0 { 2 } else { 3 } // RIGHT / LEFT
    } else if g.dy < 0.0 {
        0 // UP
    } else {
        1 // DOWN
    };

    let mods = state
        .seat
        .get_keyboard()
        .map(|k| smithay_mods_to_margo(&k.modifier_state()))
        .unwrap_or_else(margo_config::Modifiers::empty);

    let binding = state.config.gesture_bindings.iter().find(|b| {
        b.fingers == g.fingers && b.motion == motion && b.modifiers == mods
    });

    if let Some(binding) = binding {
        info!(
            "swipe match: fingers={} motion={} mods={:?} action={:?}",
            g.fingers, motion, mods, binding.action
        );
        let action = binding.action.clone();
        let arg = binding.arg.clone();
        crate::dispatch::dispatch_action(state, &action, &arg);
        state.request_repaint();
    } else {
        debug!(
            "swipe unmatched: fingers={} motion={} mods={:?}",
            g.fingers, motion, mods
        );
    }
}

fn handle_keyboard<B: InputBackend, E: KeyboardKeyEvent<B>>(state: &mut MargoState, event: E) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();
    let key_state = event.state();
    let keycode = event.key_code();

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
                    keyboard.set_focus(state, Some(focus), serial);
                }
            }
        }

        keyboard.input(            state,
            keycode,
            key_state,
            serial,
            time,
            |state, modifiers, handle| {
                // While the session is locked, no compositor keybindings —
                // EVERY key (press or release) goes straight through to the
                // focused lock surface so the user can type their password.
                // Without `Forward` here the lock screen never sees a single
                // keystroke and there's no way to unlock.
                if state.session_locked {
                    // Whitelisted escape hatch: a `force_unlock`
                    // keybind always wins, even while the session is
                    // locked, so the user has a way to recover from a
                    // wedged lock screen without rebooting. Everything
                    // else is forwarded straight to the lock surface.
                    if key_state == KeyState::Pressed {
                        let keysym = handle.modified_sym();
                        let mods = smithay_mods_to_margo(modifiers);
                        let mode = state.input_keyboard.mode.clone();
                        let mut matched = find_keybinding(
                            &state.config.key_bindings,
                            mods,
                            keysym.raw(),
                            keycode.raw(),
                            &mode,
                            false,
                        );
                        if matched.is_none() {
                            for sym in handle.raw_syms() {
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
                        if let Some(kb) = matched {
                            if matches!(kb.action.as_str(), "force_unlock" | "force-unlock") {
                                tracing::warn!(
                                    "lock: force_unlock keybind hit, breaking out"
                                );
                                let action = kb.action.clone();
                                let arg = kb.arg.clone();
                                crate::dispatch::dispatch_action(state, &action, &arg);
                                return FilterResult::Intercept(());
                            }
                        }
                    }
                    let focus = state
                        .seat
                        .get_keyboard()
                        .and_then(|kb| kb.current_focus());
                    tracing::info!(
                        "lock: forwarding key keycode={} state={:?} focus={}",
                        keycode.raw(),
                        key_state,
                        match &focus {
                            Some(crate::state::FocusTarget::SessionLock(_)) => "SessionLock",
                            Some(crate::state::FocusTarget::LayerSurface(_)) => "LayerSurface",
                            Some(crate::state::FocusTarget::Window(_)) => "Window",
                            None => "None",
                        }
                    );
                    return FilterResult::Forward;
                }
                // Check for compositor keybindings when key is pressed
                if key_state == KeyState::Pressed {
                    let keysym = handle.modified_sym();
                    let mods = smithay_mods_to_margo(modifiers);
                    let mode = state.input_keyboard.mode.clone();
                    debug!("key pressed: keysym={:#x} mods={:?} mode={}", keysym.raw(), mods, mode);
                    
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
                    if matched.is_none() {
                        for sym in handle.raw_syms() {
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

                    if let Some(kb) = matched {
                        info!("keybinding match: {:?} {:?}", kb.action, kb.arg);
                        let action = kb.action.clone();
                        let arg = kb.arg.clone();
                        crate::dispatch::dispatch_action(state, &action, &arg);
                        return FilterResult::Intercept(());
                    }
                }
                FilterResult::Forward
            },
        );
    }
}

fn handle_pointer_motion<B: InputBackend, E: PointerMotionEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let delta = (event.delta_x(), event.delta_y()).into();
    let delta_unaccel = (event.delta_x_unaccel(), event.delta_y_unaccel()).into();
    let time = event.time_msec();

    state.input_pointer.x += event.delta_x();
    state.input_pointer.y += event.delta_y();
    state.clamp_pointer_to_outputs();
    state.input_pointer.motion_events += 1;
    let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
    log_pointer_motion(state, "relative", pos);
    state.request_repaint();

    if state.session_locked {
        // Multi-monitor lock: keyboard focus has to follow the cursor so
        // the lock surface on the screen the user is looking at is the
        // one that gets keystrokes. Without this, after `alt+l` you might
        // be typing into eDP-1's lock surface while staring at DP-3's,
        // and nothing happens.
        state.refresh_keyboard_focus();
    }

    // Sloppy-focus uses the toplevel-level FocusTarget (keyboard cares about
    // windows, not subsurfaces). Pointer events use the drilled wl_surface.
    let kbd_focus = focus_under(state, pos);
    apply_sloppy_focus(state, kbd_focus.as_ref().map(|(t, _)| t));
    let ptr_focus = pointer_focus_under(state, pos);

    if let Some(ptr) = state.seat.get_pointer() {
        ptr.motion(
            state,
            ptr_focus.clone(),
            &MotionEvent { location: pos, serial, time },
        );
        ptr.relative_motion(
            state,
            ptr_focus,
            &RelativeMotionEvent {
                delta,
                delta_unaccel,
                utime: event.time() as u64 * 1000,
            },
        );
        ptr.frame(state);
    }
}

/// `sloppyfocus`: when the pointer crosses into a new toplevel window, give
/// it keyboard focus. In scroller layout this also re-centers the column.
/// Skipped for layer-shell and transient surfaces — only `Window` targets.
fn apply_sloppy_focus(state: &mut MargoState, target: Option<&FocusTarget>) {
    if !state.config.sloppyfocus {
        return;
    }
    let Some(FocusTarget::Window(window)) = target else {
        return;
    };
    // Already focused? skip.
    if let Some(idx) = state.focused_client_idx() {
        if state.clients[idx].window == *window {
            return;
        }
    }
    state.focus_surface(Some(FocusTarget::Window(window.clone())));
    // Re-arrange so scroller-mode auto-centers the new focus.
    let mon = state.focused_monitor();
    if mon < state.monitors.len() {
        state.arrange_monitor(mon);
    }
}

fn handle_pointer_motion_abs<B: InputBackend, E: PointerMotionAbsoluteEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let output = state.space.outputs().next().cloned();
    if let Some(output) = output {
        let size = state.space.output_geometry(&output).unwrap_or_default().size;
        let pos = event.position_transformed(size);
        state.input_pointer.x = pos.x;
        state.input_pointer.y = pos.y;
        state.clamp_pointer_to_outputs();
        state.input_pointer.motion_events += 1;
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        log_pointer_motion(state, "absolute", pos);
        state.request_repaint();
        let kbd_focus = focus_under(state, pos);
        apply_sloppy_focus(state, kbd_focus.as_ref().map(|(t, _)| t));
        let ptr_focus = pointer_focus_under(state, pos);
        if let Some(ptr) = state.seat.get_pointer() {
            ptr.motion(
                state,
                ptr_focus,
                &MotionEvent { location: pos, serial, time: event.time_msec() },
            );
            ptr.frame(state);
        }
    }
}

fn log_pointer_motion(state: &MargoState, kind: &str, pos: Point<f64, Logical>) {
    let count = state.input_pointer.motion_events;
    if count <= 10 || count % 120 == 0 {
        info!(
            "pointer motion kind={} count={} x={:.1} y={:.1}",
            kind, count, pos.x, pos.y
        );
    }
}

fn handle_pointer_button<B: InputBackend, E: PointerButtonEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let btn_state = event.state();
    let button = event.button_code();
    let pos = Point::from((state.input_pointer.x, state.input_pointer.y));

    // Mousebind dispatch — `mousebind = MOD,btn_left,moveresize,curmove`
    // and friends. Match on press only; release passes through so any
    // grab we kicked off cleans up via its own button handler. If we
    // dispatch an action we DON'T forward the button to clients
    // (otherwise super+left-drag would also be a click for them).
    if btn_state == ButtonState::Pressed {
        let mods = state
            .seat
            .get_keyboard()
            .map(|k| smithay_mods_to_margo(&k.modifier_state()))
            .unwrap_or_else(margo_config::Modifiers::empty);
        let matched = state
            .config
            .mouse_bindings
            .iter()
            .find(|mb| mb.modifiers == mods && mb.button == button)
            .cloned();
        if let Some(mb) = matched {
            // Make sure focus follows the click first so move/resize
            // operates on the *clicked* window, not whatever happened to
            // be focused.
            if let Some((target, _)) = focus_under(state, pos) {
                state.focus_surface(Some(target));
            }
            crate::dispatch::dispatch_action(state, &mb.action, &mb.arg);
            state.request_repaint();
            return;
        }
    }

    if btn_state == ButtonState::Pressed {
        if state.is_overview_open() {
            match focus_under(state, pos).map(|(target, _)| target) {
                Some(FocusTarget::Window(window)) => {
                    state.close_overview(Some(window));
                }
                Some(target @ FocusTarget::LayerSurface(_)) => {
                    state.focus_surface(Some(target));
                }
                Some(target @ FocusTarget::SessionLock(_)) => {
                    state.focus_surface(Some(target));
                }
                None => state.close_overview(None),
            }
        } else if let Some((target, _)) = focus_under(state, pos) {
            state.focus_surface(Some(target));
        } else {
            state.focus_surface(None);
        }
    }
    state.request_repaint();
    if let Some(ptr) = state.seat.get_pointer() {
        ptr.button(
            state,
            &ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: btn_state,
            },
        );
        ptr.frame(state);
    }
}

fn handle_pointer_axis<B: InputBackend, E: PointerAxisEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    // AxisFrame::source() and AxisFrame::value() both use smithay::backend::input types.
    let mut frame = AxisFrame::new(event.time_msec()).source(event.source());

    if event.amount_v120(Axis::Horizontal).is_some() || event.amount(Axis::Horizontal).is_some() {
        let amount = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) / 120.0 * 3.0);
        frame = frame.value(Axis::Horizontal, amount);
        if let Some(v120) = event.amount_v120(Axis::Horizontal) {
            frame = frame.v120(Axis::Horizontal, v120 as i32);
        }
    }
    if event.amount_v120(Axis::Vertical).is_some() || event.amount(Axis::Vertical).is_some() {
        let amount = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) / 120.0 * 3.0);
        frame = frame.value(Axis::Vertical, amount);
        if let Some(v120) = event.amount_v120(Axis::Vertical) {
            frame = frame.v120(Axis::Vertical, v120 as i32);
        }
    }

    if let Some(ptr) = state.seat.get_pointer() {
        ptr.axis(state, frame);
        ptr.frame(state);
    }
    state.request_repaint();
}

fn smithay_mods_to_margo(m: &ModifiersState) -> margo_config::Modifiers {
    let mut out = margo_config::Modifiers::empty();
    if m.shift { out |= margo_config::Modifiers::SHIFT; }
    if m.ctrl  { out |= margo_config::Modifiers::CTRL; }
    if m.alt   { out |= margo_config::Modifiers::ALT; }
    if m.logo  { out |= margo_config::Modifiers::LOGO; }
    if m.caps_lock { out |= margo_config::Modifiers::CAPS; }
    out
}

fn exclusive_keyboard_layer(state: &MargoState) -> Option<FocusTarget> {
    if state.session_locked {
        // Find the lock surface on the currently focused monitor/output.
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        let output = state.space.output_under(pos).next()
            .or_else(|| state.monitors.first().map(|m| &m.output));

        if let Some(output) = output {
            if let Some((_, surface)) = state.lock_surfaces.iter().find(|(o, _)| o == output) {
                return Some(FocusTarget::SessionLock(surface.clone()));
            }
        }
        // Fallback to the first available lock surface.
        return state.lock_surfaces.first().map(|(_, s)| FocusTarget::SessionLock(s.clone()));
    }

    for layer in state.layer_shell_state.layer_surfaces().rev() {
        let exclusive = layer.with_cached_state(|data| {
            data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && matches!(data.layer, WlrLayer::Top | WlrLayer::Overlay)
        });

        if !exclusive {
            continue;
        }

        let mapped = state.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let found = map
                .layers()
                .find(|mapped| mapped.layer_surface() == &layer)
                .map(|mapped| mapped.layer_surface().clone());
            found
        });

        if let Some(surface) = mapped {
            return Some(FocusTarget::LayerSurface(surface));
        }
    }

    None
}

fn focus_under(
    state: &MargoState,
    pos: Point<f64, Logical>,
) -> Option<(FocusTarget, Point<f64, Logical>)> {
    if state.session_locked {
        return state.space.output_under(pos).next().and_then(|output| {
            state.lock_surfaces.iter().find(|(o, _)| o == output).map(|(_, s)| {
                let output_geo = state.space.output_geometry(output).unwrap();
                let local = pos - output_geo.loc.to_f64();
                (FocusTarget::SessionLock(s.clone()), local)
            })
        });
    }

    layer_focus_under(state, pos, &[WlrLayer::Overlay, WlrLayer::Top])
        .or_else(|| {
            state
                .space
                .element_under(pos)
                .map(|(w, p)| (FocusTarget::Window(w.clone()), p.to_f64()))
        })
        .or_else(|| layer_focus_under(state, pos, &[WlrLayer::Bottom, WlrLayer::Background]))
}

/// Pointer-specific focus lookup: returns the actual `WlSurface` under the
/// pointer (drilled through subsurfaces and popups), with that surface's
/// origin in space. This is what `pointer.motion` should receive — when a
/// CSD GTK file dialog with multiple subsurfaces is on screen, we route to
/// the right child surface instead of always the toplevel.
fn pointer_focus_under(
    state: &MargoState,
    pos: Point<f64, Logical>,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
)> {
    use smithay::desktop::WindowSurfaceType;

    // While the session is locked, EVERY pointer event must go to the
    // lock surface for the output the cursor is on. Anything else would
    // route input to hidden background apps and defeat the lock.
    if state.session_locked {
        let output = state.space.output_under(pos).next()?;
        let (_, lock_surface) = state
            .lock_surfaces
            .iter()
            .find(|(o, _)| o == output)?;
        let output_geo = state.space.output_geometry(output)?;
        return Some((
            lock_surface.wl_surface().clone(),
            output_geo.loc.to_f64(),
        ));
    }

    // Layer surfaces sit above + below the windows.
    let layer_above = layer_pointer_under(state, pos, &[WlrLayer::Overlay, WlrLayer::Top]);
    if layer_above.is_some() {
        return layer_above;
    }

    if let Some((window, win_loc)) = state.space.element_under(pos) {
        let local = pos - win_loc.to_f64();
        if let Some((surface, surf_loc)) = window.surface_under(local, WindowSurfaceType::ALL) {
            // surf_loc is in window-local coords; add window origin to make
            // it space-coords (matches what smithay's pointer.motion expects).
            let space_loc = (surf_loc + win_loc).to_f64();
            return Some((surface, space_loc));
        }
    }

    layer_pointer_under(state, pos, &[WlrLayer::Bottom, WlrLayer::Background])
}

fn layer_pointer_under(
    state: &MargoState,
    pos: Point<f64, Logical>,
    layer_kinds: &[WlrLayer],
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
)> {
    use smithay::desktop::WindowSurfaceType;
    let output = state.space.output_under(pos).next()?;
    let output_geo = state.space.output_geometry(output)?;
    let output_pos = pos - output_geo.loc.to_f64();
    let layers = layer_map_for_output(output);

    for layer_kind in layer_kinds {
        for layer in layers.layers_on(*layer_kind).rev() {
            let Some(layer_geo) = layers.layer_geometry(layer) else {
                continue;
            };
            let layer_local = output_pos - layer_geo.loc.to_f64();
            if let Some((surface, surf_loc)) =
                layer.surface_under(layer_local, WindowSurfaceType::ALL)
            {
                let space_loc =
                    (surf_loc + layer_geo.loc + output_geo.loc).to_f64();
                return Some((surface, space_loc));
            }
        }
    }

    None
}

fn layer_focus_under(
    state: &MargoState,
    pos: Point<f64, Logical>,
    layer_kinds: &[WlrLayer],
) -> Option<(FocusTarget, Point<f64, Logical>)> {
    let output = state.space.output_under(pos).next()?;
    let output_geo = state.space.output_geometry(output)?;
    let output_pos = pos - output_geo.loc.to_f64();
    let layers = layer_map_for_output(output);

    for layer_kind in layer_kinds {
        for layer in layers.layers_on(*layer_kind).rev() {
            let Some(layer_geo) = layers.layer_geometry(layer) else {
                continue;
            };
            let surface_pos = output_pos - layer_geo.loc.to_f64();

            if layer
                .surface_under(surface_pos, WindowSurfaceType::ALL)
                .is_some()
            {
                let focus_loc = (output_geo.loc + layer_geo.loc).to_f64();
                return Some((
                    FocusTarget::LayerSurface(layer.layer_surface().clone()),
                    focus_loc,
                ));
            }
        }
    }

    None
}
