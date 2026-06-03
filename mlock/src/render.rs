//! Cairo + pango drawing for one lock frame.
//!
//! Centred auth column (top → bottom), every element config-gated:
//!   • optional round avatar
//!   • greeting line ("Good morning, Kenan")
//!   • clock + date pair
//!   • slim password capsule (shake + soft shadow) holding the dots
//!   • optional Caps Lock chip
//!   • status line — fail message + attempt count, or typing hint
//!   • power-action chips (F1/F2/F3)
//!
//! Absolutely-positioned extras (don't perturb the centred column):
//!   • top-right battery, top-left keyboard layout
//!   • bottom-centre live info — weather · notifications, and a
//!     now-playing line — published by the shell (see `sidecar`).

use anyhow::Result;
use cairo::{Format, ImageSurface};
use chrono::{Local, Timelike};

use crate::icons;
use crate::seat::SeatState;

const DIM_ALPHA: f64 = 0.55;

// Palette over the dimmed wallpaper. Read once from the shell's matugen
// output so the locker is tonally coherent with the rest of the desktop
// (DESIGN.md §0 calm / §1 "never hardcode colours") instead of a fixed
// scheme. Falls back to a calm neutral set when matugen hasn't run.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: (f64, f64, f64),     // surface — solid fallback behind wallpaper
    pub text: (f64, f64, f64),   // on-surface — dominant clock + headings
    pub muted: (f64, f64, f64),  // on-surface-variant — secondary text recedes
    pub accent: (f64, f64, f64), // primary — the single accent (input focus)
    pub danger: (f64, f64, f64), // error — failed auth
}

impl Palette {
    const FALLBACK: Self = Self {
        bg: (0.05, 0.05, 0.10),
        text: (0.96, 0.97, 0.98),
        muted: (0.78, 0.80, 0.86),
        accent: (0.96, 0.97, 0.98),
        danger: (0.95, 0.45, 0.43),
    };
}

/// The shell palette, loaded once per process (not per frame) and cached.
fn palette() -> &'static Palette {
    static PALETTE: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    PALETTE.get_or_init(|| read_palette().unwrap_or(Palette::FALLBACK))
}

// Avatar.
const AVATAR_SIZE: f64 = 84.0;
const AVATAR_RING_W: f64 = 2.0;

// Typography.
const FONT_FAMILY: &str = "Maple Mono NF, Noto Sans, sans";
const FONT_CLOCK_PT: i32 = 88;
const FONT_DATE_PT: i32 = 20;
const FONT_GREETING_PT: i32 = 18;
const FONT_STATUS_PT: i32 = 13;
const FONT_CAPS_PT: i32 = 12;
const FONT_INFO_PT: i32 = 13;

// Stack gaps — §0.8 spacing scale (4/8/12/16/24/32) so the centred stack
// keeps a single rhythm. Tighter than before so the composition reads as
// a calm column rather than a sprawled one.
const GAP_AVATAR_GREETING: f64 = 18.0;
const GAP_GREETING_CLOCK: f64 = 20.0;
const GAP_CLOCK_DATE: f64 = 6.0;
const GAP_DATE_INPUT: f64 = 32.0;
const GAP_INPUT_CAPS: f64 = 14.0;
const GAP_CAPS_STATUS: f64 = 12.0;

// Compact password input — a slim capsule, not the old 720 px slab. Sized
// to read as a single tidy field whatever the password length.
const INPUT_W: f64 = 300.0;
const INPUT_H: f64 = 46.0;
const INPUT_PAD_X: f64 = 22.0;

// Dots.
const DOT_RADIUS: f64 = 4.5;
const DOT_SPACING: f64 = 10.0;
const PLACEHOLDER_PILL_W: f64 = 120.0;
const PLACEHOLDER_PILL_H: f64 = 2.5;
const MAX_VISIBLE_DOTS: usize = 11;

// Shake animation.
const SHAKE_DURATION_MS: u64 = 400;
const SHAKE_AMPLITUDE: f64 = 10.0;
const SHAKE_FREQ_HZ: f64 = 14.0;

