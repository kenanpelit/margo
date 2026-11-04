use mshell_common::css_vars::CssVars;
use relm4::gtk::{
    self,
    cairo::{Context, LineCap, LineJoin, Operator, RectangleInt, Region},
    glib::{self, object_subclass},
    prelude::*,
    subclass::prelude::*,
};
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct FrameStyle {
    pub draw_frame: bool,
    pub border_radius: f64,
    pub background_rgba: (f64, f64, f64, f64),
    pub border_rgba: (f64, f64, f64, f64),
    pub border_width: f64,
    pub left_thickness: f64,
    pub right_thickness: f64,
    pub top_thickness: f64,
    pub bottom_thickness: f64,
    pub left_top_expander_height: f64,
    pub right_top_expander_height: f64,
    pub right_bottom_expander_height: f64,
    pub left_bottom_expander_height: f64,
    pub left_expander_width: f64,
    pub right_expander_width: f64,
    pub top_revealer_size: (f64, f64),
    pub bottom_revealer_size: (f64, f64),
    pub top_left_revealer_size: (f64, f64),
    pub top_right_revealer_size: (f64, f64),
    pub bottom_left_revealer_size: (f64, f64),
    pub bottom_right_revealer_size: (f64, f64),
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self {
            draw_frame: true,
            border_radius: 24.0,
            background_rgba: (0.0, 0.0, 0.0, 1.0),
            border_rgba: (1.0, 1.0, 1.0, 1.0),
            border_width: 2.0,
            left_thickness: 50.0,
            right_thickness: 50.0,
            top_thickness: 50.0,
            bottom_thickness: 50.0,
            left_top_expander_height: 0.0,
            right_top_expander_height: 0.0,
            left_bottom_expander_height: 0.0,
            right_bottom_expander_height: 0.0,
            left_expander_width: 0.0,
            right_expander_width: 0.0,
            top_revealer_size: (0.0, 0.0),
            bottom_revealer_size: (0.0, 0.0),
            top_left_revealer_size: (0.0, 0.0),
            top_right_revealer_size: (0.0, 0.0),
            bottom_left_revealer_size: (0.0, 0.0),
            bottom_right_revealer_size: (0.0, 0.0),
        }
    }
}

/// An axis-aligned rectangle in output (logical pixel) coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// Which part of the output-wide frame one surface draws.
///
/// The frame is one shape laid out on the whole output (bands, rounded hole,
/// menu notches). It used to be painted by a single full-output surface. Now
/// several surfaces (the top/bottom/side edge strips and the menu overlay)
/// each paint the *same* output-space shape, translated by where the surface
/// sits and clipped to the rectangles it owns. As long as the owned rectangles
/// do not overlap and sit on whole pixels, the pieces composite to exactly the
/// pixels the single surface painted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Viewport {
    /// Where this widget's (0, 0) sits on the output.
    pub origin: (f64, f64),
    /// Output size the frame shape is laid out on. `None` = this widget's own
    /// size (the widget covers the whole output).
    pub output: Option<(f64, f64)>,
    /// Output-space rectangles this surface owns: it paints and takes pointer
    /// input only there. `None` = everything.
    pub owned: Option<Vec<Rect>>,
}

// ---------------------------------------------------------------------------
// CSS custom-property helpers
// ---------------------------------------------------------------------------

fn parse_number(s: &str) -> Option<f64> {
    s.trim().trim_end_matches("px").parse().ok()
}

