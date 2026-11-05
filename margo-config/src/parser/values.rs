//! Scalar value parsers shared by every line kind: colours, bezier curves,
//! CSV lists, booleans, numbers, percentages and durations.

use anyhow::{Context, Result, bail};
use tracing::warn;

use crate::types::*;

// ── Colour parsing ───────────────────────────────────────────────────────────

pub(super) fn parse_color(s: &str) -> Result<Rgba> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches('#');
    let hex = u32::from_str_radix(s, 16).context("invalid color")?;
    // Accept the CSS-style forms and reject everything else. The old
    // `len >= 8 ? RRGGBBAA : RRGGBB` fork silently mis-parsed a 5- or
    // 7-digit typo as RRGGBB (keeping the low 24 bits) — a wrong colour
    // with no error. Each shorthand nibble is doubled (`#f80` == `#ff8800`,
    // i.e. `nibble / 15.0` == `(nibble * 17) / 255.0`).
    let (r, g, b, a) = match s.len() {
        3 => (
            ((hex >> 8) & 0xf) as f32 / 15.0,
            ((hex >> 4) & 0xf) as f32 / 15.0,
            (hex & 0xf) as f32 / 15.0,
            1.0,
        ),
        4 => (
            ((hex >> 12) & 0xf) as f32 / 15.0,
            ((hex >> 8) & 0xf) as f32 / 15.0,
            ((hex >> 4) & 0xf) as f32 / 15.0,
            (hex & 0xf) as f32 / 15.0,
        ),
        6 => (
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
            1.0,
        ),
        8 => (
            ((hex >> 24) & 0xff) as f32 / 255.0,
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
        ),
        n => bail!("color must be 3, 4, 6, or 8 hex digits, got {n}"),
    };
    Ok(Rgba([r, g, b, a]))
}

// ── Bezier curve parsing ─────────────────────────────────────────────────────

pub(super) fn parse_bezier(s: &str) -> Result<BezierCurve> {
    let nums: Vec<f64> = s
        .split(',')
        .map(|t| t.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .context("invalid bezier curve")?;
    if nums.len() != 4 {
        bail!("bezier curve must have 4 values, got {}", nums.len());
    }
    Ok(BezierCurve([nums[0], nums[1], nums[2], nums[3]]))
}

// ── CSV helpers ──────────────────────────────────────────────────────────────

pub(super) fn split_csv(s: &str, max: usize) -> Vec<String> {
    s.splitn(max, ',').map(|p| p.trim().to_string()).collect()
}

/// Split comma-separated `key:value` pairs.
pub(super) fn split_csv_colon(s: &str) -> Vec<(String, String)> {
    s.split(',')
        .filter_map(|part| {
            let mut it = part.splitn(2, ':');
            let k = it.next()?.trim().to_string();
            let v = it.next().unwrap_or("").trim().to_string();
            Some((k, v))
        })
        .collect()
}

pub(super) fn parse_float_list(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|t| t.trim().parse().ok()).collect()
}

// ── Primitive parsers ────────────────────────────────────────────────────────

