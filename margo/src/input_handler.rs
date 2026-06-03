#![allow(dead_code)]
//! Translates raw libinput / winit events into compositor actions.

use smithay::{
    backend::input::{
        Axis, ButtonState, GestureBeginEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionAbsoluteEvent, PointerMotionEvent,
    },
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::{FilterResult, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer},
};
use tracing::{debug, info};

use crate::{
    input::{TouchPoint, TouchRelease, find_keybinding},
    state::{FocusTarget, MargoState},
};

pub fn handle_input<B: InputBackend>(state: &mut MargoState, event: InputEvent<B>) {
    let _span = tracy_client::span!("handle_input");
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
        InputEvent::TouchDown { event } => handle_touch_down(state, event),
        InputEvent::TouchMotion { event } => handle_touch_motion(state, event),
        InputEvent::TouchUp { event } => handle_touch_up(state, event),
        _ => {}
    }
}

// ── Touchpad swipe gestures ──────────────────────────────────────────────────
//
// We accumulate dx/dy across the gesture, then on End we resolve a cardinal
// direction (or diagonal) and look up `gesturebind` from the config. This
// mirrors how mango (C) and niri handle touchpad swipes.

/// Hardcoded fallback when `Config::swipe_min_threshold` isn't set (or is
/// set to its default of 1, which is too aggressive for natural use).
/// Mango's C version uses ~30 px and that feels right; the config knob
/// gives users with very-short-throw swipes a way to lower it.
const SWIPE_MIN_DISTANCE_DEFAULT: f64 = 30.0;

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
    dispatch_swipe(state, g.dx, g.dy, g.fingers, "touchpad");
}

/// Map a 2D displacement into the 0..=7 motion code the gesture
/// binding table uses (matches `margo-config::parse_motion()`):
/// UP=0, DOWN=1, RIGHT=2, LEFT=3, UP_RIGHT=4, UP_LEFT=5, DOWN_LEFT=6,
/// DOWN_RIGHT=7. Diagonal threshold: each axis must contribute > 40 %
/// of the total magnitude.
fn derive_motion_code(dx: f64, dy: f64) -> u32 {
    let total = (dx * dx + dy * dy).sqrt();
    let ax = dx.abs();
    let ay = dy.abs();
    let diag = ax > 0.4 * total && ay > 0.4 * total;
    if diag {
        match (dx.is_sign_positive(), dy.is_sign_positive()) {
            (true, false) => 4,  // UP_RIGHT
            (false, false) => 5, // UP_LEFT
            (false, true) => 6,  // DOWN_LEFT
            (true, true) => 7,   // DOWN_RIGHT
        }
    } else if ax > ay {
        if dx > 0.0 { 2 } else { 3 } // RIGHT / LEFT
    } else if dy < 0.0 {
        0 // UP
    } else {
        1 // DOWN
    }
}

/// Common dispatch path used by **both** the touchpad swipe gesture
/// (libinput `GestureSwipeEnd`) and the touchscreen multi-finger
/// swipe (`InputEvent::TouchUp` after ≥ 2 fingers had touched). Looks
/// up `(fingers, motion, mods)` in `Config::gesture_bindings` and
/// fires the matched action.
///
/// `source` is just a log tag so the trace clearly says which input
/// path matched — handy when binding-debugging on a 2-in-1 with both
/// a touchpad and a touchscreen.
fn dispatch_swipe(state: &mut MargoState, dx: f64, dy: f64, fingers: u32, source: &'static str) {
    let total = (dx * dx + dy * dy).sqrt();
    let cfg_threshold = state.config.swipe_min_threshold as f64;
    let threshold = if cfg_threshold > 1.0 {
        cfg_threshold
    } else {
        SWIPE_MIN_DISTANCE_DEFAULT
    };
    if total < threshold {
        debug!(
            "{source} swipe ignored (too short): total={:.1} threshold={:.1} fingers={}",
            total, threshold, fingers
        );
        return;
    }
    let motion = derive_motion_code(dx, dy);

    let mods = state
        .seat
        .get_keyboard()
        .map(|k| smithay_mods_to_margo(&k.modifier_state()))
        .unwrap_or_else(margo_config::Modifiers::empty);

    let binding = state
        .config
        .gesture_bindings
        .iter()
        .find(|b| b.fingers == fingers && b.motion == motion && b.modifiers == mods);

    if let Some(binding) = binding {
        info!(
            source = source,
            fingers = fingers,
            motion = motion,
            mods = ?mods,
            action = ?binding.action,
            "swipe match",
        );
        let action = binding.action.clone();
        let arg = binding.arg.clone();
        crate::dispatch::dispatch_action(state, &action, &arg);
        state.request_repaint();
    } else {
        debug!(
            "{source} swipe unmatched: fingers={} motion={} mods={:?}",
            fingers, motion, mods
        );
    }
}

