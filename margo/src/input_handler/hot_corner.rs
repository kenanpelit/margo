//! Hot corners: arm when the pointer reaches a configured corner, fire
//! the bound action after the dwell delay unless something blocks it.

use crate::state::MargoState;

pub(crate) fn update_hot_corner(state: &mut MargoState) {
    if hot_corner_blocked(state) {
        // Bail before even checking the corners; a re-entry restarts
        // the dwell cleanly once the guard lifts.
        set_hot_corner(state, None);
        return;
    }
    let corner = hot_corner_at(state, state.input_pointer.x, state.input_pointer.y);
    if state.input_timers.hot_corner_dwelling != corner {
        set_hot_corner(state, corner);
        return;
    }
    fire_hot_corner_if_due(state);
}

/// States that own the screen, where a corner trigger misbehaved:
///   * `session_locked` → `toggle_overview` while the lock surface owns
///     focus pushed the cursor through to the login prompt.
///   * a pointer or keyboard grab held by a popup → the trigger would
///     smash through the grab and dismiss the popup.
///   * a fullscreen focused window, when `hot_corner_disable_on_fullscreen`.
fn hot_corner_blocked(state: &MargoState) -> bool {
    if state.session_locked {
        return true;
    }
    if state.config.hot_corner_disable_on_fullscreen
        && state
            .focused_client_idx()
            .is_some_and(|idx| state.clients[idx].is_fullscreen)
    {
        return true;
    }
    state.seat.get_pointer().is_some_and(|p| p.is_grabbed())
        || state.seat.get_keyboard().is_some_and(|k| k.is_grabbed())
}

/// The corner `(x, y)` sits in: a 1×1 logical-pixel rect at one of the
/// four corners of any output (niri's `Rectangle::new(corner, (1, 1))`).
fn hot_corner_at(state: &MargoState, x: f64, y: f64) -> Option<crate::state::HotCorner> {
    use crate::state::HotCorner;
    state.space.outputs().find_map(|output| {
        let geo = state.space.output_geometry(output)?;
        let x0 = geo.loc.x as f64;
        let y0 = geo.loc.y as f64;
        let x1 = (geo.loc.x + geo.size.w) as f64;
        let y1 = (geo.loc.y + geo.size.h) as f64;
        [
            HotCorner::TopLeft,
            HotCorner::TopRight,
            HotCorner::BottomLeft,
            HotCorner::BottomRight,
        ]
        .into_iter()
        .find(|corner| {
            let zone = corner.zone_size(&state.config);
            let horizontal = match corner {
                HotCorner::TopLeft | HotCorner::BottomLeft => x >= x0 && x < x0 + zone,
                HotCorner::TopRight | HotCorner::BottomRight => x < x1 && x >= x1 - zone,
            };
            let vertical = match corner {
                HotCorner::TopLeft | HotCorner::TopRight => y >= y0 && y < y0 + zone,
                HotCorner::BottomLeft | HotCorner::BottomRight => y < y1 && y >= y1 - zone,
            };
            horizontal && vertical
        })
    })
}

/// Enter / leave a corner. Entering arms the dwell and a one-shot timer
/// for it, so a cursor parked in the corner fires without further
/// motion; leaving disarms both.
fn set_hot_corner(state: &mut MargoState, corner: Option<crate::state::HotCorner>) {
    if state.input_timers.hot_corner_dwelling == corner {
        return;
    }
    if let Some(token) = state.input_timers.hot_corner_timer.take() {
        state.loop_handle.remove(token);
    }
    state.input_timers.hot_corner_dwelling = corner;
    state.input_timers.hot_corner_armed_at = corner.map(|_| std::time::Instant::now());
    if corner.is_none() {
        return;
    }
    let dwell = std::time::Duration::from_millis(state.config.hot_corner_dwell_ms as u64);
    let timer = calloop::timer::Timer::from_duration(dwell);
    match state
        .loop_handle
        .insert_source(timer, |_, _, state: &mut MargoState| {
            // Clear before anything else: the source is dropped on return,
            // and `remove` on a stale token could hit a reused slot.
            state.input_timers.hot_corner_timer = None;
            if hot_corner_blocked(state) {
                set_hot_corner(state, None);
            } else {
                fire_hot_corner_if_due(state);
            }
            calloop::timer::TimeoutAction::Drop
        }) {
        Ok(token) => state.input_timers.hot_corner_timer = Some(token),
        Err(e) => tracing::warn!("hot corner dwell timer: {e}"),
    }
}

/// Fire the dwelt-in corner's action once the dwell has elapsed, then
/// disarm so it doesn't re-fire while the cursor stays put — the user
/// has to leave and re-enter.
fn fire_hot_corner_if_due(state: &mut MargoState) {
    let (Some(corner), Some(armed)) = (
        state.input_timers.hot_corner_dwelling,
        state.input_timers.hot_corner_armed_at,
    ) else {
        return;
    };
    let dwell = std::time::Duration::from_millis(state.config.hot_corner_dwell_ms as u64);
    if armed.elapsed() < dwell {
        return;
    }
    state.input_timers.hot_corner_armed_at = None;
    if let Some(token) = state.input_timers.hot_corner_timer.take() {
        state.loop_handle.remove(token);
    }
    let action = corner.action_str(&state.config).trim().to_string();
    if action.is_empty() {
        return;
    }
    tracing::info!(target: "hot_corner", corner = ?corner, action = %action, "fired");
    let arg = margo_config::Arg::default();
    crate::dispatch::dispatch_action(state, &action, &arg);
}