/// Matugen accent (`primary_color.base`). Kept for callers that only need
/// the accent (state.rs); the full palette lives in `palette()`.
pub fn matugen_accent() -> (f64, f64, f64) {
    palette().accent
}

/// Parse the whole shell palette from `$XDG_CACHE_HOME/margo/
/// mshell-colors.toml` (matugen output) so the locker tracks the wallpaper
/// theme. Hand-parsed to keep a TOML dependency out of the locker; any
/// missing key falls back to the neutral default for that role.
fn read_palette() -> Option<Palette> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    let toml = std::fs::read_to_string(dir.join("margo").join("mshell-colors.toml")).ok()?;
    let fb = Palette::FALLBACK;
    let bg = field_base(&toml, "background_color").unwrap_or(fb.bg);
    let text = field_bare(&toml, "text_color").unwrap_or(fb.text);
    let accent = field_base(&toml, "primary_color").unwrap_or(fb.accent);
    let danger = field_bare(&toml, "danger_color").unwrap_or(fb.danger);
    // Secondary text tier: on-surface pulled ~⅓ toward the surface so
    // metadata recedes without a second hue (DESIGN.md §1 fonts).
    let muted = mix(text, bg, 0.34);
    Some(Palette {
        bg,
        text,
        muted,
        accent,
        danger,
    })
}

/// `<key> … base = "#rrggbb"` — matugen inline table (background/primary).
fn field_base(toml: &str, key: &str) -> Option<(f64, f64, f64)> {
    let line = toml.lines().find(|l| l.trim_start().starts_with(key))?;
    let after = line.split("base").nth(1)?;
    let h = after.find('#')?;
    parse_hex6(&after[h + 1..].chars().take(6).collect::<String>())
}

/// `<key> = "#rrggbb"` — bare matugen string (text/danger/…).
fn field_bare(toml: &str, key: &str) -> Option<(f64, f64, f64)> {
    let line = toml.lines().find(|l| l.trim_start().starts_with(key))?;
    let h = line.find('#')?;
    parse_hex6(&line[h + 1..].chars().take(6).collect::<String>())
}