// ── Touchscreen handling ────────────────────────────────────────────────────
//
// Direct touch events (true touchscreen / 2-in-1 panel; not touchpad
// gestures, which arrive as `InputEvent::GestureSwipe*` and have
// already been distilled by libinput). Multi-finger swipe gets routed
// to the same `gesture_bindings` table that touchpad swipe uses, so a
// binding written as `gesture = swipe, 3, right, view_tag` fires for
// either input path.

fn handle_touch_down<B: InputBackend, E: smithay::backend::input::TouchDownEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let id: i32 = event.slot().into();
    // libinput delivers absolute coords as `[0, 1]` normalised; we
    // only need them as a magnitude, so the unit doesn't matter as
    // long as DOWN/MOTION/UP agree.
    let pos = event.position_transformed(smithay::utils::Size::from((1, 1)));
    let (x, y) = (pos.x, pos.y);
    let now = event.time_msec();
    state.input_touch.points.push(TouchPoint {
        id,
        x,
        y,
        start_x: x,
        start_y: y,
        start_time: now,
    });
    if state.input_touch.points.len() >= 2 {
        state.input_touch.gesture_armed = true;
    }
}

fn handle_touch_motion<B: InputBackend, E: smithay::backend::input::TouchMotionEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let id: i32 = event.slot().into();
    let pos = event.position_transformed(smithay::utils::Size::from((1, 1)));
    if let Some(p) = state.input_touch.points.iter_mut().find(|p| p.id == id) {
        p.x = pos.x;
        p.y = pos.y;
    }
}

