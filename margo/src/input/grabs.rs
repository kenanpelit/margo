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
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
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
    /// `true` when the dragged client was tiled at grab start.
    /// Drives mango 0.13's drag-tile-to-tile flow: on release we
    /// look for another tile under the cursor and swap positions
    /// instead of leaving the window floating in mid-air.
    /// `false` for CSD `xdg_toplevel.move` requests — those keep
    /// the legacy behaviour (the client becomes floating wherever
    /// the user drops it).
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

        if let Some(idx) = data
            .clients
            .iter()
            .position(|c| c.window == self.window)
        {
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

        let mon = data.focused_monitor();
        if mon < data.monitors.len() {
            data.arrange_monitor(mon);
        }
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
            // Drag-tile-to-tile end of grab. Only kicks in when
            // the grabbed window was tiled at start AND the
            // config flag is on; CSD `xdg_toplevel.move` requests
            // are unaffected because their grab is built with
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
            ResizeEdge::Right
                | ResizeEdge::TopRight
                | ResizeEdge::BottomRight
        ) {
            new_size.w = (self.initial_size.w + dx).max(1);
        }
        // Left edge: width grows with negative dx; loc.x shifts.
        if matches!(
            self.edges,
            ResizeEdge::Left
                | ResizeEdge::TopLeft
                | ResizeEdge::BottomLeft
        ) {
            new_size.w = (self.initial_size.w - dx).max(1);
            new_loc.x = self.initial_loc.x + dx;
        }
        // Bottom edge: height grows with positive dy.
        if matches!(
            self.edges,
            ResizeEdge::Bottom
                | ResizeEdge::BottomLeft
                | ResizeEdge::BottomRight
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

        if let Some(idx) = data
            .clients
            .iter()
            .position(|c| c.window == self.window)
        {
            data.clients[idx].is_floating = true;
            data.clients[idx].float_geom.x = new_loc.x;
            data.clients[idx].float_geom.y = new_loc.y;
            data.clients[idx].float_geom.width = new_size.w;
            data.clients[idx].float_geom.height = new_size.h;
        }

        let mon = data.focused_monitor();
        if mon < data.monitors.len() {
            data.arrange_monitor(mon);
        }
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
/// released. If the cursor is over another tiled client on the
/// same monitor/tagset, swap the two tiles (mango's
/// drag_tile_to_tile). Otherwise just restore the dragged
/// window's pre-grab floating geometry so the drag_tile_small
/// thumbnail doesn't linger as a 300×300 floater.
fn resolve_drag_tile_drop(
    data: &mut MargoState,
    dragged: &Window,
    original_float_geom: crate::layout::Rect,
) {
    let Some(src) = data
        .clients
        .iter()
        .position(|c| &c.window == dragged)
    else {
        return;
    };

    // Always restore the pre-grab float_geom so the next time the
    // user un-tiles this window it falls back to its real size,
    // not the 300×300 thumbnail.
    data.clients[src].float_geom = original_float_geom;
    data.clients[src].is_floating = false;

    let cursor = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
        data.input_pointer.x,
        data.input_pointer.y,
    ));

    if let Some((target_window, _)) = data.space.element_under(cursor) {
        if let Some(dst) = data
            .clients
            .iter()
            .position(|c| c.window == *target_window)
        {
            if dst != src && !data.clients[dst].is_floating {
                data.clients.swap(src, dst);
            }
        }
    }

    let mon = data.focused_monitor();
    if mon < data.monitors.len() {
        data.arrange_monitor(mon);
    }
}
