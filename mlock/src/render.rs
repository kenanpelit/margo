//! Cairo + pango drawing for one frame.
//!
//! Inputs:
//!   • a mutable byte slice the size of the surface (ARGB8888, stride = w*4)
//!   • dimensions + seat state for the password dots + fail message
//! Output: that buffer painted with the lock UI.

use anyhow::{Context, Result};
use cairo::{Format, ImageSurface};
use chrono::Local;

use crate::seat::SeatState;

const BG_R: f64 = 0.05;
const BG_G: f64 = 0.05;
const BG_B: f64 = 0.10;

const TEXT_R: f64 = 0.94;
const TEXT_G: f64 = 0.95;
const TEXT_B: f64 = 0.96;

const MUTED_R: f64 = 0.66;
const MUTED_G: f64 = 0.69;
const MUTED_B: f64 = 0.74;

const FAIL_R: f64 = 0.94;
const FAIL_G: f64 = 0.45;
const FAIL_B: f64 = 0.43;

const FONT_CLOCK_PT: i32 = 96;
const FONT_DATE_PT: i32 = 22;
const FONT_USER_PT: i32 = 18;
const FONT_STATUS_PT: i32 = 14;
const FONT_DOTS_PT: i32 = 24;
const DOTS_SPACING: f64 = 16.0;
const DOT_RADIUS: f64 = 6.0;

pub fn draw_lock_frame(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    stride: i32,
    seat: &SeatState,
    user: &str,
) -> Result<()> {
    // Cairo borrows the pixel buffer; we keep `pixels` alive for the
    // surface's lifetime. SAFETY: lifetime is scoped to this function;
    // we flush + drop the surface before returning.
    let surface = unsafe {
        ImageSurface::create_for_data_unsafe(
            pixels.as_mut_ptr(),
            Format::ARgb32,
            width,
            height,
            stride,
        )
        .context("cairo surface")?
    };
    let cr = cairo::Context::new(&surface).context("cairo context")?;

    // ── 1. Background fill ─────────────────────────────────────────
    cr.set_source_rgb(BG_R, BG_G, BG_B);
    cr.paint().ok();

    // ── 2. Centred Clock + Date + User + Dots ──────────────────────
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;

    let now = Local::now();
    let clock_str = now.format("%H:%M").to_string();
    let date_str = now.format("%A, %-d %B %Y").to_string();

    draw_centered(&cr, cx, cy - 120.0, &clock_str, FONT_CLOCK_PT, true, false)?;
    draw_centered(&cr, cx, cy - 40.0, &date_str, FONT_DATE_PT, false, true)?;
    draw_centered(
        &cr,
        cx,
        cy + 20.0,
        &format!("@ {user}"),
        FONT_USER_PT,
        false,
        true,
    )?;

    // Password dots — one for each character. Hides length-leak by
    // capping at 16 visible dots (rest are implicit).
    let visible_dots = seat.password.chars().count().min(16);
    if visible_dots > 0 {
        let total_w = visible_dots as f64 * (DOT_RADIUS * 2.0 + DOTS_SPACING)
            - DOTS_SPACING.max(0.0);
        let start_x = cx - total_w / 2.0;
        cr.set_source_rgb(TEXT_R, TEXT_G, TEXT_B);
        for i in 0..visible_dots {
            let x = start_x + i as f64 * (DOT_RADIUS * 2.0 + DOTS_SPACING) + DOT_RADIUS;
            cr.arc(x, cy + 80.0, DOT_RADIUS, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
        }
    } else {
        // Empty input — show subtle hint placeholder.
        draw_centered(
            &cr,
            cx,
            cy + 80.0,
            "🔒",
            FONT_DOTS_PT,
            false,
            true,
        )?;
    }

    // Fail message.
    if let Some(msg) = seat.fail_message.as_deref() {
        cr.set_source_rgb(FAIL_R, FAIL_G, FAIL_B);
        draw_centered_raw(&cr, cx, cy + 150.0, msg, FONT_STATUS_PT)?;
    }

    surface.flush();
    Ok(())
}

fn draw_centered(
    cr: &cairo::Context,
    cx: f64,
    y: f64,
    text: &str,
    pt: i32,
    bold: bool,
    muted: bool,
) -> Result<()> {
    if muted {
        cr.set_source_rgb(MUTED_R, MUTED_G, MUTED_B);
    } else {
        cr.set_source_rgb(TEXT_R, TEXT_G, TEXT_B);
    }
    draw_centered_raw_styled(cr, cx, y, text, pt, bold)
}

fn draw_centered_raw(
    cr: &cairo::Context,
    cx: f64,
    y: f64,
    text: &str,
    pt: i32,
) -> Result<()> {
    draw_centered_raw_styled(cr, cx, y, text, pt, false)
}

fn draw_centered_raw_styled(
    cr: &cairo::Context,
    cx: f64,
    y: f64,
    text: &str,
    pt: i32,
    bold: bool,
) -> Result<()> {
    let layout = pangocairo::functions::create_layout(cr);
    let weight = if bold {
        pango::Weight::Bold
    } else {
        pango::Weight::Normal
    };
    let mut desc = pango::FontDescription::new();
    desc.set_family("Maple Mono NF, Noto Sans, sans");
    desc.set_size(pt * pango::SCALE);
    desc.set_weight(weight);
    layout.set_font_description(Some(&desc));
    layout.set_text(text);
    let (ink_w, _ink_h) = layout.pixel_size();
    let x = cx - ink_w as f64 / 2.0;
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
    Ok(())
}