fn handle_touch_up<B: InputBackend, E: smithay::backend::input::TouchUpEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let id: i32 = event.slot().into();
    let removed = state
        .input_touch
        .points
        .iter()
        .position(|p| p.id == id)
        .map(|i| state.input_touch.points.remove(i));

    if state.input_touch.gesture_armed {
        if let Some(p) = removed {
            state.input_touch.releases.push(TouchRelease {
                start_x: p.start_x,
                start_y: p.start_y,
                end_x: p.x,
                end_y: p.y,
            });
        }
    }

    // Gesture completes when every finger is up.
    if !state.input_touch.points.is_empty() {
        return;
    }
    if !state.input_touch.gesture_armed {
        return;
    }
    let releases = std::mem::take(&mut state.input_touch.releases);
    state.input_touch.gesture_armed = false;
    if releases.len() < 2 {
        // Not a multi-finger gesture; could be a tap that briefly
        // overlapped a pre-existing touch. Drop without dispatching.
        return;
    }

    // Average displacement across every finger that contributed —
    // smooths out the natural finger-to-finger variation in a
    // hand-driven swipe.
    let n = releases.len() as f64;
    let avg_dx: f64 = releases.iter().map(|r| r.end_x - r.start_x).sum::<f64>() / n;
    let avg_dy: f64 = releases.iter().map(|r| r.end_y - r.start_y).sum::<f64>() / n;
    dispatch_swipe(state, avg_dx, avg_dy, releases.len() as u32, "touchscreen");
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

        keyboard.input(
            state,
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
                                tracing::warn!("lock: force_unlock keybind hit, breaking out");
                                let action = kb.action.clone();
                                let arg = kb.arg.clone();
                                crate::dispatch::dispatch_action(state, &action, &arg);
                                return FilterResult::Intercept(());
                            }
                        }
                    }
                    let focus = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
                    tracing::info!(
                        "lock: forwarding key keycode={} state={:?} focus={}",
                        keycode.raw(),
                        key_state,
                        match &focus {
                            Some(crate::state::FocusTarget::SessionLock(_)) => "SessionLock",
                            Some(crate::state::FocusTarget::LayerSurface(_)) => "LayerSurface",
                            Some(crate::state::FocusTarget::Window(_)) => "Window",
                            Some(crate::state::FocusTarget::Popup(_)) => "Popup",
                            None => "None",
                        }
                    );
                    return FilterResult::Forward;
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
                    debug!(
                        "key pressed: keysym={:#x} mods={:?} mode={}",
                        keysym.raw(),
                        mods,
                        mode
                    );

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
                        crate::dispatch::dispatch_action(state, &action, &arg);
                        return FilterResult::Intercept(());
                    }

                    // Scroller-overview modal keys. Runs only when no
                    // keybind claimed the press, so user binds and the
                    // toggle key still win: Esc closes, Up/Down move the
                    // selection, Enter activates the selected tag.
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
                            return FilterResult::Intercept(());
                        }
                    }
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

                FilterResult::Forward
            },
        );

        // The key event may have toggled the xkb layout group (e.g.
        // a `grp:` toggle). Re-cache the active layout name so the
        // shell's keyboard-layout pill reflects the switch.
        state.refresh_keyboard_layout();
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

    // Save the pre-move cursor position so we can restore it for
    // pointer-constraints-v1 lock requests (FPS games etc.).
    let prev_x = state.input_pointer.x;
    let prev_y = state.input_pointer.y;

    state.input_pointer.x += event.delta_x();
    state.input_pointer.y += event.delta_y();
    state.clamp_pointer_to_outputs();
    state.input_pointer.motion_events += 1;
    state.refresh_pointer_monitor_tracking();
    // Edge-scroller pointer focus (no-op unless
    // `edge_scroller_focus_allow_speed > 0`). Speed = this event's
    // accelerated motion magnitude.
    let edge_speed = (event.delta_x().powi(2) + event.delta_y().powi(2)).sqrt();
    state.maybe_edge_scroller_focus(edge_speed);
    state.request_repaint();

    // Pointer-constraints enforcement. Two cases:
    //   * Active LOCK: the cursor stays pinned at its prior absolute
    //     position; only relative deltas reach the client. We undo
    //     the position update we just applied and restore prev_*.
    //   * Active CONFINE: the cursor is allowed to move, but only
    //     inside the constraint region. Smithay clamps internally,
    //     but it doesn't update *our* shadow `input_pointer.x/y`
    //     since the source of truth lives there. Re-clamp ourselves
    //     against the constraint's region so subsequent libinput
    //     deltas accumulate from the clamped value.
    if let Some(pointer) = state.seat.get_pointer() {
        if let Some(focus_surface) = pointer
            .current_focus()
            .as_ref()
            .and_then(|f| f.wl_surface())
        {
            use smithay::wayland::pointer_constraints::{
                PointerConstraint, with_pointer_constraint,
            };
            with_pointer_constraint(&focus_surface, &pointer, |constraint| {
                if let Some(constraint) = constraint {
                    if constraint.is_active() {
                        if let PointerConstraint::Locked(_) = &*constraint {
                            state.input_pointer.x = prev_x;
                            state.input_pointer.y = prev_y;
                        }
                        // Confined constraint: smithay clamps to
                        // region inside its `pointer.motion()`
                        // dispatch below; we don't have to do
                        // anything extra here.
                    }
                }
            });
        }
    }

    let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
    log_pointer_motion(state, "relative", pos);

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
    update_overview_hover(state, pos);
    let ptr_focus = pointer_focus_under(state, pos);

    if let Some(ptr) = state.seat.get_pointer() {
        ptr.motion(
            state,
            ptr_focus.clone(),
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        ptr.relative_motion(
            state,
            ptr_focus,
            &RelativeMotionEvent {
                delta,
                delta_unaccel,
                utime: event.time() * 1000,
            },
        );
        ptr.frame(state);
    }

    // Hot corner check — niri pattern. The pointer is in a corner
    // when it's in a 1×1 logical-pixel rectangle at one of the four
    // output corners; entering the corner arms a dwell timer, dwelling
    // past `Config::hot_corner_dwell_ms` fires the configured dispatch
    // action. Cleared on every motion that lands outside any corner so
    // a quick out-and-back-in restarts the timer (matches niri).
    update_hot_corner(state);
}

