//! Interactive move + resize grabs.
//!
//! Triggered by `xdg_toplevel.move` / `xdg_toplevel.resize` (CSD apps
//! dragging their titlebar / resize edge) and by the `moveresize`
//! action when a user hits Super+drag-to-move / Super+right-drag-to-
//! resize. Pattern is borrowed from anvil's `shell/grabs.rs`, slimmed
//! to margo's data model:
//!
//! * Move: as the cursor moves, we shift the grabbed window's
//!   `float_geom` and force `is_floating = true`. Tiled scroller layouts
//!   keep their reservation but the dragged window pops out of the
//!   layout for the duration of the grab.
//! * Resize: similar, but new geometry is computed from the edge
//!   bitmask and we send an `xdg_toplevel.configure(new_size)` so the
//!   client redraws to fit; commit handler picks up the new buffer and
//!   we converge.
//!
//! Both grabs end on the next button-up event (the same button that
//! initiated the grab — smithay tracks `current_pressed`).

use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Size},
};

use crate::state::MargoState;

// ── Move grab ─────────────────────────────────────────────────────────────────

pub struct MoveSurfaceGrab {
    pub start_data: GrabStartData<MargoState>,
    pub window: Window,
    pub initial_loc: Point<i32, Logical>,
    /// `true` when the dragged client was classically tiled *or*
    /// `Mosaic`-governed at grab start. Drives mango 0.13's drag-tile-to-
    /// tile flow, extended to Mosaic: on release we look for a *same-kind*
    /// client under the cursor (`is_valid_drag_tile_target`) and swap
    /// positions instead of leaving the window floating in mid-air.
    /// `false` for CSD `xdg_toplevel.move` requests — those keep the
    /// legacy behaviour (the client becomes floating wherever the user
    /// drops it).
    pub was_tiled: bool,
    /// Pre-grab floating geometry. Restored on release when no
    /// valid drop target was found, so the user can undo a drag
    /// by dropping over empty space / the same tile. Only
    /// consulted when `was_tiled` is `true`.
    pub original_float_geom: crate::layout::Rect,
}

impl PointerGrab<MargoState> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // No client gets pointer events while we're dragging.
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_loc = self.initial_loc.to_f64() + delta;
        let new_loc = new_loc.to_i32_round();

        if let Some(idx) = data.client_idx(&self.window) {
            data.clients[idx].is_floating = true;
            // Drag-tile-small visual: when the user is dragging a
            // window that was tiled at grab start AND the config
            // flag is on, shrink the floating geometry to a 300×300
            // thumbnail centred on the cursor so the tiles
            // underneath stay visible. Otherwise fall through to
            // the normal "follow cursor" placement.
            if self.was_tiled && data.config.drag_tile_small {
                let cx = event.location.x.round() as i32;
                let cy = event.location.y.round() as i32;
                data.clients[idx].float_geom.x = cx - 150;
                data.clients[idx].float_geom.y = cy - 150;
                data.clients[idx].float_geom.width = 300;
                data.clients[idx].float_geom.height = 300;
            } else {
                data.clients[idx].float_geom.x = new_loc.x;
                data.clients[idx].float_geom.y = new_loc.y;
            }
        }

        throttled_grab_arrange(data, &self.window, self.was_tiled);

        // Phase 2 drop-target highlight: recomputed after arranging so
        // the candidate's geometry reflects any repack that repositioning
        // this drag just triggered (Mosaic repacks every other owned
        // client around the one being dragged). Mirrors exactly what
        // `resolve_drag_tile_drop` will look for on release, via the same
        // `find_drag_tile_target` — see `render::drag_highlight`.
        data.drag_swap_target = if self.was_tiled && data.config.drag_tile_to_tile {
            data.clients
                .iter()
                .position(|c| c.window == self.window)
                .and_then(|idx| {
                    let cursor = (
                        event.location.x.round() as i32,
                        event.location.y.round() as i32,
                    );
                    find_drag_tile_target(data, idx, cursor)
                })
                .map(|dst| data.clients[dst].id)
        } else {
            None
        };
    }

    fn relative_motion(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            // Grab's over either way — clear the "don't repack me" flag
            // before anything below touches arrange (`resolve_drag_tile_
            // drop` ends with its own `arrange_monitor`), and the
            // highlight so it can't outlive the drag it was previewing.
            if let Some(idx) = data.client_idx(&self.window) {
                data.clients[idx].interactive_grab = false;
            }
            data.drag_swap_target = None;
            if !(self.was_tiled && data.config.drag_tile_to_tile) {
                snap_floating_window(data, &self.window);
            }
            finish_grab_arrange(data, &self.window);
            // Drag-tile-to-tile end of grab. Only kicks in when
            // the grabbed window was tiled (or Mosaic-governed) at
            // start AND the config flag is on; CSD `xdg_toplevel.move`
            // requests are unaffected because their grab is built with
            // `was_tiled: false` over in xdg_shell.rs.
            if self.was_tiled && data.config.drag_tile_to_tile {
                resolve_drag_tile_drop(data, &self.window, self.original_float_geom);
            }
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut MargoState, handle: &mut PointerInnerHandle<'_, MargoState>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }
    fn gesture_swipe_update(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }
    fn gesture_swipe_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }
    fn gesture_pinch_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }
    fn gesture_pinch_update(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }
    fn gesture_pinch_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }
    fn gesture_hold_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }
    fn gesture_hold_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<MargoState> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut MargoState) {}
}

