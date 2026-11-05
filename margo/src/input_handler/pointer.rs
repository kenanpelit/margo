//! Pointer input: relative/absolute motion, pointer constraints, sloppy
//! focus, overview hover, buttons, axis scrolling and axis binds.

use smithay::{
    backend::input::{
        Axis, AxisSource, ButtonState, InputBackend, PointerAxisEvent, PointerButtonEvent,
        PointerMotionAbsoluteEvent, PointerMotionEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer},
};
use tracing::info;

use super::hit::{focus_under, hit_test, pointer_focus_under};
use super::hot_corner::update_hot_corner;
use super::keyboard::smithay_mods_to_margo;
use crate::state::{FocusTarget, MargoState};

/// The client `windowrule = confine_pointer:1` currently applies to, if
/// any: the keyboard-focused client (falling back to the focused
/// monitor's selected client), provided it actually has the rule set and
/// is visible right now (not minimized, not hidden on another tag, not
/// behind the overview). `None` means nothing should be confined.
fn confine_pointer_rule_client(state: &MargoState) -> Option<usize> {
    let idx = state
        .focused_client_idx()
        .or_else(|| state.monitors.get(state.focused_monitor())?.selected)?;
    let client = state.clients.get(idx)?;
    if !client.confine_pointer || client.is_minimized {
        return None;
    }
    let mon = state.monitors.get(client.monitor)?;
    if mon.is_overview || !client.is_visible_on(client.monitor, mon.current_tagset()) {
        return None;
    }
    Some(idx)
}