fn update_hot_corner(state: &mut MargoState) {
    use crate::state::HotCorner;

    // Hard guards — these states own the screen and a corner-trigger
    // dispatch on top of them produced the symptom the user hit:
    //   * `session_locked` → dispatching `toggle_overview` while the
    //     lock surface owns focus pushed the cursor through to the
    //     login prompt because the lock-surface's keyboard grab kept
    //     translating Tab/Return into the GreetD authentication flow.
    //   * any pointer or keyboard grab held by a popup → corner
    //     trigger would smash through the grab and the popup
    //     would dismiss without surfacing an action.
    // Bail before we even check the corners; armed_at stays None so
    // a re-entry restarts the timer cleanly once the guard lifts.
    if state.session_locked {
        return;
    }
    if state
        .seat
        .get_pointer()
        .map(|p| p.is_grabbed())
        .unwrap_or(false)
        || state
            .seat
            .get_keyboard()
            .map(|k| k.is_grabbed())
            .unwrap_or(false)
    {
        return;
    }

    // Resolve the focused output's geometry — niri's hot-corner check
    // is per-output. Margo's outputs are arranged side-by-side in
    // `state.space`, so we hit-test against each output_geometry().
    let cursor_x = state.input_pointer.x;
    let cursor_y = state.input_pointer.y;

    // Find which output the cursor is currently inside (if any). For
    // each output we then check the four corners of its logical
    // rect against the cursor at 1 px tolerance.
    let mut current_corner: Option<HotCorner> = None;
    for output in state.space.outputs() {
        let Some(geo) = state.space.output_geometry(output) else {
            continue;
        };
        let x0 = geo.loc.x as f64;
        let y0 = geo.loc.y as f64;
        let x1 = (geo.loc.x + geo.size.w) as f64;
        let y1 = (geo.loc.y + geo.size.h) as f64;
        // 1 px tolerance — matches niri's `Rectangle::new(corner, Size::new(1, 1))`.
        let near_left = (cursor_x - x0).abs() < 1.0;
        let near_right = (cursor_x - (x1 - 1.0)).abs() < 1.0;
        let near_top = (cursor_y - y0).abs() < 1.0;
        let near_bottom = (cursor_y - (y1 - 1.0)).abs() < 1.0;
        current_corner = match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => Some(HotCorner::TopLeft),
            (_, true, true, _) => Some(HotCorner::TopRight),
            (true, _, _, true) => Some(HotCorner::BottomLeft),
            (_, true, _, true) => Some(HotCorner::BottomRight),
            _ => None,
        };
        if current_corner.is_some() {
            break;
        }
    }

    // Track entry / exit. On entry, arm the dwell timer; on exit,
    // disarm. While dwelling in the same corner across multiple motion
    // events, the `armed_at` instant is preserved.
    if state.hot_corner_dwelling != current_corner {
        state.hot_corner_dwelling = current_corner;
        state.hot_corner_armed_at = current_corner.map(|_| std::time::Instant::now());
        return;
    }

    // Same corner as last motion — check if dwell threshold elapsed.
    let Some(corner) = current_corner else { return };
    let Some(armed) = state.hot_corner_armed_at else {
        return;
    };
    let dwell = std::time::Duration::from_millis(state.config.hot_corner_dwell_ms as u64);
    if armed.elapsed() < dwell {
        return;
    }

    // Threshold reached. Fire the action and clear `armed_at` so we
    // don't re-fire on every subsequent motion while still in the
    // corner — user has to leave and re-enter to trigger again.
    let action = corner.action_str(&state.config).trim().to_string();
    state.hot_corner_armed_at = None;
    if action.is_empty() {
        return;
    }
    tracing::info!(
        target: "hot_corner",
        corner = ?corner,
        action = %action,
        "fired",
    );
    let arg = margo_config::Arg::default();
    crate::dispatch::dispatch_action(state, &action, &arg);
}