// ── Floating snap ────────────────────────────────────────────────────────────

/// `enable_floating_snap`: on release, pull a dragged floating window's
/// edges onto the nearest edge of another visible floating window, the
/// monitor or the work area when closer than `snap_distance` px (mango).
fn snap_floating_window(data: &mut MargoState, window: &Window) {
    if !data.config.enable_floating_snap || data.config.snap_distance <= 0 {
        return;
    }
    let Some(idx) = data.client_idx(window) else {
        return;
    };
    let c = &data.clients[idx];
    if !c.is_floating {
        return;
    }
    let Some(mon) = data.monitors.get(c.monitor) else {
        return;
    };
    let tagset = mon.current_tagset();
    let outset = |r: crate::layout::Rect, bw: i32| {
        crate::layout::Rect::new(r.x - bw, r.y - bw, r.width + 2 * bw, r.height + 2 * bw)
    };
    let bw = c.border_width as i32;
    let me = outset(c.float_geom, bw);
    let others: Vec<crate::layout::Rect> = data
        .clients
        .iter()
        .enumerate()
        .filter(|&(i, o)| {
            i != idx && o.is_floating && !o.is_killing && o.is_visible_on(c.monitor, tagset)
        })
        .map(|(_, o)| outset(o.geom, o.border_width as i32))
        .collect();
    let (dx, dy) = floating_snap_offset(
        me,
        &others,
        &[mon.monitor_area, mon.work_area],
        data.config.snap_distance,
    );
    data.clients[idx].float_geom.x += dx;
    data.clients[idx].float_geom.y += dy;
}

/// Offset that snaps `me` per mango's `client_snap`: per axis, the nearest
/// non-negative gap to a neighbour's facing edge or to an inner edge of a
/// `bounds` rect, applied if it is under `distance`.
pub(crate) fn floating_snap_offset(
    me: crate::layout::Rect,
    others: &[crate::layout::Rect],
    bounds: &[crate::layout::Rect],
    distance: i32,
) -> (i32, i32) {
    let (mut left, mut right, mut up, mut down) = (i32::MAX, i32::MAX, i32::MAX, i32::MAX);
    let take = |slot: &mut i32, gap: i32| {
        if gap >= 0 && gap < *slot {
            *slot = gap;
        }
    };
    for o in others {
        take(&mut left, me.x - (o.x + o.width));
        take(&mut right, o.x - (me.x + me.width));
        take(&mut up, me.y - (o.y + o.height));
        take(&mut down, o.y - (me.y + me.height));
    }
    for b in bounds {
        take(&mut left, me.x - b.x);
        take(&mut right, b.x + b.width - (me.x + me.width));
        take(&mut up, me.y - b.y);
        take(&mut down, b.y + b.height - (me.y + me.height));
    }
    let dx = if left < right && left < distance {
        -left
    } else if right <= left && right < distance {
        right
    } else {
        0
    };
    let dy = if up < down && up < distance {
        -up
    } else if down <= up && down < distance {
        down
    } else {
        0
    };
    (dx, dy)
}

#[cfg(test)]
mod snap_tests {
    use super::floating_snap_offset;
    use crate::layout::Rect;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn snaps_to_screen_edges_within_distance() {
        let me = Rect::new(20, 1080 - 210, 300, 200);
        assert_eq!(floating_snap_offset(me, &[], &[SCREEN], 30), (-20, 10));
    }

