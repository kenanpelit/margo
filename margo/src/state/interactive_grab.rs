//! Compositor-initiated move / resize grabs (`moveresize,curmove` and
//! `moveresize,curresize`). The grab types themselves live in
//! `input::grabs`.

use super::*;

impl MargoState {
    /// Start an interactive move grab on the currently focused window.
    /// Triggered by the `moveresize,curmove` action (typically a super+
    /// left-drag mousebind). No-op if there's no focused client or no
    /// pointer button is currently pressed.
    pub fn start_interactive_move(&mut self) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let window = self.clients[idx].window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            self.clients[idx].geom.x,
            self.clients[idx].geom.y,
        ));
        // Remember the pre-grab tiled state so the grab's drop
        // handler can decide between "swap with target tile" and
        // "restore to original float geometry". A client governed by
        // `Mosaic` counts too (it's always `is_floating`, but dropping it
        // on another Mosaic client should still swap their packing order)
        // — see `resolve_drag_tile_drop`.
        let was_tiled = !self.clients[idx].is_floating
            || self.clients[idx].auto_float_owner == Some(crate::layout::LayoutId::Mosaic);
        let original_float_geom = self.clients[idx].float_geom;
        self.clients[idx].interactive_grab = true;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // Use the most recent serial we've seen — we're driving the grab
        // ourselves from a synthesized command, so just take the next one.
        let serial = SERIAL_COUNTER.next_serial();
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x110, // BTN_LEFT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::MoveSurfaceGrab {
            start_data,
            window,
            initial_loc,
            was_tiled,
            original_float_geom,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Start an interactive resize grab on the focused window. Edge
    /// defaults to bottom-right (the natural drag-corner gesture). If
    /// you want a specific edge, pass it in the action arg later.
    pub fn start_interactive_resize(&mut self) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc =
            smithay::utils::Point::<i32, smithay::utils::Logical>::from((c.geom.x, c.geom.y));
        let initial_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            c.geom.width.max(1),
            c.geom.height.max(1),
        ));
        let (edges, corner) = crate::input::grabs::resize_corner(
            self.config.drag_corner,
            (self.input_pointer.x, self.input_pointer.y),
            c.geom,
        );
        self.clients[idx].interactive_grab = true;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        // `drag_warp_cursor`: start the drag with the pointer on the
        // grabbed corner, so the corner tracks the cursor exactly.
        if self.config.drag_warp_cursor {
            let old = smithay::utils::Point::from((self.input_pointer.x, self.input_pointer.y));
            self.input_pointer.x = corner.x;
            self.input_pointer.y = corner.y;
            self.clamp_pointer_to_outputs();
            pointer.motion(
                self,
                None,
                &smithay::input::pointer::MotionEvent {
                    location: smithay::utils::Point::from((
                        self.input_pointer.x,
                        self.input_pointer.y,
                    )),
                    serial,
                    time: smithay::backend::input::InputTime::from_millis(crate::utils::now_ms()),
                },
            );
            pointer.frame(self);
            for output in self
                .space
                .output_under(old)
                .chain(self.space.output_under(corner))
                .cloned()
                .collect::<Vec<_>>()
            {
                self.request_repaint_output(&output);
            }
        }
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x111, // BTN_RIGHT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::ResizeSurfaceGrab {
            start_data,
            window,
            edges,
            initial_loc,
            initial_size,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }
}