fn apply_css_vars(style: &mut FrameStyle, vars: &CssVars) {
    if let Some(c) = vars.color("--frame-bg") {
        style.background_rgba = c;
    }

    // Shell-surface opacity (Settings → Appearance). A percentage that scales
    // the painted frame fill so the wallpaper shows through (ashell-style);
    // the border stays at full alpha for definition. Clamped 60–100 %.
    if let Some(pct) = vars.get("--surface-opacity")
        && let Ok(p) = pct.trim().trim_end_matches('%').parse::<f64>()
    {
        style.background_rgba.3 *= (p / 100.0).clamp(0.6, 1.0);
    }

    if let Some(c) = vars.color("--frame-border") {
        style.border_rgba = c;
    }

    if let Some(n) = vars.get("--frame-border-width").and_then(parse_number) {
        style.border_width = n;
    }

    if let Some(n) = vars.get("--frame-border-radius").and_then(parse_number) {
        style.border_radius = n;
    }
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Polygon-based hole path algorithm
// ---------------------------------------------------------------------------
//
// The hole is described as a polygon of (x, y) vertices in clockwise order.
// `build_hole_vertices` collects all the vertices based on which features
// are active.  `trace_rounded_polygon` then traces the path with rounded
// corners, automatically determining convex vs concave at each vertex.
//
// Understanding the layout:
//
//   left_thickness includes the left bar + left revealer (menu) width.
//   right_thickness includes the right bar + right revealer width.
//   top_thickness / bottom_thickness include the top/bottom bar heights.
//
//   The inner hole rectangle is:
//     hole_x = left_thickness,  hole_y = top_thickness
//     hole_width  = total_w - left_thickness - right_thickness
//     hole_height = total_h - top_thickness - bottom_thickness
//
//   left_expander_width / right_expander_width: the width of the side
//   menus (revealers).  These are already accounted for in left/right
//   thickness.  The expander HEIGHTS represent empty space (no menu
//   content) above/below the menu within that revealer area.  These
//   empty regions are "negative space" — the hole extends leftward
//   into them.
//
//   So the left side has two notches extending LEFT from the hole edge:
//     Top-left notch:  from hole_y to hole_y + left_top_expander_height,
//                      width = left_expander_width, going left from hole_x
//     Bottom-left notch: from hole_y + hole_height - left_bottom_expander_height
//                         to hole_y + hole_height,
//                         width = left_expander_width, going left from hole_x
//
//   Same pattern mirrored for the right side (extending right from hole_x + hole_width).
//
//   Revealers (top, bottom, top_left, top_right, bottom_left, bottom_right)
//   create inward notches that poke INTO the hole from the top/bottom edges.
//   The frame is painted over these areas.  Corner revealers are anchored
//   flush with the hole's left/right edge.

/// Build the hole polygon vertices in clockwise order.
#[allow(clippy::too_many_arguments)]
fn build_hole_vertices(
    style: &FrameStyle,
    hole_x: f64,
    hole_y: f64,
    hole_width: f64,
    hole_height: f64,
) -> Vec<(f64, f64)> {
    let mut vertices: Vec<(f64, f64)> = Vec::with_capacity(64);

    let left_expander_width = style.left_expander_width;
    let right_expander_width = style.right_expander_width;
    let left_top_expander_height = style.left_top_expander_height;
    let left_bottom_expander_height = style.left_bottom_expander_height;
    let right_top_expander_height = style.right_top_expander_height;
    let right_bottom_expander_height = style.right_bottom_expander_height;

    let (top_revealer_width, top_revealer_height) = style.top_revealer_size;
    let (bottom_revealer_width, bottom_revealer_height) = style.bottom_revealer_size;
    let (top_left_revealer_width, top_left_revealer_height) = style.top_left_revealer_size;
    let (top_right_revealer_width, top_right_revealer_height) = style.top_right_revealer_size;
    let (bottom_left_revealer_width, bottom_left_revealer_height) = style.bottom_left_revealer_size;
    let (bottom_right_revealer_width, bottom_right_revealer_height) =
        style.bottom_right_revealer_size;

    let has_left_top_expander = left_top_expander_height > 0.0 && left_expander_width > 0.0;
    let has_right_top_expander = right_top_expander_height > 0.0 && right_expander_width > 0.0;
    let has_left_bottom_expander = left_bottom_expander_height > 0.0 && left_expander_width > 0.0;
    let has_right_bottom_expander =
        right_bottom_expander_height > 0.0 && right_expander_width > 0.0;

    let has_top_left_revealer = top_left_revealer_width > 0.0 && top_left_revealer_height > 0.0;
    let has_top_right_revealer = top_right_revealer_width > 0.0 && top_right_revealer_height > 0.0;
    let has_bottom_left_revealer =
        bottom_left_revealer_width > 0.0 && bottom_left_revealer_height > 0.0;
    let has_bottom_right_revealer =
        bottom_right_revealer_width > 0.0 && bottom_right_revealer_height > 0.0;
    let has_top_revealer = top_revealer_width > 0.0 && top_revealer_height > 0.0;
    let has_bottom_revealer = bottom_revealer_width > 0.0 && bottom_revealer_height > 0.0;

    let hole_right = hole_x + hole_width;
    let hole_bottom = hole_y + hole_height;

    // =====================================================================
    // Walk clockwise starting from the top-left corner (hole_x, hole_y).
    //
    // The path traces the outline of everything that should be "hole"
    // (transparent / cleared).  Expander notches are detours that
    // extend LEFT/RIGHT from the main hole edge and return.  Revealer
    // notches are detours that extend DOWN/UP into the hole from the
    // top/bottom edges.
    //
    // The walk always starts and ends on the main hole edge so that
    // close_path connects cleanly.
    // =====================================================================

    // Track whether corner revealers have already consumed the expander
    // space, so the expander detours on the left/right edges can be skipped.
    let mut left_top_expander_consumed = false;
    let mut left_bottom_expander_consumed = false;
    let mut right_top_expander_consumed = false;
    let right_bottom_expander_consumed = false;

    // === TOP-LEFT CORNER (hole_x, hole_y) ===

    // Top-left revealer: inward notch going down from the top edge.
    //
    // The revealer notch always extends downward from hole_y by
    // top_left_revealer_height.  The question is what happens on its
    // left side, which depends on whether the left menu is revealed
    // (left_expander_width > 0) and whether there is empty space above
    // the left menu content (left_top_expander_height > 0).
    //
    // Cases:
    //   A. left_expander_width == 0: No left menu panel.  Revealer is
    //      flush against the left frame band.  Merges with corner.
    //
    //   B. left_expander_width > 0 and left_top_expander_height == 0:
    //      Left menu is fully expanded (no empty space above it).
    //      There is no gap to the left of the revealer — the menu
    //      content fills that area.  Revealer is a standalone notch
    //      that does NOT extend leftward.
    //
    //   C. left_expander_width > 0 and left_top_expander_height > 0
    //      and revealer height >= expander height:
    //      Joined L-shape.  The hole extends left for the expander's
    //      height, then steps right to hole_x, then continues down
    //      to the revealer's height.  The expander detour is consumed.
    //
    //   D. left_expander_width > 0 and left_top_expander_height > 0
    //      and revealer height < expander height:
    //      The hole extends left for the revealer's height.  The
    //      expander detour on the left edge handles the remaining
    //      gap below.
    if has_top_left_revealer {
        if left_expander_width > 0.0 && left_top_expander_height > 0.0 {
            if top_left_revealer_height >= left_top_expander_height {
                // Case C: joined L-shape — expander consumed
                left_top_expander_consumed = true;
                vertices.push((hole_x - left_expander_width, hole_y));
                vertices.push((
                    hole_x - left_expander_width,
                    hole_y + left_top_expander_height,
                ));
                vertices.push((hole_x, hole_y + left_top_expander_height));
                vertices.push((hole_x, hole_y + top_left_revealer_height));
                vertices.push((
                    hole_x + top_left_revealer_width,
                    hole_y + top_left_revealer_height,
                ));
                vertices.push((hole_x + top_left_revealer_width, hole_y));
            } else {
                // Case D: revealer shorter than expander
                vertices.push((hole_x - left_expander_width, hole_y));
                vertices.push((hole_x, hole_y));
                vertices.push((hole_x, hole_y + top_left_revealer_height));
                vertices.push((
                    hole_x + top_left_revealer_width,
                    hole_y + top_left_revealer_height,
                ));
                vertices.push((hole_x + top_left_revealer_width, hole_y));
            }
        } else if left_expander_width > 0.0 {
            // Case B: left menu visible but fully expanded, no gap
            vertices.push((hole_x, hole_y));
            vertices.push((hole_x, hole_y + top_left_revealer_height));
            vertices.push((
                hole_x + top_left_revealer_width,
                hole_y + top_left_revealer_height,
            ));
            vertices.push((hole_x + top_left_revealer_width, hole_y));
        } else {
            // Case A: no left menu, merged with left edge
            vertices.push((hole_x, hole_y));
            vertices.push((hole_x, hole_y + top_left_revealer_height));
            vertices.push((
                hole_x + top_left_revealer_width,
                hole_y + top_left_revealer_height,
            ));
            vertices.push((hole_x + top_left_revealer_width, hole_y));
        }
    } else {
        vertices.push((hole_x, hole_y));
    }

    // === TOP EDGE (left → right) ===

    // Center top revealer: inward notch centered on the top edge.
    // Its position must account for the corner revealer widths — it is
    // centered in the remaining span between them.
    if has_top_revealer {
        let edge_start = hole_x + top_left_revealer_width;
        let edge_end = hole_right - top_right_revealer_width;
        let edge_center = (edge_start + edge_end) / 2.0;
        let notch_left = edge_center - top_revealer_width / 2.0;
        let notch_right = edge_center + top_revealer_width / 2.0;
        vertices.push((notch_left, hole_y));
        vertices.push((notch_left, hole_y + top_revealer_height));
        vertices.push((notch_right, hole_y + top_revealer_height));
        vertices.push((notch_right, hole_y));
    }

    // Top-right revealer: same four cases, mirrored for the right side.
    if has_top_right_revealer {
        if right_expander_width > 0.0 && right_top_expander_height > 0.0 {
            if top_right_revealer_height >= right_top_expander_height {
                // Case C: joined L-shape — expander consumed
                right_top_expander_consumed = true;
                vertices.push((hole_right - top_right_revealer_width, hole_y));
                vertices.push((
                    hole_right - top_right_revealer_width,
                    hole_y + top_right_revealer_height,
                ));
                vertices.push((hole_right, hole_y + top_right_revealer_height));
                vertices.push((hole_right, hole_y + right_top_expander_height));
                vertices.push((
                    hole_right + right_expander_width,
                    hole_y + right_top_expander_height,
                ));
                vertices.push((hole_right + right_expander_width, hole_y));
            } else {
                // Case D: revealer shorter than expander
                vertices.push((hole_right - top_right_revealer_width, hole_y));
                vertices.push((
                    hole_right - top_right_revealer_width,
                    hole_y + top_right_revealer_height,
                ));
                vertices.push((hole_right, hole_y + top_right_revealer_height));
                vertices.push((hole_right, hole_y));
                vertices.push((hole_right + right_expander_width, hole_y));
            }
        } else if right_expander_width > 0.0 {
            // Case B: right menu visible but fully expanded, no gap
            vertices.push((hole_right - top_right_revealer_width, hole_y));
            vertices.push((
                hole_right - top_right_revealer_width,
                hole_y + top_right_revealer_height,
            ));
            vertices.push((hole_right, hole_y + top_right_revealer_height));
            vertices.push((hole_right, hole_y));
        } else {
            // Case A: no right menu, merged with right edge
            vertices.push((hole_right - top_right_revealer_width, hole_y));
            vertices.push((
                hole_right - top_right_revealer_width,
                hole_y + top_right_revealer_height,
            ));
            vertices.push((hole_right, hole_y + top_right_revealer_height));
            vertices.push((hole_right, hole_y));
        }
    }

    // === TOP-RIGHT CORNER (hole_right, hole_y) ===

    // Right top expander: detour rightward into the frame band, then return.
    // Skipped if the top-right revealer already consumed this space.
    if has_right_top_expander && !right_top_expander_consumed {
        if !has_top_right_revealer {
            vertices.push((hole_right, hole_y));
        }
        vertices.push((hole_right + right_expander_width, hole_y));
        vertices.push((
            hole_right + right_expander_width,
            hole_y + right_top_expander_height,
        ));
        vertices.push((hole_right, hole_y + right_top_expander_height));
    } else if !has_top_right_revealer && !right_top_expander_consumed {
        vertices.push((hole_right, hole_y));
    }

    // === RIGHT EDGE (top → bottom) ===

    // Right bottom expander: detour rightward, then return.
    // Skipped if the bottom-right revealer already consumed this space.
    if has_right_bottom_expander && !right_bottom_expander_consumed {
        vertices.push((hole_right, hole_bottom - right_bottom_expander_height));
        vertices.push((
            hole_right + right_expander_width,
            hole_bottom - right_bottom_expander_height,
        ));
        vertices.push((hole_right + right_expander_width, hole_bottom));
        vertices.push((hole_right, hole_bottom));
    } else {
        vertices.push((hole_right, hole_bottom));
    }

    // === BOTTOM-RIGHT CORNER (hole_right, hole_bottom) ===

    // Bottom-right revealer: same four cases, mirrored for bottom-right.
    if has_bottom_right_revealer {
        if right_expander_width > 0.0 && right_bottom_expander_height > 0.0 {
            if bottom_right_revealer_height >= right_bottom_expander_height {
                // Case C: joined L-shape — expander consumed
                vertices.push((hole_right + right_expander_width, hole_bottom));
                vertices.push((
                    hole_right + right_expander_width,
                    hole_bottom - right_bottom_expander_height,
                ));
                vertices.push((hole_right, hole_bottom - right_bottom_expander_height));
                vertices.push((hole_right, hole_bottom - bottom_right_revealer_height));
                vertices.push((
                    hole_right - bottom_right_revealer_width,
                    hole_bottom - bottom_right_revealer_height,
                ));
                vertices.push((hole_right - bottom_right_revealer_width, hole_bottom));
            } else {
                // Case D: revealer shorter than expander
                vertices.push((hole_right + right_expander_width, hole_bottom));
                vertices.push((hole_right, hole_bottom));
                vertices.push((hole_right, hole_bottom - bottom_right_revealer_height));
                vertices.push((
                    hole_right - bottom_right_revealer_width,
                    hole_bottom - bottom_right_revealer_height,
                ));
                vertices.push((hole_right - bottom_right_revealer_width, hole_bottom));
            }
        } else if right_expander_width > 0.0 {
            // Case B: right menu visible but fully expanded, no gap
            vertices.push((hole_right, hole_bottom - bottom_right_revealer_height));
            vertices.push((
                hole_right - bottom_right_revealer_width,
                hole_bottom - bottom_right_revealer_height,
            ));
            vertices.push((hole_right - bottom_right_revealer_width, hole_bottom));
        } else {
            // Case A: no right menu, merged with right edge
            vertices.push((hole_right, hole_bottom - bottom_right_revealer_height));
            vertices.push((
                hole_right - bottom_right_revealer_width,
                hole_bottom - bottom_right_revealer_height,
            ));
            vertices.push((hole_right - bottom_right_revealer_width, hole_bottom));
        }
    }

    // === BOTTOM EDGE (right → left) ===

    // Center bottom revealer: same offset logic as center top.
    if has_bottom_revealer {
        let edge_start = hole_x + bottom_left_revealer_width;
        let edge_end = hole_right - bottom_right_revealer_width;
        let edge_center = (edge_start + edge_end) / 2.0;
        let notch_left = edge_center - bottom_revealer_width / 2.0;
        let notch_right = edge_center + bottom_revealer_width / 2.0;
        vertices.push((notch_right, hole_bottom));
        vertices.push((notch_right, hole_bottom - bottom_revealer_height));
        vertices.push((notch_left, hole_bottom - bottom_revealer_height));
        vertices.push((notch_left, hole_bottom));
    }

    // Bottom-left revealer: same four cases, mirrored for bottom-left.
    if has_bottom_left_revealer {
        if left_expander_width > 0.0 && left_bottom_expander_height > 0.0 {
            if bottom_left_revealer_height >= left_bottom_expander_height {
                // Case C: joined L-shape — expander consumed
                left_bottom_expander_consumed = true;
                vertices.push((hole_x + bottom_left_revealer_width, hole_bottom));
                vertices.push((
                    hole_x + bottom_left_revealer_width,
                    hole_bottom - bottom_left_revealer_height,
                ));
                vertices.push((hole_x, hole_bottom - bottom_left_revealer_height));
                vertices.push((hole_x, hole_bottom - left_bottom_expander_height));
                vertices.push((
                    hole_x - left_expander_width,
                    hole_bottom - left_bottom_expander_height,
                ));
                vertices.push((hole_x - left_expander_width, hole_bottom));
            } else {
                // Case D: revealer shorter than expander
                vertices.push((hole_x + bottom_left_revealer_width, hole_bottom));
                vertices.push((
                    hole_x + bottom_left_revealer_width,
                    hole_bottom - bottom_left_revealer_height,
                ));
                vertices.push((hole_x, hole_bottom - bottom_left_revealer_height));
                vertices.push((hole_x, hole_bottom));
                vertices.push((hole_x - left_expander_width, hole_bottom));
            }
        } else if left_expander_width > 0.0 {
            // Case B: left menu visible but fully expanded, no gap
            vertices.push((hole_x + bottom_left_revealer_width, hole_bottom));
            vertices.push((
                hole_x + bottom_left_revealer_width,
                hole_bottom - bottom_left_revealer_height,
            ));
            vertices.push((hole_x, hole_bottom - bottom_left_revealer_height));
            vertices.push((hole_x, hole_bottom));
        } else {
            // Case A: no left menu, merged with left edge
            vertices.push((hole_x + bottom_left_revealer_width, hole_bottom));
            vertices.push((
                hole_x + bottom_left_revealer_width,
                hole_bottom - bottom_left_revealer_height,
            ));
            vertices.push((hole_x, hole_bottom - bottom_left_revealer_height));
            vertices.push((hole_x, hole_bottom));
        }
    }

    // === BOTTOM-LEFT CORNER (hole_x, hole_bottom) ===

    // Left bottom expander: detour leftward into the frame band, then return.
    // Skipped if the bottom-left revealer already consumed this space.
    if has_left_bottom_expander && !left_bottom_expander_consumed {
        if !has_bottom_left_revealer {
            vertices.push((hole_x, hole_bottom));
        }
        vertices.push((hole_x - left_expander_width, hole_bottom));
        vertices.push((
            hole_x - left_expander_width,
            hole_bottom - left_bottom_expander_height,
        ));
        vertices.push((hole_x, hole_bottom - left_bottom_expander_height));
    } else if !has_bottom_left_revealer && !left_bottom_expander_consumed {
        vertices.push((hole_x, hole_bottom));
    }

    // === LEFT EDGE (bottom → top) ===

    // Left top expander: detour leftward into the frame band, then return.
    // Skipped if the top-left revealer already consumed this space.
    if has_left_top_expander && !left_top_expander_consumed {
        vertices.push((hole_x, hole_y + left_top_expander_height));
        vertices.push((
            hole_x - left_expander_width,
            hole_y + left_top_expander_height,
        ));
        vertices.push((hole_x - left_expander_width, hole_y));
        vertices.push((hole_x, hole_y));
    }

    // Path closes back to the first vertex at (hole_x, hole_y).

    vertices
}

/// Determine the cross product z-component at vertex `current`.
/// Positive = left turn (convex in CW winding).
/// Negative = right turn (concave in CW winding).
fn cross_z(previous: (f64, f64), current: (f64, f64), next: (f64, f64)) -> f64 {
    let incoming = (current.0 - previous.0, current.1 - previous.1);
    let outgoing = (next.0 - current.0, next.1 - current.1);
    incoming.0 * outgoing.1 - incoming.1 * outgoing.0
}

/// Trace the polygon with rounded corners.
///
/// At each vertex the turn direction is computed via cross product:
///   - CW convex (cross > 0): standard `arc`
///   - CW concave (cross < 0): `arc_negative`
///   - Collinear: skip
///
/// Radii are clamped so adjacent corners don't overlap on shared edges.
fn trace_rounded_polygon(cr: &Context, vertices: &[(f64, f64)], max_radius: f64) {
    let vertex_count = vertices.len();
    if vertex_count < 3 {
        return;
    }

    // Remove collinear points (where the path continues in the same direction)
    let mut cleaned: Vec<(f64, f64)> = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let previous = vertices[(i + vertex_count - 1) % vertex_count];
        let current = vertices[i];
        let next = vertices[(i + 1) % vertex_count];
        if cross_z(previous, current, next).abs() > 1e-6 {
            cleaned.push(current);
        }
    }

    let vertex_count = cleaned.len();
    if vertex_count < 3 {
        return;
    }

    // Compute radius for each corner, clamped by adjacent edge lengths
    let mut radii: Vec<f64> = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let previous = cleaned[(i + vertex_count - 1) % vertex_count];
        let current = cleaned[i];
        let next = cleaned[(i + 1) % vertex_count];

        let incoming_length =
            ((current.0 - previous.0).powi(2) + (current.1 - previous.1).powi(2)).sqrt();
        let outgoing_length = ((next.0 - current.0).powi(2) + (next.1 - current.1).powi(2)).sqrt();

        radii.push(
            max_radius
                .min(incoming_length / 2.0)
                .min(outgoing_length / 2.0),
        );
    }

    // Ensure adjacent corners don't overlap on shared edges
    for i in 0..vertex_count {
        let next_index = (i + 1) % vertex_count;
        let edge_length = {
            let (x0, y0) = cleaned[i];
            let (x1, y1) = cleaned[next_index];
            ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt()
        };
        let combined_radii = radii[i] + radii[next_index];
        if combined_radii > edge_length && combined_radii > 1e-6 {
            let scale = edge_length / combined_radii;
            radii[i] *= scale;
            radii[next_index] *= scale;
        }
    }

    cr.new_path();

    for i in 0..vertex_count {
        let previous = cleaned[(i + vertex_count - 1) % vertex_count];
        let current = cleaned[i];
        let next = cleaned[(i + 1) % vertex_count];
        let radius = radii[i];

        if radius < 1e-6 {
            if i == 0 {
                cr.move_to(current.0, current.1);
            } else {
                cr.line_to(current.0, current.1);
            }
            continue;
        }

        let incoming_length =
            ((current.0 - previous.0).powi(2) + (current.1 - previous.1).powi(2)).sqrt();
        let outgoing_length = ((next.0 - current.0).powi(2) + (next.1 - current.1).powi(2)).sqrt();

        if incoming_length < 1e-9 || outgoing_length < 1e-9 {
            if i == 0 {
                cr.move_to(current.0, current.1);
            } else {
                cr.line_to(current.0, current.1);
            }
            continue;
        }

        let incoming_dir_x = (current.0 - previous.0) / incoming_length;
        let incoming_dir_y = (current.1 - previous.1) / incoming_length;
        let outgoing_dir_x = (next.0 - current.0) / outgoing_length;
        let outgoing_dir_y = (next.1 - current.1) / outgoing_length;

        // Tangent points where arc meets edges
        let tangent_incoming = (
            current.0 - incoming_dir_x * radius,
            current.1 - incoming_dir_y * radius,
        );
        let tangent_outgoing = (
            current.0 + outgoing_dir_x * radius,
            current.1 + outgoing_dir_y * radius,
        );

        let cross = incoming_dir_x * outgoing_dir_y - incoming_dir_y * outgoing_dir_x;

        // Normal pointing toward arc center
        let (normal_x, normal_y) = if cross > 0.0 {
            (-incoming_dir_y, incoming_dir_x)
        } else {
            (incoming_dir_y, -incoming_dir_x)
        };

        let arc_center = (
            tangent_incoming.0 + normal_x * radius,
            tangent_incoming.1 + normal_y * radius,
        );

        let angle_start =
            (tangent_incoming.1 - arc_center.1).atan2(tangent_incoming.0 - arc_center.0);
        let angle_end =
            (tangent_outgoing.1 - arc_center.1).atan2(tangent_outgoing.0 - arc_center.0);

        if i == 0 {
            cr.move_to(tangent_incoming.0, tangent_incoming.1);
        } else {
            cr.line_to(tangent_incoming.0, tangent_incoming.1);
        }

        if cross > 0.0 {
            cr.arc(arc_center.0, arc_center.1, radius, angle_start, angle_end);
        } else {
            cr.arc_negative(arc_center.0, arc_center.1, radius, angle_start, angle_end);
        }
    }

    cr.close_path();
}