    #[test]
    fn snaps_next_to_a_neighbour() {
        let other = Rect::new(500, 500, 200, 200);
        let me = Rect::new(715, 520, 100, 100);
        assert_eq!(floating_snap_offset(me, &[other], &[SCREEN], 30).0, -15);
    }

    #[test]
    fn leaves_far_windows_alone() {
        let me = Rect::new(800, 400, 100, 100);
        assert_eq!(floating_snap_offset(me, &[], &[SCREEN], 30), (0, 0));
    }
}

// ── Grab arrange throttling ───────────────────────────────────────────────────

/// The monitor `window` lives on (falls back to the focused one).
fn grab_monitor(data: &MargoState, window: &Window) -> usize {
    data.client_idx(window)
        .map(|idx| data.clients[idx].monitor)
        .unwrap_or_else(|| data.focused_monitor())
}

/// `Ok` when an arrange may run now, `Err(wait)` when the last one was
/// less than `interval` ago.
fn grab_arrange_due(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> Result<(), std::time::Duration> {
    match last {
        Some(t) if now.saturating_duration_since(t) < interval => {
            Err(interval - now.saturating_duration_since(t))
        }
        _ => Ok(()),
    }
}

/// Minimum time between grab re-arranges on `mon`: the configured
/// `drag_{tile,floating}_refresh_interval` when set (> 0), otherwise one
/// frame of the monitor's current mode (mango ca34a6ac) — 60 Hz when the
/// mode is unknown.
fn grab_arrange_interval(data: &MargoState, mon: usize, tiled: bool) -> std::time::Duration {
    let configured = if tiled {
        data.config.drag_tile_refresh_interval
    } else {
        data.config.drag_floating_refresh_interval
    };
    if configured > 0.0 {
        return std::time::Duration::from_secs_f32(configured.min(1000.0) / 1000.0);
    }
    let refresh_mhz = data
        .monitors
        .get(mon)
        .and_then(|m| m.output.current_mode())
        .map_or(0, |mode| mode.refresh);
    frame_interval(refresh_mhz)
}

/// One frame at `refresh_mhz` (mHz, as smithay reports a mode),
/// 60 Hz when unknown, never below 1 ms.
fn frame_interval(refresh_mhz: i32) -> std::time::Duration {
    let mhz = if refresh_mhz > 0 { refresh_mhz } else { 60_000 };
    std::time::Duration::from_secs_f64((1000.0 / f64::from(mhz)).max(0.001))
}

/// Re-arrange the grabbed window's monitor at most once per
/// [`grab_arrange_interval`]. A pointer can report
/// motion at 1000 Hz; arranging (layout, configures, borders, z-order)
/// on every event floods a resizing client with configures it can't ack
/// in time. A skipped update is applied by a one-shot timer at the end
/// of the interval, so the window still lands where the pointer stopped.
fn throttled_grab_arrange(data: &mut MargoState, window: &Window, tiled: bool) {
    let interval = grab_arrange_interval(data, grab_monitor(data, window), tiled);
    let now = std::time::Instant::now();
    match grab_arrange_due(data.grab_arrange_last, now, interval) {
        Ok(()) => {
            if let Some(token) = data.grab_arrange_timer.take() {
                data.loop_handle.remove(token);
            }
            data.grab_arrange_last = Some(now);
            let mon = grab_monitor(data, window);
            if mon < data.monitors.len() {
                data.arrange_monitor(mon);
            }
        }
        Err(_) if data.grab_arrange_timer.is_some() => {}
        Err(wait) => {
            let window = window.clone();
            let timer = calloop::timer::Timer::from_duration(wait);
            match data
                .loop_handle
                .insert_source(timer, move |_, _, state: &mut MargoState| {
                    // Clear first: the source is dropped on return.
                    state.grab_arrange_timer = None;
                    state.grab_arrange_last = Some(std::time::Instant::now());
                    let mon = grab_monitor(state, &window);
                    if mon < state.monitors.len() {
                        state.arrange_monitor(mon);
                    }
                    calloop::timer::TimeoutAction::Drop
                }) {
                Ok(token) => data.grab_arrange_timer = Some(token),
                Err(e) => tracing::warn!("grab arrange timer: {e}"),
            }
        }
    }
}

/// End of a move/resize grab: drop any pending throttled update and
/// arrange once with the final geometry.
fn finish_grab_arrange(data: &mut MargoState, window: &Window) {
    if let Some(token) = data.grab_arrange_timer.take() {
        data.loop_handle.remove(token);
    }
    data.grab_arrange_last = None;
    let mon = grab_monitor(data, window);
    if mon < data.monitors.len() {
        data.arrange_monitor(mon);
    }
}

#[cfg(test)]
mod throttle_tests {
    use super::{frame_interval, grab_arrange_due};
    use std::time::{Duration, Instant};

    #[test]
    fn auto_interval_is_one_frame_of_the_mode() {
        let ms = |mhz| frame_interval(mhz).as_secs_f64() * 1000.0;
        assert!((ms(60_000) - 16.667).abs() < 0.01);
        assert!((ms(144_000) - 6.944).abs() < 0.01);
        // Unknown refresh falls back to 60 Hz.
        assert!((ms(0) - 16.667).abs() < 0.01);
    }

    #[test]
    fn first_update_and_updates_past_the_interval_run_immediately() {
        let now = Instant::now();
        let interval = Duration::from_millis(8);
        assert_eq!(grab_arrange_due(None, now, interval), Ok(()));
        assert_eq!(
            grab_arrange_due(Some(now - Duration::from_millis(9)), now, interval),
            Ok(())
        );
    }

    #[test]
    fn updates_inside_the_interval_wait_for_the_remainder() {
        let now = Instant::now();
        let wait = grab_arrange_due(
            Some(now - Duration::from_millis(3)),
            now,
            Duration::from_millis(8),
        );
        assert_eq!(wait, Err(Duration::from_millis(5)));
    }

    #[test]
    fn zero_interval_never_throttles() {
        let now = Instant::now();
        assert_eq!(grab_arrange_due(Some(now), now, Duration::ZERO), Ok(()));
    }
}

/// The corner a compositor-initiated resize grabs (`drag_corner`: 0 TL,
/// 1 TR, 2 BL, 3 BR, anything else = the corner nearest the cursor) and
/// that corner's position — where `drag_warp_cursor` puts the pointer.
pub(crate) fn resize_corner(
    drag_corner: i32,
    cursor: (f64, f64),
    geom: crate::layout::Rect,
) -> (ResizeEdge, Point<f64, Logical>) {
    let (left, top) = match drag_corner {
        0 => (true, true),
        1 => (false, true),
        2 => (true, false),
        3 => (false, false),
        _ => (
            cursor.0 < geom.x as f64 + geom.width as f64 / 2.0,
            cursor.1 < geom.y as f64 + geom.height as f64 / 2.0,
        ),
    };
    let edge = match (left, top) {
        (true, true) => ResizeEdge::TopLeft,
        (false, true) => ResizeEdge::TopRight,
        (true, false) => ResizeEdge::BottomLeft,
        (false, false) => ResizeEdge::BottomRight,
    };
    let x = if left { geom.x } else { geom.x + geom.width };
    let y = if top { geom.y } else { geom.y + geom.height };
    (edge, Point::from((x as f64, y as f64)))
}

#[cfg(test)]
mod corner_tests {
    use super::resize_corner;
    use crate::layout::Rect;
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;

    #[test]
    fn fixed_corners_map_to_edges_and_positions() {
        let g = Rect::new(100, 100, 400, 300);
        let (e, p) = resize_corner(1, (0.0, 0.0), g);
        assert_eq!(e, ResizeEdge::TopRight);
        assert_eq!((p.x, p.y), (500.0, 100.0));
        let (e, p) = resize_corner(2, (0.0, 0.0), g);
        assert_eq!(e, ResizeEdge::BottomLeft);
        assert_eq!((p.x, p.y), (100.0, 400.0));
    }

    #[test]
    fn auto_picks_the_corner_nearest_the_cursor() {
        let g = Rect::new(0, 0, 400, 300);
        assert_eq!(resize_corner(4, (10.0, 290.0), g).0, ResizeEdge::BottomLeft);
        assert_eq!(resize_corner(4, (390.0, 10.0), g).0, ResizeEdge::TopRight);
    }
}

// ── Resize grab ───────────────────────────────────────────────────────────────

pub struct ResizeSurfaceGrab {
    pub start_data: GrabStartData<MargoState>,
    pub window: Window,
    pub edges: ResizeEdge,
    pub initial_loc: Point<i32, Logical>,
    pub initial_size: Size<i32, Logical>,
}

impl ResizeSurfaceGrab {
    /// Apply the cursor delta to compute new geometry. Edges encode which
    /// corner is being dragged: TOP/BOTTOM/LEFT/RIGHT (or combinations).
    fn compute_new_geom(
        &self,
        delta: Point<f64, Logical>,
    ) -> (Point<i32, Logical>, Size<i32, Logical>) {
        let mut new_loc = self.initial_loc;
        let mut new_size = self.initial_size;
        let dx = delta.x as i32;
        let dy = delta.y as i32;

        // Right edge: width grows with positive dx; loc.x unchanged.
        if matches!(
            self.edges,
            ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
        ) {
            new_size.w = (self.initial_size.w + dx).max(1);
        }
        // Left edge: width grows with negative dx; loc.x shifts.
        if matches!(
            self.edges,
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
        ) {
            new_size.w = (self.initial_size.w - dx).max(1);
            new_loc.x = self.initial_loc.x + dx;
        }
        // Bottom edge: height grows with positive dy.
        if matches!(
            self.edges,
            ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
        ) {
            new_size.h = (self.initial_size.h + dy).max(1);
        }
        // Top edge: height grows with negative dy; loc.y shifts.
        if matches!(
            self.edges,
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
        ) {
            new_size.h = (self.initial_size.h - dy).max(1);
            new_loc.y = self.initial_loc.y + dy;
        }
        (new_loc, new_size)
    }
}

impl PointerGrab<MargoState> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let (new_loc, new_size) = self.compute_new_geom(delta);

        if let Some(idx) = data.client_idx(&self.window) {
            data.clients[idx].is_floating = true;
            data.clients[idx].float_geom.x = new_loc.x;
            data.clients[idx].float_geom.y = new_loc.y;
            data.clients[idx].float_geom.width = new_size.w;
            data.clients[idx].float_geom.height = new_size.h;
        }

        throttled_grab_arrange(data, &self.window, false);
    }

    fn relative_motion(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            if let Some(idx) = data.client_idx(&self.window) {
                data.clients[idx].interactive_grab = false;
            }
            finish_grab_arrange(data, &self.window);
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut MargoState, handle: &mut PointerInnerHandle<'_, MargoState>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }
    fn gesture_swipe_update(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }
    fn gesture_swipe_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }
    fn gesture_pinch_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }
    fn gesture_pinch_update(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }
    fn gesture_pinch_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }
    fn gesture_hold_begin(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }
    fn gesture_hold_end(
        &mut self,
        data: &mut MargoState,
        handle: &mut PointerInnerHandle<'_, MargoState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<MargoState> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut MargoState) {}
}

