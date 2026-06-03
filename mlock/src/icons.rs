//! Hand-drawn vector glyphs for the locker.
//!
//! The lock screen used emoji (🔒 ⇪ 🔋 ⚡ 🪫) and plain-text power hints.
//! Emoji render inconsistently across fonts, ignore the matugen palette,
//! and look dated. These are crisp line/solid icons stroked straight into
//! cairo in the theme colour instead — each fn draws inside a `size`-tall
//! box centred on `(cx, cy)`.

use cairo::Context;
use std::f64::consts::{PI, TAU};

pub type Rgb = (f64, f64, f64);

fn pen(cr: &Context, size: f64, color: Rgb, alpha: f64) {
    cr.set_line_width((size / 9.0).max(1.4));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
}

fn rrect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Closed padlock — outline body, shackle arc, keyhole dot.
pub fn lock(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    pen(cr, size, color, alpha);
    let bw = size * 0.62;
    let bh = size * 0.46;
    let bx = cx - bw / 2.0;
    let by = cy - bh / 2.0 + size * 0.14;
    rrect(cr, bx, by, bw, bh, size * 0.09);
    cr.stroke().ok();
    // Shackle: a semicircle sitting on the body's top edge.
    let sr = size * 0.2;
    cr.new_sub_path();
    cr.arc(cx, by, sr, PI, TAU);
    cr.stroke().ok();
    // Keyhole.
    cr.arc(cx, by + bh * 0.52, size * 0.055, 0.0, TAU);
    cr.fill().ok();
}

/// Eighth note — filled notehead + stem + flag. Marks the now-playing line.
pub fn note(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    pen(cr, size, color, alpha);
    let head_r = size * 0.2;
    let head_cx = cx - size * 0.12;
    let head_cy = cy + size * 0.28;
    // Stem up the right side of the head.
    let stem_x = head_cx + head_r * 0.92;
    cr.move_to(stem_x, head_cy);
    cr.line_to(stem_x, cy - size * 0.4);
    cr.stroke().ok();
    // Flag off the stem top.
    cr.move_to(stem_x, cy - size * 0.4);
    cr.curve_to(
        stem_x + size * 0.22,
        cy - size * 0.3,
        stem_x + size * 0.24,
        cy - size * 0.12,
        stem_x + size * 0.04,
        cy - size * 0.02,
    );
    cr.stroke().ok();
    // Filled notehead.
    cr.save().ok();
    cr.translate(head_cx, head_cy);
    cr.rotate(-0.35);
    cr.scale(1.25, 0.9);
    cr.arc(0.0, 0.0, head_r, 0.0, TAU);
    cr.restore().ok();
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    cr.fill().ok();
}

/// Battery body + terminal nub + a fill bar at `level` (0..1).
pub fn battery(cr: &Context, cx: f64, cy: f64, size: f64, level: f64, color: Rgb, alpha: f64) {
    let w = size;
    let h = size * 0.52;
    let bw = w * 0.88;
    let x = cx - w / 2.0;
    let y = cy - h / 2.0;
    pen(cr, h, color, alpha);
    let lw = cr.line_width();
    rrect(cr, x, y, bw, h, h * 0.24);
    cr.stroke().ok();
    // Terminal nub.
    let nub_h = h * 0.42;
    rrect(cr, x + bw, cy - nub_h / 2.0, w * 0.07, nub_h, w * 0.03);
    cr.fill().ok();
    // Fill bar.
    let pad = lw * 1.4;
    let fill_w = (bw - pad * 2.0) * level.clamp(0.0, 1.0);
    if fill_w > 0.5 {
        rrect(
            cr,
            x + pad,
            y + pad,
            fill_w,
            h - pad * 2.0,
            (h - pad * 2.0) * 0.3,
        );
        cr.fill().ok();
    }
}

/// Lightning bolt (solid) — charging glyph.
pub fn bolt(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    let s = size;
    cr.new_sub_path();
    cr.move_to(cx + s * 0.10, cy - s * 0.5);
    cr.line_to(cx - s * 0.22, cy + s * 0.06);
    cr.line_to(cx - s * 0.02, cy + s * 0.06);
    cr.line_to(cx - s * 0.10, cy + s * 0.5);
    cr.line_to(cx + s * 0.22, cy - s * 0.06);
    cr.line_to(cx + s * 0.02, cy - s * 0.06);
    cr.close_path();
    cr.fill().ok();
}

/// Caps-lock glyph — an up-arrow over a baseline bar (the ⇪ shape).
pub fn caps(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    pen(cr, size, color, alpha);
    let half = size * 0.34;
    let stem = half * 0.42;
    cr.new_sub_path();
    cr.move_to(cx, cy - size * 0.42);
    cr.line_to(cx - half, cy - size * 0.02);
    cr.line_to(cx - stem, cy - size * 0.02);
    cr.line_to(cx - stem, cy + size * 0.18);
    cr.line_to(cx + stem, cy + size * 0.18);
    cr.line_to(cx + stem, cy - size * 0.02);
    cr.line_to(cx + half, cy - size * 0.02);
    cr.close_path();
    cr.stroke().ok();
    // Baseline bar.
    cr.move_to(cx - stem, cy + size * 0.34);
    cr.line_to(cx + stem, cy + size * 0.34);
    cr.stroke().ok();
}

/// Power symbol — a ring open at the top with a vertical stem (⏻).
pub fn power(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    pen(cr, size, color, alpha);
    let r = size * 0.36;
    // Ring with a gap centred at the top.
    let gap = 0.55;
    cr.new_sub_path();
    cr.arc(cx, cy, r, -PI / 2.0 + gap, -PI / 2.0 - gap + TAU);
    cr.stroke().ok();
    // Stem from above the ring down to its centre.
    cr.move_to(cx, cy - r - size * 0.07);
    cr.line_to(cx, cy - size * 0.02);
    cr.stroke().ok();
}

/// Restart — a near-complete circular arrow (⟳).
pub fn restart(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    pen(cr, size, color, alpha);
    let r = size * 0.34;
    let start = -PI * 0.35;
    let end = PI * 1.15;
    cr.new_sub_path();
    cr.arc(cx, cy, r, start, end);
    cr.stroke().ok();
    // Arrowhead at the arc's end, tangent to the circle.
    let ex = cx + r * end.cos();
    let ey = cy + r * end.sin();
    let a = size * 0.16;
    cr.new_sub_path();
    cr.move_to(ex - a, ey - a * 0.2);
    cr.line_to(ex, ey);
    cr.line_to(ex + a * 0.2, ey - a);
    cr.stroke().ok();
}

/// Crescent moon (solid) — suspend glyph (☾).
pub fn moon(cr: &Context, cx: f64, cy: f64, size: f64, color: Rgb, alpha: f64) {
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    let r = size * 0.42;
    // Outer disc minus an offset disc → crescent, via arc_negative.
    cr.new_sub_path();
    cr.arc(cx, cy, r, PI * 0.5, PI * 1.5);
    cr.arc_negative(cx - r * 0.45, cy, r * 0.92, PI * 1.5, PI * 0.5);
    cr.close_path();
    cr.fill().ok();
}