/// `sloppyfocus`: when the pointer crosses into a new toplevel window, give
/// it keyboard focus. In scroller layout this also re-centers the column.
/// Skipped for layer-shell and transient surfaces — only `Window` targets.
fn apply_sloppy_focus(state: &mut MargoState, target: Option<&FocusTarget>) {
    if !state.config.sloppyfocus {
        return;
    }
    // Overview is open: pointer hover MUST NOT trigger an actual
    // focus change. `is_overview_hovered` + `border::refresh` already
    // paint the selected thumbnail's focuscolor border, which is the
    // visual feedback the user expects. Letting sloppy focus fire
    // here would push the hovered window onto `focus_history`, and
    // the next `arrange_monitor` (mouse motion already requests one
    // via `request_repaint`) would recompute the tiled vec in MRU
    // order and visibly re-shuffle the grid mid-hover — the user's
    // "touchpad ile gezerken sıralama değişiyor" symptom. Commit on
    // overview close (`overview_activate` → `close_overview` →
    // `focus_surface`) is the only place focus_history should mutate
    // during an overview session.
    if state.is_overview_open() {
        return;
    }
    // While a popup grab is up, motion over an underlying toplevel
    // must not refocus it: PopupKeyboardGrab will drop our
    // `keyboard.set_focus()` anyway, but the surrounding side
    // effects (selected, IPC watch push, scripting hooks,
    // border crossfade) still run and shake the popup loose.
    if state
        .seat
        .get_keyboard()
        .map(|k| k.is_grabbed())
        .unwrap_or(false)
        || state
            .seat
            .get_pointer()
            .map(|p| p.is_grabbed())
            .unwrap_or(false)
    {
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
    //
    // Off by default. With `scroller_focus_center = 1` (a common
    // setting) every cursor crossing into another column kicks the
    // scroller to recenter on it, restarts a 480 ms slide animation,
    // and the user perceives the constant re-centering as window
    // jitter — that's the original "border ve pencere kayması"
    // report. We keep the call available behind `sloppyfocus_arrange`
    // for users who explicitly want the scroller to follow the mouse.
    if state.config.sloppyfocus_arrange {
        let mon = state.focused_monitor();
        if mon < state.monitors.len() {
            state.arrange_monitor(mon);
        }
    }
}

fn handle_pointer_motion_abs<B: InputBackend, E: PointerMotionAbsoluteEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let output = state.space.outputs().next().cloned();
    if let Some(output) = output {
        let size = state
            .space
            .output_geometry(&output)
            .unwrap_or_default()
            .size;
        let pos = event.position_transformed(size);
        state.input_pointer.x = pos.x;
        state.input_pointer.y = pos.y;
        state.clamp_pointer_to_outputs();
        state.input_pointer.motion_events += 1;
        state.refresh_pointer_monitor_tracking();
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        log_pointer_motion(state, "absolute", pos);
        state.request_repaint();

        let kbd_focus = focus_under(state, pos);
        apply_sloppy_focus(state, kbd_focus.as_ref().map(|(t, _)| t));
        update_overview_hover(state, pos);
        let ptr_focus = pointer_focus_under(state, pos);
        if let Some(ptr) = state.seat.get_pointer() {
            ptr.motion(
                state,
                ptr_focus,
                &MotionEvent {
                    location: pos,
                    serial,
                    time: event.time_msec(),
                },
            );
            ptr.frame(state);
        }
    }
}

