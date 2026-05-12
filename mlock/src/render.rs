//! Cairo + pango drawing for one lock frame.
//!
//! Layout (top → bottom, vertically centred):
//!   • optional 96 px round avatar
//!   • greeting line ("Günaydın, Kenan")
//!   • large clock + date pair
//!   • frosted card (with drop shadow + shake offset) containing
//!     password dots / placeholder pill
//!   • optional Caps Lock chip below the card
//!   • status line — fail message + attempt count, or typing hint

use anyhow::Result;
use cairo::{Format, ImageSurface};
use chrono::{Local, Timelike};

use crate::seat::SeatState;

// Palette over the dimmed wallpaper.
const TEXT: (f64, f64, f64) = (0.96, 0.97, 0.98);
const MUTED: (f64, f64, f64) = (0.78, 0.80, 0.86);
const FAIL: (f64, f64, f64) = (0.95, 0.45, 0.43);
const WARN: (f64, f64, f64) = (0.98, 0.78, 0.45);

const FALLBACK_BG: (f64, f64, f64) = (0.05, 0.05, 0.10);
const DIM_ALPHA: f64 = 0.55;

// Frosted card.
const CARD_ALPHA: f64 = 0.18;
const CARD_RADIUS: f64 = 18.0;
const CARD_PADDING_X: f64 = 48.0;
const CARD_PADDING_Y: f64 = 28.0;

// Avatar.
const AVATAR_SIZE: f64 = 96.0;
const AVATAR_RING_W: f64 = 2.0;

// Typography.
const FONT_FAMILY: &str = "Maple Mono NF, Noto Sans, sans";
const FONT_CLOCK_PT: i32 = 110;
const FONT_DATE_PT: i32 = 22;
const FONT_GREETING_PT: i32 = 20;
const FONT_STATUS_PT: i32 = 14;
const FONT_CAPS_PT: i32 = 12;

// Stack gaps.
const GAP_AVATAR_GREETING: f64 = 20.0;
const GAP_GREETING_CLOCK: f64 = 36.0;
const GAP_CLOCK_DATE: f64 = 8.0;
const GAP_DATE_CARD: f64 = 52.0;
const GAP_INSIDE_CARD: f64 = 0.0; // dots only — no user label any more
const GAP_CARD_CAPS: f64 = 14.0;
const GAP_CAPS_STATUS: f64 = 10.0;

// Dots.
const DOT_RADIUS: f64 = 6.0;
const DOT_SPACING: f64 = 14.0;
const DOTS_BAND_HEIGHT: f64 = 28.0;
const PLACEHOLDER_PILL_W: f64 = 220.0;
const PLACEHOLDER_PILL_H: f64 = 3.0;
const MAX_VISIBLE_DOTS: usize = 24;

// Shake animation.
const SHAKE_DURATION_MS: u64 = 400;
const SHAKE_AMPLITUDE: f64 = 10.0;
const SHAKE_FREQ_HZ: f64 = 14.0;