// ── drag_tile_to_tile drop resolver ──────────────────────────────────────────

/// Called from `MoveSurfaceGrab::button` when the left button is
/// released. If the cursor is over another *same-kind* client — both
/// classically tiled, or both `Mosaic`-governed — swap their positions
/// (mango's drag_tile_to_tile; for Mosaic this reorders `mosaic_arrange`'s
/// shelf-pack input, which is Mosaic's version of "combine two windows").
/// Otherwise just restore the dragged window's pre-grab floating geometry
/// so the drag_tile_small thumbnail doesn't linger as a 300×300 floater —
/// for a Mosaic client this is a no-op in practice, since the very next
/// `reconcile_mosaic_layout` pass repacks it from the work area + its own
/// min/max size hints regardless of where `float_geom` currently sits.
pub(crate) fn resolve_drag_tile_drop(
    data: &mut MargoState,
    dragged: &Window,
    original_float_geom: crate::layout::Rect,
) {
    let Some(src) = data.client_idx(dragged) else {
        return;
    };
    let src_is_mosaic = data.clients[src].auto_float_owner == Some(crate::layout::LayoutId::Mosaic);

    // Always restore the pre-grab float_geom so the next time the
    // user un-tiles this window it falls back to its real size,
    // not the 300×300 thumbnail. A classically-tiled source goes back to
    // being tiled (`is_floating = false`); a Mosaic source stays floating
    // — Mosaic never un-floats a client just because a drag ended, only
    // when the tag leaves Mosaic entirely (`reconcile_mosaic_layout`).
    data.clients[src].float_geom = original_float_geom;
    if !src_is_mosaic {
        data.clients[src].is_floating = false;
    }

    let cursor = (data.input_pointer.x as i32, data.input_pointer.y as i32);
    if let Some(dst) = find_drag_tile_target(data, src, cursor) {
        data.swap_clients(src, dst);
    }
    data.drag_swap_target = None;

    let mon = data.focused_monitor();
    if mon < data.monitors.len() {
        data.arrange_monitor(mon);
    }
}