/// The hole rectangle and its clamped corner radius for an output of
/// `total_width` x `total_height`.
pub(crate) fn hole_geometry(
    style: &FrameStyle,
    total_width: f64,
    total_height: f64,
) -> (f64, f64, f64, f64, f64) {
    let hole_x = style.left_thickness;
    let hole_y = style.top_thickness;
    let hole_width = (total_width - style.left_thickness - style.right_thickness).max(0.0);
    let hole_height = (total_height - style.top_thickness - style.bottom_thickness).max(0.0);
    let border_radius = style
        .border_radius
        .clamp(0.0, hole_width.min(hole_height) / 2.0);
    (hole_x, hole_y, hole_width, hole_height, border_radius)
}

/// Paint the frame for an output of `total_width` x `total_height` into `cr`,
/// whose origin is the widget's (0, 0). `viewport` translates the output-space
/// shape into widget space and clips it to the owned rectangles.
pub(crate) fn paint_frame(
    cr: &Context,
    style: &FrameStyle,
    total_width: f64,
    total_height: f64,
    viewport: &Viewport,
) {
    if !style.draw_frame || total_width <= 0.0 || total_height <= 0.0 {
        return;
    }
    let (hole_x, hole_y, hole_width, hole_height, border_radius) =
        hole_geometry(style, total_width, total_height);
    let vertices = build_hole_vertices(style, hole_x, hole_y, hole_width, hole_height);
    if vertices.is_empty() {
        return;
    }

    let _ = cr.save();
    let (origin_x, origin_y) = viewport.origin;
    cr.translate(-origin_x, -origin_y);
    if let Some(owned) = &viewport.owned {
        for rect in owned.iter().filter(|r| !r.is_empty()) {
            cr.rectangle(rect.x, rect.y, rect.w, rect.h);
        }
        cr.clip();
    }

    // 1) Paint full background
    cr.set_operator(Operator::Over);
    let (bg_r, bg_g, bg_b, bg_a) = style.background_rgba;
    cr.set_source_rgba(bg_r, bg_g, bg_b, bg_a);
    cr.rectangle(0.0, 0.0, total_width, total_height);
    let _ = cr.fill();

    // 2) Clear the hole
    cr.set_operator(Operator::Clear);
    trace_rounded_polygon(cr, &vertices, border_radius);
    let _ = cr.fill();

    // 3) Border, stroked inside the hole (clipped to it).
    let (border_r, border_g, border_b, border_a) = style.border_rgba;
    if style.border_width > 0.0 && border_a > 0.0 {
        let _ = cr.save();
        cr.set_operator(Operator::Over);
        trace_rounded_polygon(cr, &vertices, border_radius);
        cr.clip();
        trace_rounded_polygon(cr, &vertices, border_radius);
        cr.set_source_rgba(border_r, border_g, border_b, border_a);
        cr.set_line_width(style.border_width * 2.0);
        cr.set_line_join(LineJoin::Round);
        cr.set_line_cap(LineCap::Round);
        let _ = cr.stroke();
        let _ = cr.restore();
    }
    let _ = cr.restore();
}

