//! Cursor elements: the per-output cursor image (theme, client surface
//! or drag icon) pushed on top of a frame.

use super::*;

/// Build just the cursor sprite render elements for a given output
/// without any of the surrounding scene (no clients, no layers, no
/// borders). Used by the screencast Metadata cursor path: xdp-gnome's
/// CursorMode::Metadata sends the cursor as a sidecar bitmap to the
/// PipeWire consumer rather than embedding it in the frame, so the
/// consumer can composite the cursor sharply at low cast resolutions.
/// We need the same elements the embedded path would produce, but
/// extracted from the main scene so `CursorData::compute` can wrap
/// them, `add_cursor_metadata` can render them to a side bitmap,
/// and the main render runs without them.
///
/// Returns `(elements, cursor_logical_loc)`. Empty vec when the
/// pointer is off this output, hidden, or a non-renderable image.
pub fn build_cursor_elements_for_output(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> (Vec<MargoRenderElement>, Point<f64, Logical>) {
    let output_scale = od.output.current_scale().fractional_scale();
    let Some(output_geo) = state.space.output_geometry(&od.output) else {
        return (Vec::new(), Point::default());
    };
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if state.input_timers.cursor_hidden || !output_geo.to_f64().contains(ptr_global) {
        return (Vec::new(), Point::default());
    }
    let ptr_pos = ptr_global - output_geo.loc.to_f64();
    let mut elements = Vec::new();
    match &state.cursor_status {
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .and_then(|attrs| attrs.lock().ok().map(|attrs| attrs.hotspot))
                    .unwrap_or_default()
            });
            let ptr_i =
                (ptr_pos - hotspot.to_f64()).to_physical_precise_round::<f64, i32>(output_scale);
            let cursor_elems = render_elements_from_surface_tree(
                renderer,
                surface,
                ptr_i,
                output_scale,
                1.0f32,
                Kind::Cursor,
            );
            for e in cursor_elems {
                elements.push(MargoRenderElement::WaylandSurface(e));
            }
        }
        CursorImageStatus::Hidden => {}
        _ => {
            if let Some(cursor_elem) =
                state
                    .cursor_manager
                    .render_element(renderer, ptr_pos, output_scale)
            {
                elements.push(MargoRenderElement::Cursor(cursor_elem));
            }
        }
    }
    (elements, ptr_pos)
}

/// The pointer sprite for this output (if the pointer is on it).
pub(super) fn push_cursor_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if state.input_timers.cursor_hidden || !output_geo.to_f64().contains(ptr_global) {
        return;
    }
    let ptr_pos = ptr_global - output_geo.loc.to_f64();
    match &state.cursor_status {
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .and_then(|attrs| attrs.lock().ok().map(|attrs| attrs.hotspot))
                    .unwrap_or_default()
            });
            let ptr_i =
                (ptr_pos - hotspot.to_f64()).to_physical_precise_round::<f64, i32>(output_scale);
            let cursor_elems = render_elements_from_surface_tree(
                renderer,
                surface,
                ptr_i,
                output_scale,
                1.0f32,
                Kind::Cursor,
            );
            for e in cursor_elems {
                elements.push(MargoRenderElement::WaylandSurface(e));
            }
        }
        CursorImageStatus::Hidden => {}
        _ => {
            if let Some(cursor_elem) =
                state
                    .cursor_manager
                    .render_element(renderer, ptr_pos, output_scale)
            {
                elements.push(MargoRenderElement::Cursor(cursor_elem));
            }
        }
    }
}