/// Apply pointer constraints to the just-moved `input_pointer`, given
/// its position before the move.
fn constrain_pointer(state: &mut MargoState, prev_x: f64, prev_y: f64) {
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
    let mut has_active_pointer_constraint = false;
    if !state.session_locked
        && let Some(pointer) = state.seat.get_pointer()
    {
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
                        has_active_pointer_constraint = true;
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

    // `windowrule = confine_pointer:1` — mango port (`045c7003`). A
    // same-shaped manual clamp for windows that don't use the pointer-
    // constraints protocol at all (X11/Proton games whose own confine
    // Xwayland emulates, and which Xwayland itself can drop — see the
    // rule's doc comment). Yields entirely to a real protocol-level
    // constraint when one is active, exactly like mango does.
    if !state.session_locked && !has_active_pointer_constraint {
        if let Some(idx) = confine_pointer_rule_client(state) {
            let (min_x, min_y, max_x, max_y) = state.clients[idx].confine_box();
            state.input_pointer.x = state.input_pointer.x.clamp(min_x, max_x);
            state.input_pointer.y = state.input_pointer.y.clamp(min_y, max_y);
        }
    }
}

pub(super) fn handle_pointer_motion<B: InputBackend, E: PointerMotionEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    state.note_pointer_activity();
    let serial = SERIAL_COUNTER.next_serial();
    let delta = (event.delta_x(), event.delta_y()).into();
    let delta_unaccel = (event.delta_x_unaccel(), event.delta_y_unaccel()).into();
    let time = event.time();

    // Save the pre-move cursor position so we can restore it for
    // pointer-constraints-v1 lock requests (FPS games etc.).
    let prev_x = state.input_pointer.x;
    let prev_y = state.input_pointer.y;

    state.input_pointer.x += event.delta_x();
    state.input_pointer.y += event.delta_y();
    state.clamp_pointer_to_outputs();
    state.input_pointer.motion_events += 1;
    state.refresh_pointer_monitor_tracking();
    // The hidden desktop must not react to edge focus or honour a stale
    // desktop pointer constraint while the session lock owns input.
    if !state.session_locked {
        // Edge-scroller pointer focus (no-op unless
        // `edge_scroller_focus_allow_speed > 0`). Speed = this event's
        // accelerated motion magnitude.
        let edge_speed = (event.delta_x().powi(2) + event.delta_y().powi(2)).sqrt();
        state.maybe_edge_scroller_focus(edge_speed);
    }

    constrain_pointer(state, prev_x, prev_y);

    let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
    log_pointer_motion(state, "relative", pos);
    request_cursor_repaint(state, Point::from((prev_x, prev_y)), pos);

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
    let (kbd_focus, ptr_focus) = hit_test(state, pos);
    apply_sloppy_focus(state, kbd_focus.as_ref().map(|(t, _)| t));
    update_overview_hover(state, pos);

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
                time: event.time(),
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

pub(super) fn handle_pointer_motion_abs<B: InputBackend, E: PointerMotionAbsoluteEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    state.note_pointer_activity();
    let serial = SERIAL_COUNTER.next_serial();
    let old_pos = Point::from((state.input_pointer.x, state.input_pointer.y));
    // Absolute devices (tablets in mouse mode, VM pointers) span the
    // whole output layout, not just the first output at the origin.
    let layout = state
        .space
        .outputs()
        .filter_map(|output| state.space.output_geometry(output))
        .reduce(|a, b| a.merge(b));
    if let Some(layout) = layout {
        let pos = event.position_transformed(layout.size) + layout.loc.to_f64();
        state.input_pointer.x = pos.x;
        state.input_pointer.y = pos.y;
        state.clamp_pointer_to_outputs();
        state.input_pointer.motion_events += 1;
        state.refresh_pointer_monitor_tracking();
        constrain_pointer(state, old_pos.x, old_pos.y);
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        log_pointer_motion(state, "absolute", pos);
        request_cursor_repaint(state, old_pos, pos);
        if state.session_locked {
            state.refresh_keyboard_focus();
        }

        let (kbd_focus, ptr_focus) = hit_test(state, pos);
        apply_sloppy_focus(state, kbd_focus.as_ref().map(|(t, _)| t));
        update_overview_hover(state, pos);
        if let Some(ptr) = state.seat.get_pointer() {
            ptr.motion(
                state,
                ptr_focus,
                &MotionEvent {
                    location: pos,
                    serial,
                    time: event.time(),
                },
            );
            ptr.frame(state);
        }
        update_hot_corner(state);
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

/// Repaint only displays touched by the cursor's old/new hotspot. Pointer
/// motion can arrive hundreds of times per second; turning every event into a
/// global scene repaint needlessly wakes idle secondary outputs.
fn request_cursor_repaint(
    state: &mut MargoState,
    old_pos: Point<f64, Logical>,
    new_pos: Point<f64, Logical>,
) {
    if old_pos == new_pos {
        return;
    }

    // Common case (no overlapping outputs): at most one output under
    // each position — no allocation on this 1000 Hz path.
    let overlapping = state.space.output_under(old_pos).nth(1).is_some()
        || state.space.output_under(new_pos).nth(1).is_some();
    if !overlapping {
        let old_output = state.space.output_under(old_pos).next().cloned();
        let new_output = state.space.output_under(new_pos).next().cloned();
        if let Some(output) = &old_output {
            state.request_repaint_output(output);
        }
        if let Some(output) = new_output.filter(|o| Some(o) != old_output.as_ref()) {
            state.request_repaint_output(&output);
        }
        return;
    }
    let mut outputs: Vec<_> = state.space.output_under(old_pos).cloned().collect();
    for output in state.space.output_under(new_pos) {
        if !outputs.contains(output) {
            outputs.push(output.clone());
        }
    }
    for output in outputs {
        state.request_repaint_output(&output);
    }
}

pub(super) fn handle_pointer_button<B: InputBackend, E: PointerButtonEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    state.note_pointer_activity();
    let serial = SERIAL_COUNTER.next_serial();
    let btn_state = event.state();
    let button = event.button_code();
    let pos = Point::from((state.input_pointer.x, state.input_pointer.y));

    if state.session_locked {
        let ptr_focus = pointer_focus_under(state, pos);
        if let Some(ptr) = state.seat.get_pointer() {
            ptr.motion(
                state,
                ptr_focus,
                &MotionEvent {
                    location: pos,
                    serial,
                    time: event.time(),
                },
            );
            ptr.button(
                state,
                &ButtonEvent {
                    serial,
                    time: event.time(),
                    button,
                    state: btn_state,
                },
            );
            ptr.frame(state);
        }
        return;
    }

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
                    // wlr-layer-shell: a layer surface with
                    // `keyboard_interactivity = none` must NEVER take keyboard
                    // focus, even when clicked. Otherwise an on-screen keyboard
                    // (mkeys) — an Overlay layer surface that injects via
                    // zwp_virtual_keyboard — would steal focus from the app it
                    // is typing into the instant you tap a key, so the
                    // keystrokes land on the keyboard surface itself and the
                    // target window receives nothing. Leave keyboard focus
                    // untouched for such surfaces (pointer focus still works,
                    // so the keys are still clickable).
                    let takes_keyboard = match &target {
                        FocusTarget::LayerSurface(layer) => layer.with_cached_state(|data| {
                            data.keyboard_interactivity != KeyboardInteractivity::None
                        }),
                        _ => true,
                    };
                    // Clicking a window while an mshell-frame menu/pill is
                    // open (Exclusive Top layer — never the Overlay
                    // screenshot selector or similar modal grabs) closes
                    // it first, GNOME-quick-settings-style: Exclusive is
                    // unconditional (see `compute_desired_focus`), so
                    // without this the click would focus the window but
                    // the still-open menu keeps the keyboard regardless.
                    // `menu close-all` is the exact action the frame's own
                    // Esc shortcut already runs — idempotent, a no-op if
                    // nothing is open.
                    if matches!(target, FocusTarget::Window(_)) && top_menu_layer_open(state) {
                        let _ = crate::utils::spawn(["mshellctl", "menu", "close-all"]);
                    }
                    if takes_keyboard {
                        state.focus_surface(Some(target));
                    }
                }
                _ => {
                    state.focus_surface(None);
                }
            }

            // The cursor may not have moved since a programmatic layout
            // change (gesture `viewtoright`/`viewtoleft`, `super`+N tag
            // switch, overview close) hid the window it was over and
            // remapped `space`. Smithay only refreshes pointer focus on a
            // real motion event, so without this the button below is
            // delivered to the window that *used* to be under the cursor —
            // now hidden on another tag. Browsers / Electron apps
            // self-activate on click via xdg-activation, and that
            // activation switches the view back to the hidden window's tag
            // (its `!already_visible` guard passes because the window is
            // genuinely off-tag now) — the "swipe to a tag, then click
            // without moving, and it jumps back" bug. Re-send a
            // zero-distance motion, reusing this press's serial so a
            // client-side move/resize grab started from the click still
            // validates, so the button lands on whatever is genuinely
            // under the cursor now. See handlers/xdg_activation.rs.
            let ptr_focus = pointer_focus_under(state, pos);
            if let Some(ptr) = state.seat.get_pointer() {
                ptr.motion(
                    state,
                    ptr_focus,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time(),
                    },
                );
            }
        }
    }
    if let Some(ptr) = state.seat.get_pointer() {
        ptr.button(
            state,
            &ButtonEvent {
                serial,
                time: event.time(),
                button: event.button_code(),
                state: btn_state,
            },
        );
        ptr.frame(state);
    }
}

