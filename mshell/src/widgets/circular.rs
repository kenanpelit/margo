//! Eww's `(circular-progress)` widget, in GTK4.
//!
//! Wraps a `GtkOverlay` around a `GtkDrawingArea` and an arbitrary
//! child (typically a Nerd-Font icon `GtkLabel`). The drawing area
//! paints a thin background track + a foreground arc that runs from
//! 12 o'clock clockwise around to the value's position, matching the
//! eww output exactly.
//!
//! Colours come from CSS so each consumer (battery / memory) can
//! restyle without touching this file: set the foreground colour
//! with a CSS class (`.ring-fg-batt` → mint, `.ring-fg-mem` → peach
//! …) read at paint time via `widget.color()`. The background track
//! is a fixed dim grey that matches saimoom's `#38384d`.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, DrawingArea, Label, Overlay};

const SIZE: i32 = 26;
const THICKNESS: f64 = 3.0;
/// Background track colour, lifted straight from eww.scss
/// (`.batbar, .membar { background-color: #38384d; }`).
const TRACK_RGB: (f64, f64, f64) = (0x38 as f64 / 255.0, 0x38 as f64 / 255.0, 0x4d as f64 / 255.0);

/// Handle returned to the caller — `value` is set in 0.0..=1.0 to
/// move the arc.
#[derive(Clone)]
pub struct Ring {
    pub widget: Overlay,
    drawing: DrawingArea,
    value: Rc<Cell<f64>>,
}

impl Ring {
    pub fn new(name: &str, icon: &str) -> Self {
        let drawing = DrawingArea::builder()
            .content_width(SIZE)
            .content_height(SIZE)
            .build();
        drawing.add_css_class("ring");

        let value: Rc<Cell<f64>> = Rc::new(Cell::new(0.0_f64));
        let value_for_draw = value.clone();
        let drawing_for_draw = drawing.clone();
        drawing.set_draw_func(move |_da, cr, w, h| {
            let v: f64 = value_for_draw.get().clamp(0.0_f64, 1.0_f64);
            let fg = stroke_color(&drawing_for_draw);
            paint(cr, w as f64, h as f64, v, fg);
        });

        let label = Label::builder()
            .label(icon)
            .halign(Align::Center)
            .valign(Align::Center)
            .build();
        label.add_css_class("ring-icon");

        let overlay = Overlay::builder().name(name).build();
        overlay.set_child(Some(&drawing));
        overlay.add_overlay(&label);
        overlay.add_css_class("module");
        overlay.add_css_class("ring-host");

        Self {
            widget: overlay,
            drawing,
            value,
        }
    }

    /// Update the arc position. Triggers a redraw if the value
    /// actually changed (avoids draw-thrash on the 1 Hz tick).
    pub fn set_value(&self, v: f64) {
        let v = v.clamp(0.0, 1.0);
        if (self.value.get() - v).abs() > f64::EPSILON {
            self.value.set(v);
            self.drawing.queue_draw();
        }
    }
}

fn paint(cr: &gtk::cairo::Context, w: f64, h: f64, value: f64, fg: (f64, f64, f64)) {
    let cx = w / 2.0;
    let cy = h / 2.0;
    let radius = (w.min(h) / 2.0) - THICKNESS / 2.0 - 1.0;

    cr.set_line_width(THICKNESS);
    cr.set_line_cap(gtk::cairo::LineCap::Round);

    // Background track — full circle, dim grey.
    cr.set_source_rgb(TRACK_RGB.0, TRACK_RGB.1, TRACK_RGB.2);
    cr.arc(cx, cy, radius, 0.0, 2.0 * PI);
    let _ = cr.stroke();

    if value <= 0.0 {
        return;
    }

    // Foreground arc, clockwise from 12 o'clock.
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    let start = -PI / 2.0;
    let end = start + 2.0 * PI * value;
    cr.arc(cx, cy, radius, start, end);
    let _ = cr.stroke();
}

/// CSS-driven foreground colour. We resolve `widget.color()` so each
/// consumer can tag its drawing area with a class like `.ring-fg-batt`
/// and let `style.css` set `color: #afbea2;` — the ring picks it up
/// without touching this file.
fn stroke_color(widget: &DrawingArea) -> (f64, f64, f64) {
    let rgba = widget.color();
    (
        rgba.red() as f64,
        rgba.green() as f64,
        rgba.blue() as f64,
    )
}

/// Convenience: build a `Ring` whose foreground colour comes from the
/// given CSS class on the inner DrawingArea.
pub fn build(name: &str, icon: &str, fg_class: &str) -> Ring {
    let ring = Ring::new(name, icon);
    ring.drawing.add_css_class(fg_class);
    ring
}

// Silence the dead-code lint while widgets/circular grows callers in
// the next two stages.
#[allow(dead_code)]
fn _unused() {
    let _ = glib::ControlFlow::Continue;
}