/// Find the drag-tile-to-tile target at `cursor` (global-logical) for the
/// client at index `src` in `data.clients`: the first other client, on
/// `src`'s monitor and current tag, whose geometry contains `cursor` and
/// which passes [`is_valid_drag_tile_target`] for `src`'s kind. Scans
/// actual client geometry rather than `space.element_under(cursor)` —
/// with `drag_tile_small` (on by default) the dragged window itself
/// shrinks to a 300×300 thumbnail centred exactly on the cursor, so it
/// always contains the cursor point too; `element_under` would return
/// whichever of the two is topmost, which tracks z-order/creation order
/// rather than "what the user visually dropped on". Excluding `src`
/// outright here sidesteps that self-occlusion entirely.
///
/// Shared by [`resolve_drag_tile_drop`] (the actual swap, on release) and
/// `MoveSurfaceGrab::motion` (the live highlight preview,
/// `render::drag_highlight`) so the two can never disagree about what
/// counts as a valid target.
fn find_drag_tile_target(data: &MargoState, src: usize, cursor: (i32, i32)) -> Option<usize> {
    let src_is_mosaic = data.clients[src].auto_float_owner == Some(crate::layout::LayoutId::Mosaic);
    let src_monitor = data.clients[src].monitor;
    let tagset = data
        .monitors
        .get(src_monitor)
        .map(|m| m.current_tagset())
        .unwrap_or(0);
    let (cx, cy) = cursor;
    data.clients.iter().enumerate().find_map(|(i, c)| {
        if i == src
            || c.monitor != src_monitor
            || c.is_overlay
            || c.is_in_scratchpad
            || c.is_minimized
            || !c.is_visible_on(src_monitor, tagset)
            || cx < c.geom.x
            || cx >= c.geom.x + c.geom.width
            || cy < c.geom.y
            || cy >= c.geom.y + c.geom.height
        {
            return None;
        }
        let dst_is_mosaic = c.auto_float_owner == Some(crate::layout::LayoutId::Mosaic);
        is_valid_drag_tile_target(src_is_mosaic, c.is_floating, dst_is_mosaic).then_some(i)
    })
}