fn subtract_rect(region: &Region, x: i32, y: i32, width: i32, height: i32) {
    if width > 0 && height > 0 {
        let _ = region.subtract_rectangle(&RectangleInt::new(x, y, width, height));
    }
}

fn union_inward_notch(region: &Region, x: f64, y: f64, width: f64, height: f64) {
    if width > 0.0 && height > 0.0 {
        let _ = region.union_rectangle(&RectangleInt::new(
            x.floor() as i32,
            y.floor() as i32,
            width.ceil() as i32,
            height.ceil() as i32,
        ));
    }
}

/// The frame's pointer-input region in output space: the bands, a
/// `border_width` ring inside the hole, and the menu notches. Everything else
/// (the hole) passes clicks through to the windows below.
pub(crate) fn output_input_region(
    style: &FrameStyle,
    total_width: f64,
    total_height: f64,
) -> Region {
    let (hole_x, hole_y, hole_width, hole_height, _) =
        hole_geometry(style, total_width, total_height);
    let window_width = total_width as i32;
    let window_height = total_height as i32;

    let hole_left = hole_x.floor().max(0.0) as i32;
    let hole_top = hole_y.floor().max(0.0) as i32;
    let hole_right = (hole_x + hole_width).ceil().min(window_width as f64) as i32;
    let hole_bottom = (hole_y + hole_height).ceil().min(window_height as f64) as i32;

    let region = Region::create();
    let _ = region.union_rectangle(&RectangleInt::new(0, 0, window_width, window_height));

    let border_width = style.border_width.ceil() as i32;
    let border_width_f = style.border_width;

    // Subtract main hole (inset by border_width so the border stays in the input region)
    subtract_rect(
        &region,
        hole_left + border_width,
        hole_top + border_width,
        (hole_right - hole_left - border_width * 2).max(0),
        (hole_bottom - hole_top - border_width * 2).max(0),
    );

    let left_expander_width = style.left_expander_width.ceil() as i32;
    let right_expander_width = style.right_expander_width.ceil() as i32;

    // Subtract side expander negative-space areas (inset by border_width)
    // Left top expander: shrink width from the right (hole edge) by border_width
    subtract_rect(
        &region,
        hole_left - left_expander_width,
        hole_top,
        (left_expander_width - border_width).max(0),
        style.left_top_expander_height.ceil() as i32,
    );
    // Left bottom expander
    subtract_rect(
        &region,
        hole_left - left_expander_width,
        hole_bottom - style.left_bottom_expander_height.ceil() as i32,
        (left_expander_width - border_width).max(0),
        style.left_bottom_expander_height.ceil() as i32,
    );
    // Right top expander: shift x rightward by border_width, shrink width
    subtract_rect(
        &region,
        hole_right + border_width,
        hole_top,
        (right_expander_width - border_width).max(0),
        style.right_top_expander_height.ceil() as i32,
    );
    // Right bottom expander
    subtract_rect(
        &region,
        hole_right + border_width,
        hole_bottom - style.right_bottom_expander_height.ceil() as i32,
        (right_expander_width - border_width).max(0),
        style.right_bottom_expander_height.ceil() as i32,
    );

    // Union back inward revealer notches so frame captures input there.
    // Expand each notch by border_width on sides that touch the hole edge
    // so the border strip is also covered.
    let (top_revealer_width, top_revealer_height) = style.top_revealer_size;
    let (bottom_revealer_width, bottom_revealer_height) = style.bottom_revealer_size;
    let (top_left_revealer_width, top_left_revealer_height) = style.top_left_revealer_size;
    let (top_right_revealer_width, top_right_revealer_height) = style.top_right_revealer_size;
    let (bottom_left_revealer_width, bottom_left_revealer_height) = style.bottom_left_revealer_size;
    let (bottom_right_revealer_width, bottom_right_revealer_height) =
        style.bottom_right_revealer_size;

    // Center top revealer: touches top hole edge, expand upward
    union_inward_notch(
        &region,
        hole_x + hole_width / 2.0 - top_revealer_width / 2.0,
        hole_y - border_width_f,
        top_revealer_width,
        top_revealer_height + border_width_f,
    );

    // Center bottom revealer: touches bottom hole edge, expand downward
    union_inward_notch(
        &region,
        hole_x + hole_width / 2.0 - bottom_revealer_width / 2.0,
        hole_y + hole_height - bottom_revealer_height,
        bottom_revealer_width,
        bottom_revealer_height + border_width_f,
    );

    // Top-left corner revealer: touches top and left hole edges
    union_inward_notch(
        &region,
        hole_x - border_width_f,
        hole_y - border_width_f,
        top_left_revealer_width + border_width_f,
        top_left_revealer_height + border_width_f,
    );

    // Top-right corner revealer: touches top and right hole edges
    union_inward_notch(
        &region,
        hole_x + hole_width - top_right_revealer_width,
        hole_y - border_width_f,
        top_right_revealer_width + border_width_f,
        top_right_revealer_height + border_width_f,
    );

    // Bottom-left corner revealer: touches bottom and left hole edges
    union_inward_notch(
        &region,
        hole_x - border_width_f,
        hole_y + hole_height - bottom_left_revealer_height,
        bottom_left_revealer_width + border_width_f,
        bottom_left_revealer_height + border_width_f,
    );

    // Bottom-right corner revealer: touches bottom and right hole edges
    union_inward_notch(
        &region,
        hole_x + hole_width - bottom_right_revealer_width,
        hole_y + hole_height - bottom_right_revealer_height,
        bottom_right_revealer_width + border_width_f,
        bottom_right_revealer_height + border_width_f,
    );

    region
}