fn mix(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn parse_hex6(hex: &str) -> Option<(f64, f64, f64)> {
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

pub fn draw_lock_frame(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    stride: i32,
    seat: &SeatState,
    user: &str,
    wallpaper: Option<&image::RgbaImage>,
    avatar: Option<&image::RgbaImage>,
    accent: (f64, f64, f64),
    toggles: &crate::config::LockToggles,
    info: &crate::sidecar::LockInfo,
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
    let pal = palette();

    // 1. Wallpaper or solid fallback.
    if let Some(wp) = wallpaper {
        paint_wallpaper_cover(&cr, wp, width, height)?;
    } else {
        cr.set_source_rgb(pal.bg.0, pal.bg.1, pal.bg.2);
        cr.paint().ok();
    }

    // 2. Uniform dim.
    cr.set_source_rgba(0.0, 0.0, 0.0, DIM_ALPHA);
    cr.paint().ok();

    // 3. Vignette — radial fade darkens the edges, draws the eye to
    //    the centre.
    draw_vignette(&cr, width, height);

    // 4. Build the (config-gated) text layouts up-front so we can measure
    //    heights BEFORE laying out — the centred stack stays balanced no
    //    matter which elements the user turned off.
    let now = Local::now();
    let show_avatar = toggles.avatar && avatar.is_some();

    let greeting_layout = toggles.greeting.then(|| {
        let s = format!("{}, {}", greeting_for(now.hour()), display_name(user));
        layout(&cr, &s, FONT_GREETING_PT, false)
    });
    let clock_layout = layout(&cr, &now.format("%H:%M").to_string(), FONT_CLOCK_PT, true);
    let date_layout = toggles.date.then(|| {
        layout(
            &cr,
            &now.format("%A, %-d %B %Y").to_string(),
            FONT_DATE_PT,
            false,
        )
    });

    let (clock_w, clock_h) = clock_layout.pixel_size();
    let greeting_h = greeting_layout.as_ref().map_or(0, |l| l.pixel_size().1) as f64;
    let date_h = date_layout.as_ref().map_or(0, |l| l.pixel_size().1) as f64;

    let caps_visible = seat.caps_lock;
    let caps_chip_h = if caps_visible {
        FONT_CAPS_PT as f64 * 1.7 + 14.0
    } else {
        0.0
    };
    let status_h = FONT_STATUS_PT as f64 * 1.6;

    // Each present block contributes its own leading/trailing gap, so the
    // total matches the draw walk below exactly.
    let mut total = clock_h as f64 + GAP_DATE_INPUT + INPUT_H + GAP_CAPS_STATUS + status_h;
    if show_avatar {
        total += AVATAR_SIZE + GAP_AVATAR_GREETING;
    }
    if greeting_layout.is_some() {
        total += greeting_h + GAP_GREETING_CLOCK;
    }
    if date_layout.is_some() {
        total += GAP_CLOCK_DATE + date_h;
    }
    if caps_visible {
        total += GAP_INPUT_CAPS + caps_chip_h;
    }

    let cx = width as f64 / 2.0;
    let mut y = (height as f64 - total) / 2.0;

    // 5. Avatar.
    if show_avatar && let Some(av) = avatar {
        draw_avatar(
            &cr,
            cx,
            y + AVATAR_SIZE / 2.0,
            AVATAR_SIZE / 2.0,
            av,
            accent,
        )?;
        y += AVATAR_SIZE + GAP_AVATAR_GREETING;
    }

    // 6. Greeting.
    if let Some(gl) = &greeting_layout {
        cr.set_source_rgb(pal.muted.0, pal.muted.1, pal.muted.2);
        cr.move_to(cx - gl.pixel_size().0 as f64 / 2.0, y);
        pangocairo::functions::show_layout(&cr, gl);
        y += greeting_h + GAP_GREETING_CLOCK;
    }

    // 7. Clock.
    cr.set_source_rgb(pal.text.0, pal.text.1, pal.text.2);
    cr.move_to(cx - clock_w as f64 / 2.0, y);
    pangocairo::functions::show_layout(&cr, &clock_layout);
    y += clock_h as f64;

    // 8. Date.
    if let Some(dl) = &date_layout {
        y += GAP_CLOCK_DATE;
        cr.set_source_rgb(pal.muted.0, pal.muted.1, pal.muted.2);
        cr.move_to(cx - dl.pixel_size().0 as f64 / 2.0, y);
        pangocairo::functions::show_layout(&cr, dl);
        y += date_h;
    }
    y += GAP_DATE_INPUT;

    // 9. Compact password capsule with shake offset + soft shadow.
    let shake_dx = shake_offset(seat);
    let input_x = cx - INPUT_W / 2.0 + shake_dx;

    // On a failed attempt the border escalates to the danger tone alongside
    // the shake + red status line (DESIGN.md §2 severity ladder).
    let border = if seat.fail_message.is_some() {
        pal.danger
    } else {
        accent
    };
    draw_input_pill(&cr, input_x, y, INPUT_W, INPUT_H, border);

    // 10. Dots / placeholder, centred in the capsule. The visible-dot count
    //     is capped to what fits inside the pill's padding.
    let band_y = y + INPUT_H / 2.0;
    let fit =
        (((INPUT_W - INPUT_PAD_X * 2.0) + DOT_SPACING) / (DOT_RADIUS * 2.0 + DOT_SPACING)) as usize;
    let cap = MAX_VISIBLE_DOTS.min(fit.max(1));
    let visible_dots = seat.password.chars().count().min(cap);

    if visible_dots > 0 {
        let total_dot_w = visible_dots as f64 * (DOT_RADIUS * 2.0 + DOT_SPACING) - DOT_SPACING;
        let mut dx = cx - total_dot_w / 2.0 + DOT_RADIUS + shake_dx;
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        for _ in 0..visible_dots {
            cr.arc(dx, band_y, DOT_RADIUS, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
            dx += DOT_RADIUS * 2.0 + DOT_SPACING;
        }
    } else {
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.35);
        rounded_rect(
            &cr,
            cx - PLACEHOLDER_PILL_W / 2.0 + shake_dx,
            band_y - PLACEHOLDER_PILL_H / 2.0,
            PLACEHOLDER_PILL_W,
            PLACEHOLDER_PILL_H,
            PLACEHOLDER_PILL_H / 2.0,
        );
        cr.fill().ok();
    }

    y += INPUT_H;

    // 11. Caps Lock chip — drawn caps glyph + label.
    if caps_visible {
        y += GAP_INPUT_CAPS;
        let chip = layout(&cr, "CAPS LOCK", FONT_CAPS_PT, true);
        let (cw, ch) = chip.pixel_size();
        let icon_w = ch as f64 * 1.05;
        let icon_gap = 8.0;
        let pad_x = 14.0;
        let pad_y = 6.0;
        let content_w = icon_w + icon_gap + cw as f64;
        let chip_x = cx - content_w / 2.0;
        rounded_rect(
            &cr,
            chip_x - pad_x,
            y - pad_y / 2.0,
            content_w + pad_x * 2.0,
            ch as f64 + pad_y * 2.0,
            10.0,
        );
        cr.set_source_rgba(pal.accent.0, pal.accent.1, pal.accent.2, 0.22);
        cr.fill_preserve().ok();
        cr.set_source_rgba(pal.accent.0, pal.accent.1, pal.accent.2, 0.65);
        cr.set_line_width(1.0);
        cr.stroke().ok();

        icons::caps(
            &cr,
            chip_x + icon_w / 2.0,
            y + ch as f64 / 2.0,
            icon_w,
            pal.accent,
            0.95,
        );
        cr.set_source_rgb(pal.accent.0, pal.accent.1, pal.accent.2);
        cr.move_to(chip_x + icon_w + icon_gap, y + pad_y / 2.0);
        pangocairo::functions::show_layout(&cr, &chip);
        y += caps_chip_h;
    }

    // 12. Status line (fail / hint). The empty-password hint leads with a
    //     drawn padlock instead of an emoji.
    y += GAP_CAPS_STATUS;
    let is_lock_hint = seat.fail_message.is_none() && visible_dots == 0;
    let status_text = seat.fail_message.clone().unwrap_or_else(|| {
        if visible_dots > 0 {
            "Press Enter to unlock".to_string()
        } else {
            "Type your password".to_string()
        }
    });
    let layout_status = layout(&cr, &status_text, FONT_STATUS_PT, false);
    let (sw, sh) = layout_status.pixel_size();
    if is_lock_hint {
        let icon_w = sh as f64 * 0.95;
        let icon_gap = 8.0;
        let total = icon_w + icon_gap + sw as f64;
        let x0 = cx - total / 2.0;
        icons::lock(
            &cr,
            x0 + icon_w / 2.0,
            y + sh as f64 / 2.0,
            icon_w,
            pal.muted,
            0.7,
        );
        cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.7);
        cr.move_to(x0 + icon_w + icon_gap, y);
        pangocairo::functions::show_layout(&cr, &layout_status);
    } else {
        if seat.fail_message.is_some() {
            cr.set_source_rgb(pal.danger.0, pal.danger.1, pal.danger.2);
        } else {
            cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.7);
        }
        cr.move_to(cx - sw as f64 / 2.0, y);
        pangocairo::functions::show_layout(&cr, &layout_status);
    }
    y += sh as f64 + 12.0;

    // 13. Power-confirm banner OR F-key hint row.
    if let Some((action, _)) = seat.power_confirm {
        let msg = format!("Press the F-key again to confirm: {}", action.label());
        let layout_confirm = layout(&cr, &msg, FONT_STATUS_PT, true);
        let (cw, _) = layout_confirm.pixel_size();
        cr.set_source_rgb(pal.danger.0, pal.danger.1, pal.danger.2);
        cr.move_to(cx - cw as f64 / 2.0, y);
        pangocairo::functions::show_layout(&cr, &layout_confirm);
    } else {
        // Power-action chips: a drawn icon + key + label, laid out in a
        // centred row, instead of one dim line of plain text.
        type IconFn = fn(&cairo::Context, f64, f64, f64, (f64, f64, f64), f64);
        let chips: [(IconFn, &str); 3] = [
            (icons::power, "F1  Shut down"),
            (icons::restart, "F2  Restart"),
            (icons::moon, "F3  Suspend"),
        ];
        let icon_w = FONT_CAPS_PT as f64 * 1.15;
        let icon_gap = 7.0;
        let pad_x = 12.0;
        let pad_y = 6.0;
        let chip_gap = 10.0;

        let measured: Vec<(pango::Layout, f64, f64)> = chips
            .iter()
            .map(|(_, label)| {
                let l = layout(&cr, label, FONT_CAPS_PT, false);
                let (lw, lh) = l.pixel_size();
                (l, icon_w + icon_gap + lw as f64, lh as f64)
            })
            .collect();
        let total_w: f64 = measured.iter().map(|m| m.1 + pad_x * 2.0).sum::<f64>()
            + chip_gap * (chips.len() as f64 - 1.0);
        let chip_h = measured.iter().map(|m| m.2).fold(0.0_f64, f64::max) + pad_y * 2.0;

        let mut x = cx - total_w / 2.0;
        for ((icon_fn, _), m) in chips.iter().zip(measured.iter()) {
            let chip_w = m.1 + pad_x * 2.0;
            let icy = y + chip_h / 2.0;
            rounded_rect(&cr, x, y, chip_w, chip_h, 10.0);
            cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.10);
            cr.fill().ok();
            icon_fn(&cr, x + pad_x + icon_w / 2.0, icy, icon_w, pal.muted, 0.85);
            cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.78);
            cr.move_to(x + pad_x + icon_w + icon_gap, icy - m.2 / 2.0);
            pangocairo::functions::show_layout(&cr, &m.0);
            x += chip_w + chip_gap;
        }
    }

    // 14. Top-right battery indicator (laptops only).
    if toggles.battery
        && let Some(bat) = seat.battery
    {
        draw_battery(&cr, width as f64 - 32.0, 28.0, bat);
    }

    // 15. Top-left keyboard layout (multi-layout setups only).
    //     Absolutely positioned like the battery, so it never
    //     perturbs the centred stack's height maths.
    if toggles.layout
        && let Some(name) = seat.layout_name()
    {
        let lay = layout(&cr, &name.to_uppercase(), FONT_CAPS_PT, true);
        cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.8);
        cr.move_to(32.0, 24.0);
        pangocairo::functions::show_layout(&cr, &lay);
    }

    // 16. Bottom-centre info cluster — live desktop context published by
    //     the shell (notifications / weather / now-playing). Absolutely
    //     positioned at the bottom edge so it never disturbs the centred
    //     auth column; each line is gated by both its config toggle and
    //     whether there's anything to show.
    draw_info_cluster(&cr, cx, height as f64, toggles, info, pal);

    surface.flush();
    Ok(())
}