pub(super) fn parse_bool(s: &str) -> bool {
    // Case-insensitive: `True`, `YES`, `On` are the same as their
    // lowercase forms. Without this, a capitalised value silently
    // parses as `false` — the worst kind of config typo.
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(super) fn parse_i32(s: &str) -> i32 {
    parse_or_zero(s)
}

pub(super) fn parse_u32(s: &str) -> u32 {
    parse_or_zero(s)
}

pub(super) fn parse_f32(s: &str) -> f32 {
    parse_or_zero(s)
}

pub(super) fn parse_f64(s: &str) -> f64 {
    parse_or_zero(s)
}

/// Numeric parse that falls back to zero — loudly, so a malformed value
/// (`0,5` with a decimal comma, `12px`) shows up in the log as well as
/// in `mctl check-config`'s W003.
fn parse_or_zero<T: std::str::FromStr + Default>(s: &str) -> T {
    s.trim().parse().unwrap_or_else(|_| {
        warn!("`{}` is not a valid number — using 0", s.trim());
        T::default()
    })
}

/// Char-safe clamp for the fixed-width compositor keyword fields
/// (`animation_type_*` cap at 9 chars, `keymode` at 27). A raw byte slice
/// `val[..9]` panics whenever byte 9 lands in the middle of a multibyte
/// UTF-8 char (e.g. `animation_type_open = kayan日`) — and that panic
/// unwinds past the per-line reload guard and takes the whole compositor
/// down on `mctl reload`. Truncate on a char boundary instead, and warn
/// when a longer value is cut so a silent truncation isn't mistaken for a
/// working config. Identical to the old byte slice for ASCII values.
pub(super) fn clamp_keyword(val: &str, max: usize) -> String {
    let out: String = val.chars().take(max).collect();
    if val.chars().count() > max {
        warn!("config value {val:?} exceeds the {max}-char limit — using {out:?}");
    }
    out
}

pub(super) fn parse_bool_s(s: &str) -> bool {
    parse_bool(s)
}

pub(super) fn parse_i32_s(s: &str) -> i32 {
    parse_i32(s)
}

pub(super) fn parse_f32_s(s: &str) -> f32 {
    parse_f32(s)
}

pub(super) fn parse_u32_s(s: &str) -> u32 {
    parse_u32(s)
}

/// Detect a window-rule percent suffix (`50%`, ` 50 % `, `0.5%`)
/// and return the numeric value as a 0..=1 fraction. Returns
/// `None` for any input lacking a trailing `%` so callers can
/// fall back to absolute pixel parsing.
///
/// Values are clamped to `[0.0, 1.0]` — a typo like `200%` is
/// pinned to full size rather than overflowing the monitor.
pub(super) fn parse_percent_fraction(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    let stripped = trimmed.strip_suffix('%')?;
    let n: f32 = stripped.trim().parse().ok()?;
    Some((n / 100.0).clamp(0.0, 1.0))
}

/// Parse `HH:MM` or `HH:MM:SS` into seconds-since-midnight. Returns
/// 0 on malformed input — caller treats 0 as "unset" so a typo
/// silently falls back to the default schedule mode.
pub(super) fn parse_hms_to_seconds(s: &str) -> u32 {
    let h: u32;
    let mut m = 0u32;
    let mut sec = 0u32;
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return 0;
    }
    if let Ok(v) = parts[0].parse::<u32>() {
        h = v;
    } else {
        return 0;
    }
    if parts.len() >= 2 {
        if let Ok(v) = parts[1].parse::<u32>() {
            m = v;
        } else {
            return 0;
        }
    }
    if parts.len() == 3 {
        if let Ok(v) = parts[2].parse::<u32>() {
            sec = v;
        } else {
            return 0;
        }
    }
    if h >= 24 || m >= 60 || sec >= 60 {
        return 0;
    }
    h * 3600 + m * 60 + sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keyword_is_char_safe() {
        // Regression: a byte slice `val[..9]` panicked when byte 9 split a
        // multibyte char, crashing the compositor on `mctl reload`.
        // Truncation must happen on a char boundary, never mid-UTF-8.
        assert_eq!(clamp_keyword("kayan日günü", 9).chars().count(), 9);
        assert_eq!(clamp_keyword("日本語のアニメーション名", 4), "日本語の");
        // ASCII behaviour is identical to the old byte slice.
        assert_eq!(clamp_keyword("slidefromtop", 9), "slidefrom");
        // Values within the limit pass through untouched.
        assert_eq!(clamp_keyword("slide_in", 9), "slide_in");
        assert_eq!(clamp_keyword("", 9), "");
    }

    #[test]
    fn parse_bool_is_case_insensitive() {
        for t in ["1", "true", "TRUE", "True", "yes", "YES", "on", "On"] {
            assert!(super::parse_bool(t), "{t:?} should be true");
        }
        for f in ["0", "false", "FALSE", "no", "off", "", "enabled"] {
            assert!(!super::parse_bool(f), "{f:?} should be false");
        }
    }

    #[test]
    fn parse_color_forms_and_shorthands() {
        // Full forms (channel exprs mirror the impl for bit-exact f32).
        assert_eq!(
            super::parse_color("#ff8800").unwrap().0,
            [1.0, 136.0 / 255.0, 0.0, 1.0]
        );
        assert_eq!(
            super::parse_color("#00000000").unwrap().0,
            [0.0, 0.0, 0.0, 0.0]
        );
        // 3/4-digit shorthand doubles each nibble (`#f80` → `#ff8800`).
        assert_eq!(
            super::parse_color("#f80").unwrap().0,
            [1.0, 8.0 / 15.0, 0.0, 1.0]
        );
        assert_eq!(
            super::parse_color("#f808").unwrap().0,
            [1.0, 8.0 / 15.0, 0.0, 8.0 / 15.0]
        );
        // A stray 5- or 7-digit typo is an error, not a silent mis-parse.
        assert!(super::parse_color("#ff880").is_err());
        assert!(super::parse_color("#ff88000").is_err());
        assert!(super::parse_color("#nothex").is_err());
    }
}
