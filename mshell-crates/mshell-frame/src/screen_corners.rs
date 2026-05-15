//! Rounded screen-corners overlay.
//!
//! Four tiny layer-shell windows per monitor, one per corner,
//! sized `radius × radius`. Each is anchored to its corner edge
//! pair and draws a mask shape: solid fill with a quarter-circle
//! cut out so the visible result looks like the screen itself
//! has rounded corners.
//!
//! Layer: `Overlay`. Sits above app windows so it actually masks
//! their content. Input region: empty (set on realize) so clicks
//! pass straight through to whatever's underneath — without this
//! the corner would intercept clicks on the few pixels at the
//! very edge.
//!
//! Colour: solid black. Most users want black corners regardless
//! of theme (matches the bezel they "should" be sitting behind).
//! Themed corners are a future knob.

use gtk4_layer_shell::{Edge, Layer, LayerShell};
use relm4::gtk;
use relm4::gtk::cairo;
use relm4::gtk::prelude::{
    DrawingAreaExt, DrawingAreaExtManual, GtkWindowExt, NativeExt, SurfaceExt, WidgetExt,
};

/// Which screen corner an overlay window covers. The variant
/// drives both the layer-shell anchor pair and the Cairo arc
/// orientation (the curve always bulges away from the corner).
#[derive(Debug, Clone, Copy)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Spawn four corner-overlay windows on `monitor`. Returns the
/// owning `Vec` — drop it to remove the corners (which is what
/// `ShellInput::RemoveWindowGroup` ends up doing on hotplug).
pub fn spawn(monitor: &gtk::gdk::Monitor, radius: u32) -> Vec<gtk::Window> {
    if radius == 0 {
        return Vec::new();
    }
    [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ]
    .into_iter()
    .map(|c| build_corner_window(monitor, c, radius))
    .collect()
}

fn build_corner_window(monitor: &gtk::gdk::Monitor, corner: Corner, radius: u32) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_decorated(false);
    window.set_default_size(radius as i32, radius as i32);
    window.add_css_class("screen-corner");

    window.init_layer_shell();
    window.set_monitor(Some(monitor));
    window.set_namespace(Some("mshell-screen-corner"));
    // Overlay so we sit above every app surface — otherwise an
    // active toplevel could paint over the mask and the corner
    // would reappear sharp.
    window.set_layer(Layer::Overlay);
    window.set_exclusive_zone(0);

    let (e1, e2) = match corner {
        Corner::TopLeft => (Edge::Top, Edge::Left),
        Corner::TopRight => (Edge::Top, Edge::Right),
        Corner::BottomLeft => (Edge::Bottom, Edge::Left),
        Corner::BottomRight => (Edge::Bottom, Edge::Right),
    };
    window.set_anchor(e1, true);
    window.set_anchor(e2, true);

    let area = gtk::DrawingArea::new();
    area.set_content_width(radius as i32);
    area.set_content_height(radius as i32);
    let r = radius as f64;
    area.set_draw_func(move |_, ctx, _, _| {
        draw_corner_mask(ctx, corner, r);
    });
    window.set_child(Some(&area));

    // Empty input region so the corner pixels pass clicks
    // through to whatever's underneath. Has to wait for the
    // surface to exist, hence the realize signal.
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            let empty = cairo::Region::create();
            surface.set_input_region(&empty);
        }
    });

    window.set_visible(true);
    window
}

/// Draw the mask shape. The visible result is the union of the
/// `r × r` square minus a quarter-disk that exits toward the
/// screen interior — i.e. solid colour on the "outside" of the
/// rounded corner, transparent on the "inside".
fn draw_corner_mask(ctx: &cairo::Context, corner: Corner, r: f64) {
    // Solid black — same colour the monitor bezel would be.
    // Themed corners (matching `Color.mSurface`) are a future
    // knob; black is the most common request because it
    // disappears against the bezel.
    let _ = ctx.save();

    // Centre of the rounding arc — the point inside the visible
    // screen area that the curve sweeps around.
    let (cx, cy, sweep_start, sweep_end) = match corner {
        // Top-left: centre is at (r, r), arc sweeps from the
        // top edge (angle 3π/2) clockwise to the left edge
        // (angle π). `arc_negative` traces clockwise.
        Corner::TopLeft => (r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::PI),
        // Top-right: centre (0, r), arc from right edge to top.
        Corner::TopRight => (0.0, r, 0.0, -std::f64::consts::FRAC_PI_2),
        // Bottom-left: centre (r, 0), arc from left to bottom.
        Corner::BottomLeft => (r, 0.0, std::f64::consts::PI, std::f64::consts::FRAC_PI_2),
        // Bottom-right: centre (0, 0), arc from bottom to right.
        Corner::BottomRight => (
            0.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            0.0,
        ),
    };

    // Build the L-shape: walk the outer rectangle perimeter for
    // this corner, then arc-back along the inner curve.
    match corner {
        Corner::TopLeft => {
            ctx.move_to(0.0, 0.0);
            ctx.line_to(r, 0.0);
            ctx.arc_negative(cx, cy, r, sweep_start, sweep_end);
            ctx.line_to(0.0, 0.0);
        }
        Corner::TopRight => {
            ctx.move_to(r, 0.0);
            ctx.line_to(r, r);
            ctx.arc_negative(cx, cy, r, sweep_start, sweep_end);
            ctx.line_to(r, 0.0);
        }
        Corner::BottomLeft => {
            ctx.move_to(0.0, r);
            ctx.line_to(0.0, 0.0);
            ctx.arc_negative(cx, cy, r, sweep_start, sweep_end);
            ctx.line_to(0.0, r);
        }
        Corner::BottomRight => {
            ctx.move_to(r, r);
            ctx.line_to(0.0, r);
            ctx.arc_negative(cx, cy, r, sweep_start, sweep_end);
            ctx.line_to(r, r);
        }
    }
    ctx.close_path();
    ctx.set_source_rgb(0.0, 0.0, 0.0);
    let _ = ctx.fill();
    let _ = ctx.restore();
}
