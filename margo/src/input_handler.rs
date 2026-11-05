#![allow(dead_code)]
//! Translates raw libinput / winit events into compositor actions.

use smithay::backend::input::{
    Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent,
    InputBackend, InputEvent,
};

use crate::state::MargoState;

mod gestures;
mod hit;
mod hot_corner;
mod keyboard;
mod pointer;

pub(crate) use gestures::{
    dispatch_swipe, gesture_hold_begin, gesture_hold_end, gesture_pinch_begin, gesture_pinch_end,
    gesture_pinch_update, gesture_swipe_begin, gesture_swipe_end, gesture_swipe_update,
};
pub(crate) use hit::hit_test;
#[cfg(test)]
pub(crate) use hot_corner::update_hot_corner;
use keyboard::handle_keyboard;
use pointer::{
    handle_pointer_axis, handle_pointer_button, handle_pointer_motion, handle_pointer_motion_abs,
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
            | InputEvent::GesturePinchBegin { .. }
            | InputEvent::GesturePinchUpdate { .. }
            | InputEvent::GesturePinchEnd { .. }
            | InputEvent::GestureHoldBegin { .. }
            | InputEvent::GestureHoldEnd { .. }
            | InputEvent::TouchDown { .. }
            | InputEvent::TouchMotion { .. }
            | InputEvent::TouchUp { .. }
    ) {
        let seat = state.seat.clone();
        state.protocols.idle_notifier.notify_activity(&seat);
        // Safety net: any real input wakes a DPMS-off panel, so a manual /
        // idle-daemon screen-off can never strand the user on black. Swallow
        // the waking event so the keystroke / click that turns the screen
        // back on doesn't also land on the focused surface (no stray newline
        // in the terminal you ran `mctl dispatch dpms off` from).
        if state.wake_dpms_on_input() {
            return;
        }
    }

    match event {
        InputEvent::Keyboard { event } => handle_keyboard(state, event),
        InputEvent::PointerMotion { event } => handle_pointer_motion(state, event),
        InputEvent::PointerMotionAbsolute { event } => handle_pointer_motion_abs(state, event),
        InputEvent::PointerButton { event } => handle_pointer_button(state, event),
        InputEvent::PointerAxis { event } => handle_pointer_axis(state, event),
        InputEvent::GestureSwipeBegin { event } => {
            gesture_swipe_begin(state, event.fingers(), event.time().millis())
        }
        InputEvent::GestureSwipeUpdate { event } => gesture_swipe_update(
            state,
            event.time().millis(),
            (event.delta_x(), event.delta_y()),
        ),
        InputEvent::GestureSwipeEnd { event } => {
            gesture_swipe_end(state, event.time().millis(), event.cancelled())
        }
        InputEvent::GesturePinchBegin { event } => {
            gesture_pinch_begin(state, event.fingers(), event.time().millis())
        }
        InputEvent::GesturePinchUpdate { event } => gesture_pinch_update(
            state,
            event.time().millis(),
            (event.delta_x(), event.delta_y()),
            event.scale(),
            event.rotation(),
        ),
        InputEvent::GesturePinchEnd { event } => {
            gesture_pinch_end(state, event.time().millis(), event.cancelled())
        }
        InputEvent::GestureHoldBegin { event } => {
            gesture_hold_begin(state, event.fingers(), event.time().millis())
        }
        InputEvent::GestureHoldEnd { event } => {
            gesture_hold_end(state, event.time().millis(), event.cancelled())
        }
        InputEvent::TouchDown { event } => crate::input::touch::handle_touch_down(state, event),
        InputEvent::TouchMotion { event } => crate::input::touch::handle_touch_motion(state, event),
        InputEvent::TouchUp { event } => crate::input::touch::handle_touch_up(state, event),
        InputEvent::TouchFrame { .. } => crate::input::touch::touch_frame(state),
        InputEvent::TouchCancel { .. } => crate::input::touch::touch_cancel(state),
        InputEvent::SwitchToggle { event } => handle_switch_toggle(state, event),
        _ => {}
    }
}

/// `switchbind = fold|unfold,action,…` (mango): a lid / tablet-mode switch
/// turning on (`fold`) or off (`unfold`) runs the first matching bind.
fn handle_switch_toggle<B: InputBackend, E: smithay::backend::input::SwitchToggleEvent<B>>(
    state: &mut MargoState,
    event: E,
) {
    use smithay::backend::input::SwitchState;
    let fold = match event.state() {
        SwitchState::On => margo_config::FoldState::Fold,
        SwitchState::Off => margo_config::FoldState::Unfold,
    };
    let Some(bind) = state
        .config
        .switch_bindings
        .iter()
        .find(|b| b.fold == fold)
        .cloned()
    else {
        return;
    };
    tracing::info!(switch = ?event.switch(), ?fold, action = %bind.action, "switchbind");
    crate::dispatch::dispatch_action(state, &bind.action, &bind.arg);
    state.request_repaint();
}