pub(super) fn handle_pointer_axis<B: InputBackend, E: PointerAxisEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    state.note_pointer_activity();
    if state.session_locked {
        let pos = Point::from((state.input_pointer.x, state.input_pointer.y));
        let ptr_focus = pointer_focus_under(state, pos);
        if let Some(ptr) = state.seat.get_pointer() {
            ptr.motion(
                state,
                ptr_focus,
                &MotionEvent {
                    location: pos,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time(),
                },
            );
            let source = event.source();
            let mut frame = AxisFrame::new(event.time()).source(source);
            for axis in [Axis::Horizontal, Axis::Vertical] {
                if let Some(amount) = event.amount(axis) {
                    frame = frame.value(axis, amount);
                }
                if let Some(v120) = event.amount_v120(axis) {
                    frame = frame.v120(axis, v120 as i32);
                }
            }
            ptr.axis(state, frame);
            ptr.frame(state);
        }
        return;
    }
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

    if run_axis_binding(state, &event) {
        return;
    }

    // AxisFrame::source() and AxisFrame::value() both use smithay::backend::input types.
    let source = event.source();
    let mut frame = AxisFrame::new(event.time()).source(source);
    // mango: touchpad scrolls scale by trackpad_scroll_factor, wheels by
    // axis_scroll_factor.
    let factor = match source {
        AxisSource::Finger | AxisSource::Continuous => state.config.trackpad_scroll_factor,
        _ => state.config.axis_scroll_factor,
    };

    for axis in [Axis::Horizontal, Axis::Vertical] {
        if event.amount_v120(axis).is_none() && event.amount(axis).is_none() {
            continue;
        }
        let amount = event
            .amount(axis)
            .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) / 120.0 * 3.0);

        // libinput emits a 0.0-amount finger event when the fingers lift off
        // the touchpad; forward it as wl_pointer.axis_stop so the client ends
        // the scroll sequence. Firefox/Gecko ignores touchpad (finger-source)
        // scrolling entirely without this stop — only the wheel (v120) path
        // works otherwise. (Our virtual-pointer path already emits stop.)
        if source == AxisSource::Finger && amount == 0.0 {
            frame = frame.stop(axis);
            continue;
        }

        frame = frame.value(axis, amount * factor);
        if let Some(v120) = event.amount_v120(axis) {
            frame = frame.v120(axis, (v120 * factor).round() as i32);
        }
    }

    if let Some(ptr) = state.seat.get_pointer() {
        ptr.axis(state, frame);
        ptr.frame(state);
    }
}

