//! Hit testing: which surface (window, layer, lock surface) is under a
//! point, for pointer focus and click-to-focus.

use smithay::{
    desktop::{WindowSurfaceType, layer_map_for_output},
    utils::{Logical, Point},
    wayland::shell::wlr_layer::Layer as WlrLayer,
};

use crate::state::{FocusTarget, MargoState};

pub(crate) type SurfaceHit = (
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
);

pub(crate) type FocusHit = (FocusTarget, Point<f64, Logical>);

/// Keyboard-level target (toplevel / layer / lock surface) and the
/// drilled pointer surface under `pos`, from a single walk of the layer
/// maps and the space. Pointer motion needs both — one for sloppy focus,
/// one for `pointer.motion` — and used to hit-test twice per event.
pub(crate) fn hit_test(
    state: &MargoState,
    pos: Point<f64, Logical>,
) -> (Option<FocusHit>, Option<SurfaceHit>) {
    if state.session_locked {
        return (lock_focus_under(state, pos), lock_pointer_under(state, pos));
    }

    if let Some((layer, focus_loc, surface)) =
        layer_hit(state, pos, &[WlrLayer::Overlay, WlrLayer::Top])
    {
        return (
            Some((FocusTarget::LayerSurface(layer), focus_loc)),
            Some(surface),
        );
    }

    let mut focus = None;
    if let Some((window, win_loc)) = state.space.element_under(pos) {
        focus = Some((FocusTarget::Window(window.clone()), win_loc.to_f64()));
        let local = pos - win_loc.to_f64();
        if let Some((surface, surf_loc)) = window.surface_under(local, WindowSurfaceType::ALL) {
            // surf_loc is window-local; add the window origin for the
            // space coordinates smithay's pointer.motion expects.
            return (focus, Some((surface, (surf_loc + win_loc).to_f64())));
        }
    }

    match layer_hit(state, pos, &[WlrLayer::Bottom, WlrLayer::Background]) {
        Some((layer, focus_loc, surface)) => (
            focus.or(Some((FocusTarget::LayerSurface(layer), focus_loc))),
            Some(surface),
        ),
        None => (focus, None),
    }
}

pub(super) fn focus_under(state: &MargoState, pos: Point<f64, Logical>) -> Option<FocusHit> {
    if state.session_locked {
        return lock_focus_under(state, pos);
    }
    layer_hit(state, pos, &[WlrLayer::Overlay, WlrLayer::Top])
        .map(|(layer, loc, _)| (FocusTarget::LayerSurface(layer), loc))
        .or_else(|| {
            state
                .space
                .element_under(pos)
                .map(|(w, p)| (FocusTarget::Window(w.clone()), p.to_f64()))
        })
        .or_else(|| {
            layer_hit(state, pos, &[WlrLayer::Bottom, WlrLayer::Background])
                .map(|(layer, loc, _)| (FocusTarget::LayerSurface(layer), loc))
        })
}

/// Pointer-specific focus lookup: the actual `WlSurface` under the
/// pointer (drilled through subsurfaces and popups), with that surface's
/// origin in space — so a CSD GTK dialog with several subsurfaces routes
/// to the right child instead of always the toplevel.
pub(super) fn pointer_focus_under(
    state: &MargoState,
    pos: Point<f64, Logical>,
) -> Option<SurfaceHit> {
    hit_test(state, pos).1
}

impl MargoState {
    /// The drilled pointer surface under `pos` (see `pointer_focus_under`).
    pub(crate) fn pointer_surface_under(&self, pos: Point<f64, Logical>) -> Option<SurfaceHit> {
        pointer_focus_under(self, pos)
    }
}

fn lock_focus_under(state: &MargoState, pos: Point<f64, Logical>) -> Option<FocusHit> {
    let output = state.space.output_under(pos).next()?;
    let (_, surface) = state
        .lock_surfaces
        .iter()
        .find(|(o, surface)| o == output && MargoState::lock_surface_ready(surface))?;
    // `?` rather than unwrap: a panic in the input loop while locked
    // strands the user.
    let output_geo = state.space.output_geometry(output)?;
    let local = pos - output_geo.loc.to_f64();
    Some((FocusTarget::SessionLock(surface.clone()), local))
}

/// While the session is locked EVERY pointer event goes to the lock
/// surface of the output under the cursor — anything else would route
/// input to hidden background apps and defeat the lock.
fn lock_pointer_under(state: &MargoState, pos: Point<f64, Logical>) -> Option<SurfaceHit> {
    let output = state.space.output_under(pos).next()?;
    let (_, lock_surface) = state
        .lock_surfaces
        .iter()
        .find(|(o, surface)| o == output && MargoState::lock_surface_ready(surface))?;
    let output_geo = state.space.output_geometry(output)?;
    Some((lock_surface.wl_surface().clone(), output_geo.loc.to_f64()))
}

/// Topmost input-accepting layer surface of `layer_kinds` under `pos`:
/// the layer (focus target), its origin in space, and the drilled
/// surface hit with that surface's origin in space.
fn layer_hit(
    state: &MargoState,
    pos: Point<f64, Logical>,
    layer_kinds: &[WlrLayer],
) -> Option<(
    smithay::wayland::shell::wlr_layer::LayerSurface,
    Point<f64, Logical>,
    SurfaceHit,
)> {
    let output = state.space.output_under(pos).next()?;
    let output_geo = state.space.output_geometry(output)?;
    let output_pos = pos - output_geo.loc.to_f64();
    let layers = layer_map_for_output(output);

    for layer_kind in layer_kinds {
        for layer in layers.layers_on(*layer_kind).rev() {
            if !state.layer_accepts_input_on_output(output, layer.layer()) {
                continue;
            }
            let Some(layer_geo) = layers.layer_geometry(layer) else {
                continue;
            };
            let layer_local = output_pos - layer_geo.loc.to_f64();
            if let Some((surface, surf_loc)) =
                layer.surface_under(layer_local, WindowSurfaceType::ALL)
            {
                let focus_loc = (output_geo.loc + layer_geo.loc).to_f64();
                let space_loc = (surf_loc + layer_geo.loc + output_geo.loc).to_f64();
                return Some((
                    layer.layer_surface().clone(),
                    focus_loc,
                    (surface, space_loc),
                ));
            }
        }
    }
    None
}
