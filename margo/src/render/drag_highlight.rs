//! Phase 2 drag-tile-to-tile drop-target preview.
//!
//! While a tiled or `Mosaic`-governed client is being dragged onto
//! another (see `input::grabs::find_drag_tile_target`), the current
//! valid swap target gets a translucent highlight in the theme's accent
//! colour so the user can see exactly where a drop will register.
//!
//! This didn't used to be predictable: the old target lookup went
//! through `space.element_under(cursor)`, and with `drag_tile_small` (on
//! by default) shrinking the dragged window to a 300×300 thumbnail
//! centred exactly on the cursor, that thumbnail could itself be
//! topmost-at-the-cursor and silently swallow the hit test depending on
//! z-order/creation order — see `resolve_drag_tile_drop`'s fix. The
//! highlight and the actual swap now both go through the same
//! `find_drag_tile_target`, so what lights up is always exactly what a
//! drop would hit.

use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::GlesPixelProgram;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use crate::render::rounded_solid::RoundedSolidElement;

/// Build the highlight element for `target_geom` (the swap target's
/// current geometry, global-logical coordinates). `accent` is
/// straight RGBA in `0.0..=1.0`; its alpha is overridden so the target's
/// own content stays legible underneath the fill.
pub fn render_element(
    target_geom: crate::layout::Rect,
    output_origin: Point<i32, Logical>,
    output_scale: f64,
    radius: f32,
    accent: [f32; 4],
    program: GlesPixelProgram,
) -> RoundedSolidElement {
    let loc = Point::<i32, Physical>::from((
        (((target_geom.x - output_origin.x) as f64) * output_scale).round() as i32,
        (((target_geom.y - output_origin.y) as f64) * output_scale).round() as i32,
    ));
    let size = Size::<i32, Physical>::from((
        ((target_geom.width as f64) * output_scale).round() as i32,
        ((target_geom.height as f64) * output_scale).round() as i32,
    ));
    let mut fill = accent;
    fill[3] = 0.35;
    RoundedSolidElement::new(Id::new(), Rectangle::new(loc, size), radius, fill, program)
}
