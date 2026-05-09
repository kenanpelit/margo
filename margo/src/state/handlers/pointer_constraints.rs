//! `pointer-constraints-v1` + `relative-pointer-v1` handlers.
//!
//! Constraint *enforcement* (lock the cursor, clamp to region) lives
//! in `input_handler::handle_pointer_motion`; this file only wires
//! the protocol surface (creation + post-unlock cursor-position
//! hint resolution).

use smithay::{
    delegate_pointer_constraints, delegate_relative_pointer,
    input::pointer::PointerHandle,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
    wayland::{
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler},
        seat::WaylandFocus,
    },
};

use crate::state::MargoState;

impl PointerConstraintsHandler for MargoState {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // Activate immediately if the pointer is already over the
        // requesting surface (the common path: fullscreen games,
        // Blender drags request a constraint while focused). If
        // the pointer is elsewhere, smithay defers activation until
        // pointer focus moves in.
        let Some(current_focus) = pointer.current_focus() else {
            return;
        };
        if current_focus.wl_surface().as_deref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        // While a lock is active, the client may suggest a
        // post-unlock cursor position (e.g. "the crosshair was at
        // (320, 200) when I locked, please put the cursor there
        // when unlocking"). Only honour it if the constraint is
        // currently active and the surface still owns the pointer.
        let active = with_pointer_constraint(surface, pointer, |constraint| {
            constraint.is_some_and(|c| c.is_active())
        });
        if !active {
            return;
        }
        // Resolve the surface's screen origin so we can convert the
        // surface-relative `location` hint to compositor-global
        // coordinates.
        let origin = self
            .space
            .elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(surface)).then(|| {
                    self.space.element_location(window).unwrap_or_default()
                })
            })
            .unwrap_or_default()
            .to_f64();
        let target = origin + location;
        pointer.set_location(target);
        self.input_pointer.x = target.x;
        self.input_pointer.y = target.y;
    }
}
delegate_pointer_constraints!(MargoState);
delegate_relative_pointer!(MargoState);