/// The input region of one surface, in its own (widget) coordinates: the
/// output-space region limited to the rectangles `viewport` owns, moved to the
/// widget's origin and cut to the widget's `widget_width` x `widget_height`.
pub(crate) fn surface_input_region(
    style: &FrameStyle,
    total_width: f64,
    total_height: f64,
    viewport: &Viewport,
    widget_width: i32,
    widget_height: i32,
) -> Region {
    let region = output_input_region(style, total_width, total_height);
    if let Some(owned) = &viewport.owned {
        let mask = Region::create();
        for rect in owned.iter().filter(|r| !r.is_empty()) {
            let _ = mask.union_rectangle(&RectangleInt::new(
                rect.x.floor() as i32,
                rect.y.floor() as i32,
                rect.w.ceil() as i32,
                rect.h.ceil() as i32,
            ));
        }
        let _ = region.intersect(&mask);
    }
    let (origin_x, origin_y) = viewport.origin;
    region.translate(-(origin_x.round() as i32), -(origin_y.round() as i32));
    let _ = region.intersect_rectangle(&RectangleInt::new(0, 0, widget_width, widget_height));
    region
}

/// Stable hash of a region's rectangles, to skip resubmitting an unchanged
/// input region.
fn region_hash(region: &Region) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for i in 0..region.num_rectangles() {
        let r = region.rectangle(i);
        (r.x(), r.y(), r.width(), r.height()).hash(&mut hasher);
    }
    hasher.finish()
}