/// `axisbind = MODS,DIR,action,…` (mango semantics): a scroll in DIR with
/// exactly MODS held runs the action instead of reaching the client. A
/// continuous scroll keeps firing at most once per
/// `axis_bind_apply_timeout` ms, but reversing direction fires at once.
/// Returns whether the event was consumed.
fn run_axis_binding<B: InputBackend, E: PointerAxisEvent<B>>(
    state: &mut MargoState,
    event: &E,
) -> bool {
    if state.config.axis_bindings.is_empty() {
        return false;
    }
    let delta = |axis| {
        event
            .amount(axis)
            .or_else(|| event.amount_v120(axis))
            .filter(|d| *d != 0.0)
    };
    // Directions as parsed: 0 up, 1 down, 2 left, 3 right.
    let (direction, delta) = match (delta(Axis::Vertical), delta(Axis::Horizontal)) {
        (Some(d), _) => (if d > 0.0 { 1 } else { 0 }, d),
        (None, Some(d)) => (if d > 0.0 { 3 } else { 2 }, d),
        (None, None) => return false,
    };
    let mods = state
        .seat
        .get_keyboard()
        .map(|k| smithay_mods_to_margo(&k.modifier_state()))
        .unwrap_or_else(margo_config::Modifiers::empty);
    let Some(bind) = state
        .config
        .axis_bindings
        .iter()
        .find(|b| b.modifiers == mods && b.direction == direction)
        .cloned()
    else {
        return false;
    };
    let now = event.time().millis();
    let sign: i8 = if delta > 0.0 { 1 } else { -1 };
    let reversed =
        state.input_timers.axis_bind_dir != 0 && state.input_timers.axis_bind_dir != sign;
    let due =
        now.wrapping_sub(state.input_timers.axis_bind_time) > state.config.axis_bind_apply_timeout;
    state.input_timers.axis_bind_time = now;
    state.input_timers.axis_bind_dir = sign;
    if due || reversed {
        crate::dispatch::dispatch_action(state, &bind.action, &bind.arg);
        state.request_repaint();
    }
    true
}

/// True if an mshell-frame menu/pill is currently open — i.e. some Top
/// layer surface holds Exclusive keyboard interactivity. Deliberately
/// excludes Overlay (the screenshot region selector and similar modal
/// grabs, which a stray window click should never dismiss) and skips
/// the per-output `layer_accepts_input_on_output` / mapping check that
/// `exclusive_keyboard_layer` does — the caller only uses this to gate
/// an idempotent `mshellctl menu close-all` spawn, so a false positive
/// costs nothing.
fn top_menu_layer_open(state: &MargoState) -> bool {
    state.protocols.layer_shell.layer_surfaces().any(|layer| {
        layer.with_cached_state(|data| {
            data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && data.layer == WlrLayer::Top
        })
    })
}