/// Draw the bottom-centre context lines, stacked upward from the bottom
/// edge: a now-playing line (title — artist) above a combined
/// weather · notifications line.
fn draw_info_cluster(
    cr: &cairo::Context,
    cx: f64,
    height: f64,
    toggles: &crate::config::LockToggles,
    info: &crate::sidecar::LockInfo,
    pal: &Palette,
) {
    let mut bits: Vec<String> = Vec::new();
    if toggles.weather && !info.weather.is_empty() {
        bits.push(info.weather.clone());
    }
    if toggles.notifications && info.notifications > 0 {
        bits.push(if info.notifications == 1 {
            "1 notification".to_string()
        } else {
            format!("{} notifications", info.notifications)
        });
    }
    let context_line = bits.join("    ·    ");

    let now_playing = if toggles.media && info.has_media() {
        Some(
            match (info.media_title.is_empty(), info.media_artist.is_empty()) {
                (false, false) => format!(
                    "{} — {}",
                    trunc(&info.media_title),
                    trunc(&info.media_artist)
                ),
                (false, true) => trunc(&info.media_title),
                _ => trunc(&info.media_artist),
            },
        )
    } else {
        None
    };

    // Walk upward from the bottom margin.
    let mut baseline = height - 36.0;
    if !context_line.is_empty() {
        let l = layout(cr, &context_line, FONT_INFO_PT, false);
        let (lw, lh) = l.pixel_size();
        baseline -= lh as f64;
        cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, 0.85);
        cr.move_to(cx - lw as f64 / 2.0, baseline);
        pangocairo::functions::show_layout(cr, &l);
        baseline -= 8.0;
    }
    if let Some(np) = now_playing {
        let icon_w = FONT_INFO_PT as f64 * 1.1;
        let gap = 7.0;
        let l = layout(cr, &np, FONT_INFO_PT, true);
        let (lw, lh) = l.pixel_size();
        baseline -= lh as f64;
        let total_w = icon_w + gap + lw as f64;
        let x0 = cx - total_w / 2.0;
        icons::note(
            cr,
            x0 + icon_w / 2.0,
            baseline + lh as f64 / 2.0,
            icon_w,
            pal.accent,
            0.9,
        );
        cr.set_source_rgba(pal.text.0, pal.text.1, pal.text.2, 0.9);
        cr.move_to(x0 + icon_w + gap, baseline);
        pangocairo::functions::show_layout(cr, &l);
    }
}