// --- Subclass internals ---

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct FrameDrawWidget {
        pub(super) style: RefCell<FrameStyle>,
        pub(super) resolved_style: RefCell<Option<FrameStyle>>,
        pub(super) css_hash: Cell<u64>,
        /// Which part of the output-wide frame this widget draws.
        pub(super) viewport: RefCell<Viewport>,
        /// Set when the widget's window got a new surface (first realize,
        /// or remapped after an unrealize): that surface takes input
        /// everywhere until it gets a region, so the next region goes out at
        /// once instead of after the debounce.
        pub(super) fresh_surface: Cell<bool>,
        /// Last (radius, border width) announced via `metrics-changed`.
        pub(super) last_metrics: Cell<Option<(f64, f64)>>,
        /// Hash of the rectangles last submitted to
        /// `surface.set_input_region`. We re-compute the region on every
        /// snapshot (the cheap part), but the actual wayland call is
        /// gated on the geometry changing — without this guard, every
        /// animation frame submits an identical region and that triggers
        /// spurious pointer leave / enter events, which makes the bar
        /// visibly flicker as buttons drop and re-gain their hover state.
        pub(super) last_region_hash: Cell<u64>,
        /// Debounce timer for `update_input_region`. While a menu is
        /// revealing or hiding, the revealer animates 60× per second and
        /// would otherwise submit a brand-new `set_input_region` each
        /// frame — that triggers a pointer leave/enter cycle each time
        /// and makes the bar visibly flicker. We collect updates and
        /// only push the latest region to wayland once the animation
        /// settles (no further updates for ~120 ms). The compositor still
        /// sees the final hole geometry before the user has a chance to
        /// interact with the freshly-revealed menu, so click routing
        /// stays correct.
        pub(super) pending_region_timeout: RefCell<Option<glib::SourceId>>,
        pub(super) pending_region_args: RefCell<Option<RegionArgs>>,
    }

    #[derive(Clone, Debug)]
    pub(super) struct RegionArgs {
        pub style: FrameStyle,
        pub total_width: f64,
        pub total_height: f64,
        pub viewport: Viewport,
        pub widget_width: i32,
        pub widget_height: i32,
    }

    #[object_subclass]
    impl ObjectSubclass for FrameDrawWidget {
        const NAME: &'static str = "FrameDrawWidget";
        type Type = super::FrameDrawWidget;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("frame-draw");
        }
    }

    impl ObjectImpl for FrameDrawWidget {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                // (border radius, border width) after the theme CSS resolved
                // to new values. The edge strips size themselves from these.
                vec![
                    glib::subclass::Signal::builder("metrics-changed")
                        .param_types([f64::static_type(), f64::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_hexpand(true);
            widget.set_vexpand(true);
            widget.set_can_focus(false);
            widget.set_can_target(false);
            widget.set_sensitive(false);

            widget.connect_realize(|w| {
                w.imp().last_region_hash.set(0);
                w.imp().fresh_surface.set(true);
            });

            widget.connect_notify(Some("css-classes"), |w, _| {
                w.imp().invalidate_css_cache();
                w.queue_draw();
            });

            if let Some(settings) = gtk::Settings::default() {
                let w = widget.downgrade();
                settings.connect_notify_local(Some("gtk-theme-name"), move |_, _| {
                    if let Some(w) = w.upgrade() {
                        w.imp().invalidate_css_cache();
                        w.queue_draw();
                    }
                });

                let w = widget.downgrade();
                settings.connect_notify_local(
                    Some("gtk-application-prefer-dark-theme"),
                    move |_, _| {
                        if let Some(w) = w.upgrade() {
                            w.imp().invalidate_css_cache();
                            w.queue_draw();
                        }
                    },
                );
            }
        }
    }

    impl WidgetImpl for FrameDrawWidget {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let widget_width = widget.width() as f64;
            let widget_height = widget.height() as f64;

            if widget_width <= 0.0 || widget_height <= 0.0 {
                return;
            }

            let style = self.get_resolved_style();
            let viewport = self.viewport.borrow().clone();
            let (total_width, total_height) =
                viewport.output.unwrap_or((widget_width, widget_height));

            if style.draw_frame {
                let bounds =
                    gtk::graphene::Rect::new(0.0, 0.0, widget_width as f32, widget_height as f32);
                let cr = snapshot.append_cairo(&bounds);
                paint_frame(&cr, &style, total_width, total_height, &viewport);
            }

            self.schedule_input_region_update(RegionArgs {
                style,
                total_width,
                total_height,
                viewport,
                widget_width: widget_width as i32,
                widget_height: widget_height as i32,
            });
        }
    }

    impl FrameDrawWidget {
        fn get_resolved_style(&self) -> FrameStyle {
            let widget = self.obj();

            #[allow(deprecated)]
            let ctx = widget.style_context();
            #[allow(deprecated)]
            let css_str = ctx.to_string(gtk::StyleContextPrintFlags::SHOW_STYLE);
            let new_hash = hash_str(css_str.as_str());

            if new_hash != self.css_hash.get() || self.resolved_style.borrow().is_none() {
                let mut style = self.style.borrow().clone();
                let vars = CssVars::parse(css_str.as_str());
                apply_css_vars(&mut style, &vars);

                self.css_hash.set(new_hash);
                let metrics = (style.border_radius, style.border_width);
                *self.resolved_style.borrow_mut() = Some(style);
                if self.last_metrics.get() != Some(metrics) {
                    self.last_metrics.set(Some(metrics));
                    // Emitted from snapshot: let listeners resize outside it.
                    let weak = self.obj().downgrade();
                    glib::idle_add_local_once(move || {
                        if let Some(w) = weak.upgrade() {
                            w.emit_by_name::<()>("metrics-changed", &[&metrics.0, &metrics.1]);
                        }
                    });
                }
            }

            self.resolved_style.borrow().clone().unwrap()
        }

        pub(super) fn invalidate_css_cache(&self) {
            self.css_hash.set(0);
            *self.resolved_style.borrow_mut() = None;
        }

        /// Coalesce repeated input-region updates during animations.
        /// Stores the latest args and (re)arms a short debounce timer;
        /// only the final args after the animation settles get pushed
        /// to wayland.
        fn schedule_input_region_update(&self, args: RegionArgs) {
            if self.fresh_surface.replace(false) {
                if let Some(id) = self.pending_region_timeout.borrow_mut().take() {
                    id.remove();
                }
                self.pending_region_args.replace(None);
                self.update_input_region(&args);
                return;
            }
            *self.pending_region_args.borrow_mut() = Some(args);
            if let Some(id) = self.pending_region_timeout.borrow_mut().take() {
                id.remove();
            }
            let weak = self.obj().downgrade();
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                    let Some(widget) = weak.upgrade() else {
                        return;
                    };
                    let imp = widget.imp();
                    imp.pending_region_timeout.replace(None);
                    let Some(args) = imp.pending_region_args.borrow_mut().take() else {
                        return;
                    };
                    imp.update_input_region(&args);
                });
            self.pending_region_timeout.replace(Some(id));
        }

        fn update_input_region(&self, args: &RegionArgs) {
            let widget = self.obj();

            let Some(native) = widget.native() else {
                return;
            };
            let Some(surface) = native.surface() else {
                return;
            };

            let region = surface_input_region(
                &args.style,
                args.total_width,
                args.total_height,
                &args.viewport,
                args.widget_width,
                args.widget_height,
            );
            // Wayland's set_input_region produces pointer-leave / -enter
            // events even when the region is unchanged, and that shows up as
            // bar flicker during menu reveal animations: skip identical ones.
            let hash = region_hash(&region);
            if hash == self.last_region_hash.get() {
                return;
            }
            self.last_region_hash.set(hash);
            surface.set_input_region(Some(&region));
        }
    }
}