pub fn draw_lock_frame(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    stride: i32,
    seat: &SeatState,
    user: &str,
    wallpaper: Option<&image::RgbaImage>,
    avatar: Option<&image::RgbaImage>,
) -> Result<()> {
    let surface = unsafe {
        ImageSurface::create_for_data_unsafe(
            pixels.as_mut_ptr(),
            Format::ARgb32,
            width,
            height,
            stride,
        )?
    };
    let cr = cairo::Context::new(&surface)?;

    // 1. Wallpaper or solid fallback.
    if let Some(wp) = wallpaper {
        paint_wallpaper_cover(&cr, wp, width, height)?;
    } else {
        cr.set_source_rgb(FALLBACK_BG.0, FALLBACK_BG.1, FALLBACK_BG.2);
        cr.paint().ok();
    }

    // 2. Uniform dim.
    cr.set_source_rgba(0.0, 0.0, 0.0, DIM_ALPHA);
    cr.paint().ok();

    // 3. Vignette — radial fade darkens the edges, draws the eye to
    //    the centre.
    draw_vignette(&cr, width, height);

    // 4. Build all text layouts up-front so we can measure heights
    //    BEFORE laying out (no overlap, no magic offsets).
    let now = Local::now();
    let greeting_str = format!("{}, {}", greeting_for(now.hour()), display_name(user));
    let clock_str = now.format("%H:%M").to_string();
    let date_str = now.format("%A, %-d %B %Y").to_string();

    let greeting_layout = layout(&cr, &greeting_str, FONT_GREETING_PT, false);
    let clock_layout = layout(&cr, &clock_str, FONT_CLOCK_PT, true);
    let date_layout = layout(&cr, &date_str, FONT_DATE_PT, false);

    let (greeting_w, greeting_h) = greeting_layout.pixel_size();
    let (clock_w, clock_h) = clock_layout.pixel_size();
    let (date_w, date_h) = date_layout.pixel_size();

    let card_content_h = DOTS_BAND_HEIGHT;
    let card_h = card_content_h + CARD_PADDING_Y * 2.0 + GAP_INSIDE_CARD;
    let card_w = (PLACEHOLDER_PILL_W + 60.0)
        .max(MAX_VISIBLE_DOTS as f64 * (DOT_RADIUS * 2.0 + DOT_SPACING))
        + CARD_PADDING_X * 2.0;

    let caps_visible = seat.caps_lock;
    let caps_chip_h = if caps_visible {
        FONT_CAPS_PT as f64 * 1.7 + 14.0
    } else {
        0.0
    };

    let status_h = FONT_STATUS_PT as f64 * 1.6;

    let avatar_block_h = if avatar.is_some() {
        AVATAR_SIZE + GAP_AVATAR_GREETING
    } else {
        0.0
    };

    let total = avatar_block_h
        + greeting_h as f64 + GAP_GREETING_CLOCK
        + clock_h as f64 + GAP_CLOCK_DATE
        + date_h as f64 + GAP_DATE_CARD
        + card_h
        + (if caps_visible { GAP_CARD_CAPS + caps_chip_h } else { 0.0 })
        + GAP_CAPS_STATUS + status_h;

    let cx = width as f64 / 2.0;
    let mut y = (height as f64 - total) / 2.0;

    // 5. Avatar.
    if let Some(av) = avatar {
        draw_avatar(&cr, cx, y + AVATAR_SIZE / 2.0, AVATAR_SIZE / 2.0, av)?;
        y += AVATAR_SIZE + GAP_AVATAR_GREETING;
    }

    // 6. Greeting.
    cr.set_source_rgb(MUTED.0, MUTED.1, MUTED.2);
    cr.move_to(cx - greeting_w as f64 / 2.0, y);
    pangocairo::functions::show_layout(&cr, &greeting_layout);
    y += greeting_h as f64 + GAP_GREETING_CLOCK;

    // 7. Clock.
    cr.set_source_rgb(TEXT.0, TEXT.1, TEXT.2);
    cr.move_to(cx - clock_w as f64 / 2.0, y);
    pangocairo::functions::show_layout(&cr, &clock_layout);
    y += clock_h as f64 + GAP_CLOCK_DATE;

    // 8. Date.
    cr.set_source_rgb(MUTED.0, MUTED.1, MUTED.2);
    cr.move_to(cx - date_w as f64 / 2.0, y);
    pangocairo::functions::show_layout(&cr, &date_layout);
    y += date_h as f64 + GAP_DATE_CARD;

    // 9. Card with optional shake offset + drop shadow.
    let shake_dx = shake_offset(seat);
    let card_x = cx - card_w / 2.0 + shake_dx;

    draw_card_with_shadow(&cr, card_x, y, card_w, card_h);

    // 10. Dots / placeholder pill.
    let band_y = y + CARD_PADDING_Y + DOTS_BAND_HEIGHT / 2.0;
    let visible_dots = seat.password.chars().count().min(MAX_VISIBLE_DOTS);

    if visible_dots > 0 {
        let total_dot_w =
            visible_dots as f64 * (DOT_RADIUS * 2.0 + DOT_SPACING) - DOT_SPACING;
        let mut dx = cx - total_dot_w / 2.0 + DOT_RADIUS + shake_dx;
        cr.set_source_rgb(TEXT.0, TEXT.1, TEXT.2);
        for _ in 0..visible_dots {
            cr.arc(dx, band_y, DOT_RADIUS, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
            dx += DOT_RADIUS * 2.0 + DOT_SPACING;
        }
    } else {
        cr.set_source_rgba(TEXT.0, TEXT.1, TEXT.2, 0.4);
        cr.rectangle(
            cx - PLACEHOLDER_PILL_W / 2.0 + shake_dx,
            band_y - PLACEHOLDER_PILL_H / 2.0,
            PLACEHOLDER_PILL_W,
            PLACEHOLDER_PILL_H,
        );
        cr.fill().ok();
    }

    y += card_h;

    // 11. Caps Lock chip.
    if caps_visible {
        y += GAP_CARD_CAPS;
        let chip = layout(&cr, "⇪ CAPS LOCK", FONT_CAPS_PT, true);
        let (cw, ch) = chip.pixel_size();
        let chip_x = cx - cw as f64 / 2.0;
        let pad_x = 14.0;
        let pad_y = 6.0;
        rounded_rect(
            &cr,
            chip_x - pad_x,
            y - pad_y / 2.0,
            cw as f64 + pad_x * 2.0,
            ch as f64 + pad_y * 2.0,
            10.0,
        );
        cr.set_source_rgba(WARN.0, WARN.1, WARN.2, 0.22);
        cr.fill_preserve().ok();
        cr.set_source_rgba(WARN.0, WARN.1, WARN.2, 0.65);
        cr.set_line_width(1.0);
        cr.stroke().ok();

        cr.set_source_rgb(WARN.0, WARN.1, WARN.2);
        cr.move_to(chip_x, y + pad_y / 2.0);
        pangocairo::functions::show_layout(&cr, &chip);
        y += caps_chip_h;
    }

    // 12. Status line.
    y += GAP_CAPS_STATUS;
    let status_text = seat.fail_message.clone().unwrap_or_else(|| {
        if visible_dots > 0 {
            "Enter ile giriş".to_string()
        } else {
            "🔒  Parolanızı yazın".to_string()
        }
    });
    let layout = layout(&cr, &status_text, FONT_STATUS_PT, false);
    let (sw, _) = layout.pixel_size();
    if seat.fail_message.is_some() {
        cr.set_source_rgb(FAIL.0, FAIL.1, FAIL.2);
    } else {
        cr.set_source_rgba(MUTED.0, MUTED.1, MUTED.2, 0.7);
    }
    cr.move_to(cx - sw as f64 / 2.0, y);
    pangocairo::functions::show_layout(&cr, &layout);

    surface.flush();
    Ok(())
}

fn layout(cr: &cairo::Context, text: &str, pt: i32, bold: bool) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    let mut desc = pango::FontDescription::new();
    desc.set_family(FONT_FAMILY);
    desc.set_size(pt * pango::SCALE);
    desc.set_weight(if bold {
        pango::Weight::Bold
    } else {
        pango::Weight::Normal
    });
    layout.set_font_description(Some(&desc));
    layout.set_text(text);
    layout
}