/// Clamp a metadata string so a long title can't overrun the screen.
fn trunc(s: &str) -> String {
    const MAX: usize = 42;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn draw_battery(cr: &cairo::Context, right_x: f64, top_y: f64, bat: crate::battery::BatteryInfo) {
    let pal = palette();
    let color = if bat.percent <= 15 && !bat.charging {
        pal.danger
    } else {
        pal.muted
    };

    let text = format!("{}%", bat.percent);
    let layout = pangocairo::functions::create_layout(cr);
    let mut desc = pango::FontDescription::new();
    desc.set_family(FONT_FAMILY);
    desc.set_size(FONT_CAPS_PT * pango::SCALE);
    desc.set_weight(pango::Weight::Medium);
    layout.set_font_description(Some(&desc));
    layout.set_text(&text);
    let (tw, th) = layout.pixel_size();

    // Drawn battery glyph + percent, right-aligned to `right_x`.
    let icon_w = 24.0;
    let gap = 8.0;
    let x0 = right_x - (icon_w + gap + tw as f64);
    let icy = top_y + th as f64 / 2.0;

    icons::battery(
        cr,
        x0 + icon_w / 2.0,
        icy,
        icon_w,
        bat.percent as f64 / 100.0,
        color,
        0.92,
    );
    if bat.charging {
        icons::bolt(cr, x0 + icon_w * 0.42, icy, icon_w * 0.46, pal.accent, 1.0);
    }

    cr.set_source_rgba(color.0, color.1, color.2, 0.92);
    cr.move_to(x0 + icon_w + gap, top_y);
    pangocairo::functions::show_layout(cr, &layout);
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
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Good night",
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

/// The slim password capsule — a frosted full-radius pill with a soft
/// shadow and an accent (or danger) hairline border.
fn draw_input_pill(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, border: (f64, f64, f64)) {
    let r = h / 2.0;
    // Soft shadow — two faded, slightly larger pills.
    for (offset, alpha) in [(1.5, 0.16), (5.0, 0.09)] {
        let off: f64 = offset;
        rounded_rect(cr, x - off, y + off, w + off * 2.0, h + off * 2.0, r + off);
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
        cr.fill().ok();
    }

    // Frosted fill, tinted toward the theme's on-surface tone so the field
    // inherits the matugen palette's warmth (DESIGN.md §0.1 / §14).
    rounded_rect(cr, x, y, w, h, r);
    let frost = palette().text;
    cr.set_source_rgba(frost.0, frost.1, frost.2, 0.14);
    cr.fill_preserve().ok();
    // Accent border — always visible so the theme reads even before typing.
    cr.set_line_width(1.5);
    cr.set_source_rgba(border.0, border.1, border.2, 0.7);
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
    accent: (f64, f64, f64),
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

    // Accent ring around the avatar — matugen theme cue.
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    cr.set_line_width(AVATAR_RING_W);
    cr.set_source_rgba(accent.0, accent.1, accent.2, 0.75);
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