// --- Public wrapper ---

glib::wrapper! {
    pub struct FrameDrawWidget(ObjectSubclass<imp::FrameDrawWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FrameDrawWidget {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn update_style(&self, f: impl FnOnce(&mut FrameStyle)) {
        f(&mut self.imp().style.borrow_mut());
        self.imp().invalidate_css_cache();
        self.queue_draw();
    }

    /// Set which part of the output-wide frame this widget draws.
    pub fn set_viewport(&self, viewport: Viewport) {
        if *self.imp().viewport.borrow() == viewport {
            return;
        }
        *self.imp().viewport.borrow_mut() = viewport;
        self.queue_draw();
    }

    pub fn set_style(&self, style: FrameStyle) {
        *self.imp().style.borrow_mut() = style;
        self.imp().invalidate_css_cache();
        self.queue_draw();
    }

    pub fn set_draw_frame(&self, draw: bool) {
        self.imp().style.borrow_mut().draw_frame = draw;
        self.imp().invalidate_css_cache();
        self.queue_draw();
    }

    pub fn border_width(&self) -> f64 {
        self.imp().style.borrow().border_width
    }

    pub fn border_radius(&self) -> f64 {
        self.imp().style.borrow().border_radius
    }

    pub fn set_border_width(&self, width: f64) {
        self.imp().style.borrow_mut().border_width = width;
        self.imp().invalidate_css_cache();
        self.queue_draw();
    }

    pub fn set_border_radius(&self, radius: f64) {
        self.imp().style.borrow_mut().border_radius = radius;
        self.imp().invalidate_css_cache();
        self.queue_draw();
    }
}

impl Default for FrameDrawWidget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use relm4::gtk::cairo::{Format, ImageSurface};

    /// The owner's look: 36 px top bar, 30 px bottom bar, 12 px radius,
    /// 2 px border, opaque frame colours.
    pub(crate) fn owner_style() -> FrameStyle {
        FrameStyle {
            border_radius: 12.0,
            background_rgba: (31.0 / 255.0, 31.0 / 255.0, 46.0 / 255.0, 1.0),
            border_rgba: (48.0 / 255.0, 51.0 / 255.0, 69.0 / 255.0, 1.0),
            border_width: 2.0,
            left_thickness: 0.0,
            right_thickness: 0.0,
            top_thickness: 36.0,
            bottom_thickness: 30.0,
            ..FrameStyle::default()
        }
    }

    /// The same style with a top-right menu (and its notch) open.
    pub(crate) fn with_menus(mut style: FrameStyle) -> FrameStyle {
        style.top_right_revealer_size = (120.0, 90.0);
        style.top_revealer_size = (80.0, 40.5);
        style.right_thickness = 60.0;
        style.right_expander_width = 60.0;
        style.right_top_expander_height = 20.0;
        style.right_bottom_expander_height = 30.0;
        style
    }

    /// Paint one surface of `width` x `height` and return its pixels.
    pub(crate) fn render(
        style: &FrameStyle,
        total: (f64, f64),
        viewport: &Viewport,
        width: i32,
        height: i32,
    ) -> ImageSurface {
        let surface = ImageSurface::create(Format::ARgb32, width, height).expect("surface");
        {
            let cr = Context::new(&surface).expect("context");
            paint_frame(&cr, style, total.0, total.1, viewport);
        }
        surface.flush();
        surface
    }

    /// Composite surface pieces (each placed at its viewport origin) the way
    /// the compositor does, and return the output-sized result.
    pub(crate) fn composite(total: (i32, i32), pieces: &[(ImageSurface, (f64, f64))]) -> Vec<u8> {
        let out = ImageSurface::create(Format::ARgb32, total.0, total.1).expect("surface");
        {
            let cr = Context::new(&out).expect("context");
            for (piece, origin) in pieces {
                cr.set_source_surface(piece, origin.0, origin.1)
                    .expect("source");
                cr.paint().expect("paint");
            }
        }
        out.flush();
        pixels(out)
    }

    pub(crate) fn pixels(mut surface: ImageSurface) -> Vec<u8> {
        surface.data().expect("data").to_vec()
    }

    /// Assert two output-sized pixel buffers match, naming the first pixel
    /// that differs instead of dumping megabytes. Cairo flattens arcs at the
    /// translated coordinates, so an antialiased arc pixel may round one
    /// step (1/255) differently; anything more is a real difference.
    pub(crate) fn assert_same_pixels(got: &[u8], want: &[u8], width: usize, what: &str) {
        assert_eq!(got.len(), want.len(), "{what}: size");
        let differs = |i: usize| {
            got[i * 4..i * 4 + 4]
                .iter()
                .zip(&want[i * 4..i * 4 + 4])
                .any(|(a, b)| a.abs_diff(*b) > 1)
        };
        if let Some(i) = (0..got.len() / 4).find(|&i| differs(i)) {
            let n = (0..got.len() / 4).filter(|&i| differs(i)).count();
            panic!(
                "{what}: pixel ({}, {}) is {:?}, want {:?} ({n} pixels differ)",
                i % width,
                i / width,
                &got[i * 4..i * 4 + 4],
                &want[i * 4..i * 4 + 4]
            );
        }
    }

    #[test]
    fn full_viewport_paints_like_before() {
        // The default viewport is the old single full-output surface.
        let style = with_menus(owner_style());
        let a = pixels(render(
            &style,
            (400.0, 300.0),
            &Viewport::default(),
            400,
            300,
        ));
        let b = pixels(render(
            &style,
            (400.0, 300.0),
            &Viewport {
                origin: (0.0, 0.0),
                output: Some((400.0, 300.0)),
                owned: Some(vec![Rect::new(0.0, 0.0, 400.0, 300.0)]),
            },
            400,
            300,
        ));
        assert_eq!(a, b);
        // Sanity: something was painted, and the hole centre is clear.
        assert!(a.iter().any(|&p| p != 0));
        let stride = 400 * 4;
        assert_eq!(
            &a[150 * stride + 200 * 4..150 * stride + 200 * 4 + 4],
            &[0, 0, 0, 0]
        );
    }

    #[test]
    fn disjoint_pieces_composite_to_the_full_frame() {
        let (w, h) = (400.0, 300.0);
        for style in [owner_style(), with_menus(owner_style())] {
            let full = pixels(render(&style, (w, h), &Viewport::default(), 400, 300));
            // An arbitrary split: a top strip, a bottom strip, two side strips
            // and the middle, each its own surface at its own origin.
            let parts = [
                Rect::new(0.0, 0.0, w, 50.0),
                Rect::new(0.0, 255.0, w, 45.0),
                Rect::new(0.0, 50.0, 3.0, 205.0),
                Rect::new(397.0, 50.0, 3.0, 205.0),
                Rect::new(3.0, 50.0, 394.0, 205.0),
            ];
            let pieces: Vec<_> = parts
                .iter()
                .map(|r| {
                    let vp = Viewport {
                        origin: (r.x, r.y),
                        output: Some((w, h)),
                        owned: Some(vec![*r]),
                    };
                    (
                        render(&style, (w, h), &vp, r.w as i32, r.h as i32),
                        (r.x, r.y),
                    )
                })
                .collect();
            assert_same_pixels(&composite((400, 300), &pieces), &full, 400, "pieces");
        }
    }

    fn region_rects(region: &Region) -> Vec<(i32, i32, i32, i32)> {
        (0..region.num_rectangles())
            .map(|i| {
                let r = region.rectangle(i);
                (r.x(), r.y(), r.width(), r.height())
            })
            .collect()
    }

    #[test]
    fn surface_input_regions_partition_the_output_region() {
        let (w, h) = (400.0, 300.0);
        let style = with_menus(owner_style());
        let whole = output_input_region(&style, w, h);
        // The zero-size corner notches add bw x bw squares just outside the
        // output (harmless, the surface ignores them); compare on-output.
        let _ = whole.intersect_rectangle(&RectangleInt::new(0, 0, w as i32, h as i32));
        let parts = [Rect::new(0.0, 0.0, w, 50.0), Rect::new(0.0, 50.0, w, 250.0)];
        let union = Region::create();
        for r in parts {
            let vp = Viewport {
                origin: (r.x, r.y),
                output: Some((w, h)),
                owned: Some(vec![r]),
            };
            let local = surface_input_region(&style, w, h, &vp, r.w as i32, r.h as i32);
            local.translate(r.x as i32, r.y as i32);
            let _ = union.union(&local);
        }
        assert_eq!(region_rects(&union), region_rects(&whole));
        // The hole centre passes clicks through; the top band takes them.
        assert!(!whole.contains_point(200, 150));
        assert!(whole.contains_point(5, 5));
    }
}