fn greeting_for(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Günaydın",
        12..=16 => "İyi günler",
        17..=20 => "İyi akşamlar",
        _ => "İyi geceler",
    }
}

fn display_name(user: &str) -> String {
    // Capitalise the first byte of the system name — "kenan" → "Kenan".
    let mut chars = user.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn shake_offset(seat: &SeatState) -> f64 {
    let Some(deadline) = seat.shake_until else {
        return 0.0;
    };
    let now = std::time::Instant::now();
    if now >= deadline {
        return 0.0;
    }
    let remaining = deadline.duration_since(now).as_secs_f64() * 1000.0;
    let progress = remaining / SHAKE_DURATION_MS as f64; // 1.0 → 0.0
    let envelope = progress.clamp(0.0, 1.0);
    // Decaying sine — disturbing then settled.
    let t = (SHAKE_DURATION_MS as f64 - remaining) / 1000.0;
    (t * SHAKE_FREQ_HZ * std::f64::consts::TAU).sin() * SHAKE_AMPLITUDE * envelope
}

fn draw_card_with_shadow(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64) {
    // Soft shadow: stack three increasingly faded, increasingly larger
    // rounded rects — cheap blur fake that reads convincingly.
    for (offset, alpha) in [(2.0, 0.18), (6.0, 0.12), (12.0, 0.07)] {
        let off: f64 = offset;
        let pad = off;
        rounded_rect(
            cr,
            x - pad,
            y + off,
            w + pad * 2.0,
            h + pad * 2.0,
            CARD_RADIUS + pad,
        );
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
        cr.fill().ok();
    }

    // Card surface.
    rounded_rect(cr, x, y, w, h, CARD_RADIUS);
    cr.set_source_rgba(1.0, 1.0, 1.0, CARD_ALPHA);
    cr.fill_preserve().ok();
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
    cr.stroke().ok();
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

fn draw_vignette(cr: &cairo::Context, w: i32, h: i32) {
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let radius = (cx * cx + cy * cy).sqrt();
    let pat = cairo::RadialGradient::new(cx, cy, radius * 0.5, cx, cy, radius);
    pat.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
    pat.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.35);
    cr.set_source(&pat).ok();
    cr.paint().ok();
}

fn draw_avatar(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    radius: f64,
    img: &image::RgbaImage,
) -> Result<()> {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let stride = iw * 4;

    // RGBA → premultiplied BGRA (cairo ARgb32 layout).
    let mut bgra: Vec<u8> = Vec::with_capacity((stride * ih) as usize);
    for px in img.chunks_exact(4) {
        let (r, g, b, a) = (px[0] as u16, px[1] as u16, px[2] as u16, px[3] as u16);
        let pm = |c: u16| ((c * a + 127) / 255) as u8;
        bgra.push(pm(b));
        bgra.push(pm(g));
        bgra.push(pm(r));
        bgra.push(a as u8);
    }
    let src = ImageSurface::create_for_data(bgra, Format::ARgb32, iw, ih, stride)?;

    cr.save()?;
    // Circular clip + draw + restore.
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    cr.clip();
    let scale = (radius * 2.0) / iw as f64;
    cr.translate(cx - radius, cy - radius);
    cr.scale(scale, scale);
    cr.set_source_surface(&src, 0.0, 0.0)?;
    cr.paint().ok();
    cr.restore()?;

    // Subtle ring around the avatar.
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    cr.set_line_width(AVATAR_RING_W);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.32);
    cr.stroke().ok();
    Ok(())
}