/// Mark the overview thumbnail under the cursor as hovered so the
/// border layer paints it with `focuscolor`. No-op outside overview.
/// Walks the client list once — overview always has a small N (only
/// tiled, non-minimized, non-scratchpad clients), so an O(n) scan
/// per motion event is fine. Skips when the geom rect is empty
/// (deferred-map clients land here briefly).
fn update_overview_hover(state: &mut MargoState, pos: Point<f64, Logical>) {
    if !state.is_overview_open() {
        return;
    }
    let mut new_hovered: Option<usize> = None;
    let px = pos.x as i32;
    let py = pos.y as i32;
    for (i, c) in state.clients.iter().enumerate() {
        let g = &c.geom;
        if g.width <= 0 || g.height <= 0 {
            continue;
        }
        if px >= g.x && px < g.x + g.width && py >= g.y && py < g.y + g.height {
            new_hovered = Some(i);
            break;
        }
    }
    let mut changed = false;
    for (i, c) in state.clients.iter_mut().enumerate() {
        let want = new_hovered == Some(i);
        if c.is_overview_hovered != want {
            c.is_overview_hovered = want;
            changed = true;
        }
    }
    if changed {
        crate::border::refresh(state);
        state.request_repaint();
    }
}

fn log_pointer_motion(state: &MargoState, kind: &str, pos: Point<f64, Logical>) {
    let count = state.input_pointer.motion_events;
    if count <= 10 || count.is_multiple_of(120) {
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

    // Scroller overview: a left-press activates the tag cell / window
    // under the cursor (switch + focus), a backdrop click closes. Every
    // button event is consumed so none reaches the scaled-down clients.
    if state.is_scroller_overview_open() {
        const BTN_LEFT: u32 = 0x110;
        if btn_state == ButtonState::Pressed && button == BTN_LEFT {
            state.scroller_overview_click(pos.x, pos.y);
        }
        return;
    }

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

    // Skip our own focus-on-click logic while smithay holds an
    // active pointer or keyboard grab. The interesting case is an
    // xdg_popup grab (right-click menu, GTK/Chromium chevron menus):
    // PopupPointerGrab routes the click to the popup or dismisses
    // it on outside-click. Our `focus_under(pos)` only knows about
    // toplevels/layers — it can't see the popup — so it would
    // return whatever window geometrically sits beneath the popup
    // and `focus_surface(...)` would yank `selected` over to the
    // wrong toplevel. Symptoms: GTK/Chromium menus visibly opening
    // for one frame and closing again, right-click never producing
    // a stable menu, double-clicks being interpreted as window
    // focus swaps. Let the grab own the click; if it dismisses,
    // smithay also drops keyboard/pointer focus and our normal
    // motion handling re-establishes focus on the next event.
    let pointer_grabbed = state
        .seat
        .get_pointer()
        .map(|p| p.is_grabbed())
        .unwrap_or(false);
    let keyboard_grabbed = state
        .seat
        .get_keyboard()
        .map(|k| k.is_grabbed())
        .unwrap_or(false);
    let in_grab = pointer_grabbed || keyboard_grabbed;

    if btn_state == ButtonState::Pressed && !in_grab {
        if state.is_overview_open() {
            // Phase 3 spatial-mode press routing. Three outcomes:
            //   * Click on a window thumbnail → close overview onto it
            //     (legacy click-to-activate, unchanged).
            //   * Click on layer / session-lock / popup surface → focus
            //     it (unchanged).
            //   * Click on empty space + LMB + spatial mode → start
            //     panning the world camera. Grid mode falls through
            //     to close_overview(None) like before.
            match focus_under(state, pos).map(|(target, _)| target) {
                Some(FocusTarget::Window(window)) => {
                    state.close_overview(Some(window));
                }
                Some(target @ FocusTarget::LayerSurface(_))
                | Some(target @ FocusTarget::SessionLock(_))
                | Some(target @ FocusTarget::Popup(_)) => {
                    state.focus_surface(Some(target));
                }
                None => state.close_overview(None),
            }
        } else {
            match focus_under(state, pos) {
                Some((target, _)) => {
                    state.focus_surface(Some(target));
                }
                _ => {
                    state.focus_surface(None);
                }
            }
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

fn handle_pointer_axis<B: InputBackend, E: PointerAxisEvent<B>>(state: &mut MargoState, event: E) {
    // Scroller overview: scroll continuously pans the tag strip on the
    // monitor under the pointer (niri-style), with momentum + snap handled
    // in the physics tick. Convert the device delta to cell units: a wheel
    // notch (v120 = 120) moves one cell; touchpad finger scroll (no v120)
    // pans smoothly via a small gain. Consume the event so the scaled-down
    // clients never receive it.
    if state.is_scroller_overview_open() {
        let v120 = event
            .amount_v120(Axis::Vertical)
            .filter(|v| *v != 0.0)
            .or_else(|| event.amount_v120(Axis::Horizontal).filter(|v| *v != 0.0));
        let discrete = v120.is_some();
        let delta_cells = if let Some(v120) = v120 {
            v120 / 120.0
        } else {
            let amt = event
                .amount(Axis::Vertical)
                .filter(|a| *a != 0.0)
                .or_else(|| event.amount(Axis::Horizontal).filter(|a| *a != 0.0))
                .unwrap_or(0.0);
            amt * 0.006
        };
        if delta_cells != 0.0 {
            // Target the monitor the pointer is over so two displays pan
            // independently; fall back to the focused monitor.
            let (px, py) = (state.input_pointer.x, state.input_pointer.y);
            let mon = state
                .monitors
                .iter()
                .position(|m| {
                    let a = m.monitor_area;
                    (px as i32) >= a.x
                        && (px as i32) < a.x + a.width
                        && (py as i32) >= a.y
                        && (py as i32) < a.y + a.height
                })
                .unwrap_or_else(|| state.focused_monitor());
            state.scroller_overview_scroll(mon, delta_cells, discrete, crate::utils::now_ms());
        }
        return;
    }

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

fn smithay_mods_to_margo(m: &ModifiersState) -> margo_config::Modifiers {
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
            if let Some((_, surface)) = state.lock_surfaces.iter().find(|(o, _)| o == output) {
                return Some(FocusTarget::SessionLock(surface.clone()));
            }
        }
        // Fallback to the first available lock surface.
        return state
            .lock_surfaces
            .first()
            .map(|(_, s)| FocusTarget::SessionLock(s.clone()));
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
            map.layers()
                .find(|mapped| mapped.layer_surface() == &layer)
                .map(|mapped| mapped.layer_surface().clone())
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
            state
                .lock_surfaces
                .iter()
                .find(|(o, _)| o == output)
                .map(|(_, s)| {
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
        let (_, lock_surface) = state.lock_surfaces.iter().find(|(o, _)| o == output)?;
        let output_geo = state.space.output_geometry(output)?;
        return Some((lock_surface.wl_surface().clone(), output_geo.loc.to_f64()));
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
                let space_loc = (surf_loc + layer_geo.loc + output_geo.loc).to_f64();
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

#[cfg(test)]
mod gesture_tests {
    use super::derive_motion_code;

    #[test]
    fn cardinal_directions_map_to_codes_0_through_3() {
        assert_eq!(derive_motion_code(0.0, -100.0), 0); // UP
        assert_eq!(derive_motion_code(0.0, 100.0), 1); // DOWN
        assert_eq!(derive_motion_code(100.0, 0.0), 2); // RIGHT
        assert_eq!(derive_motion_code(-100.0, 0.0), 3); // LEFT
    }

    #[test]
    fn balanced_diagonals_map_to_codes_4_through_7() {
        assert_eq!(derive_motion_code(70.0, -70.0), 4); // UP_RIGHT
        assert_eq!(derive_motion_code(-70.0, -70.0), 5); // UP_LEFT
        assert_eq!(derive_motion_code(-70.0, 70.0), 6); // DOWN_LEFT
        assert_eq!(derive_motion_code(70.0, 70.0), 7); // DOWN_RIGHT
    }

    #[test]
    fn small_off_axis_component_is_not_diagonal() {
        // 80 px right, 20 px down — secondary axis at 25 % of magnitude,
        // below the 40 % threshold. Should resolve to a pure direction.
        assert_eq!(derive_motion_code(80.0, 20.0), 2); // RIGHT, not DOWN_RIGHT
        assert_eq!(derive_motion_code(20.0, -80.0), 0); // UP, not UP_RIGHT
    }

    #[test]
    fn zero_displacement_falls_through_to_down() {
        // Defensive: an exact-zero swipe shouldn't crash. The current
        // code path lands on DOWN (1) — we just lock in that the
        // function is total over (0, 0).
        assert_eq!(derive_motion_code(0.0, 0.0), 1);
    }
}