/// Whether a drop target is a valid drag-tile-to-tile partner for the
/// dragged client, given whether the *source* is `Mosaic`-governed. Pure
/// so it's directly unit-testable without a `Space`/pointer fixture —
/// `resolve_drag_tile_drop`'s own hit-testing needs a real committed
/// client buffer, which the compositor always has but a synthetic test
/// harness does not.
///
/// A `Mosaic` source may only swap with another `Mosaic`-governed client
/// (reordering `mosaic_arrange`'s packing input); a classically-tiled
/// source may only swap with another classically-tiled client (`!is_floating`)
/// — matching the pre-Mosaic behaviour exactly. Cross-kind drops (e.g. a
/// `Tile`-family window dropped on a `Mosaic` one) are never valid.
fn is_valid_drag_tile_target(
    src_is_mosaic: bool,
    dst_is_floating: bool,
    dst_is_mosaic: bool,
) -> bool {
    if src_is_mosaic {
        dst_is_mosaic
    } else {
        !dst_is_floating
    }
}

#[cfg(test)]
mod drag_tile_target_tests {
    use super::is_valid_drag_tile_target;

    #[test]
    fn mosaic_source_only_targets_mosaic() {
        assert!(is_valid_drag_tile_target(true, true, true));
        assert!(!is_valid_drag_tile_target(true, true, false));
        assert!(!is_valid_drag_tile_target(true, false, false));
    }

    #[test]
    fn tiled_source_only_targets_tiled() {
        assert!(is_valid_drag_tile_target(false, false, false));
        assert!(!is_valid_drag_tile_target(false, true, false));
        assert!(!is_valid_drag_tile_target(false, true, true));
    }
}