fn paint_wallpaper_cover(
    cr: &cairo::Context,
    wp: &image::RgbaImage,
    target_w: i32,
    target_h: i32,
) -> Result<()> {
    let (iw, ih) = (wp.width() as i32, wp.height() as i32);
    let stride = iw * 4;
    let len = (stride * ih) as usize;

    let mut bgra: Vec<u8> = Vec::with_capacity(len);
    for px in wp.chunks_exact(4) {
        let r = px[0] as u16;
        let g = px[1] as u16;
        let b = px[2] as u16;
        let a = px[3] as u16;
        let pm = |c: u16| ((c * a + 127) / 255) as u8;
        bgra.push(pm(b));
        bgra.push(pm(g));
        bgra.push(pm(r));
        bgra.push(a as u8);
    }

    let src = ImageSurface::create_for_data(bgra, Format::ARgb32, iw, ih, stride)?;

    let sx = target_w as f64 / iw as f64;
    let sy = target_h as f64 / ih as f64;
    let s = sx.max(sy);
    let scaled_w = iw as f64 * s;
    let scaled_h = ih as f64 * s;
    let offset_x = (target_w as f64 - scaled_w) / 2.0;
    let offset_y = (target_h as f64 - scaled_h) / 2.0;

    cr.save()?;
    cr.translate(offset_x, offset_y);
    cr.scale(s, s);
    cr.set_source_surface(&src, 0.0, 0.0)?;
    cr.paint().ok();
    cr.restore()?;

    Ok(())
}
