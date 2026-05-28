//! Lock-screen background choice, read from `~/.config/margo/mlock.conf`.
//!
//! Kept hand-parsed (like the matugen palette in `render.rs`) so the
//! locker stays free of the shell's config crate — it must work even if
//! the shell is misconfigured. A **missing file means Wallpaper mode**,
//! so the prior behaviour is preserved on a clean install.
//!
//! ```conf
//! background = wallpaper      # or: color | image
//! background_color = #1e1e2e  # used when background = color
//! background_image = ~/Pictures/lock.jpg   # used when background = image
//! ```

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BgMode {
    /// The active output's desktop wallpaper (current behaviour).
    Wallpaper,
    /// A flat solid colour.
    Color,
    /// A specific image file, independent of the desktop wallpaper.
    Image,
}

pub struct BgConfig {
    pub mode: BgMode,
    pub color: (f64, f64, f64),
    pub image: Option<PathBuf>,
}

impl Default for BgConfig {
    fn default() -> Self {
        Self {
            mode: BgMode::Wallpaper,
            color: (0.05, 0.05, 0.10),
            image: None,
        }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("mlock.conf")
}

/// Read the background config; any parse failure or missing file yields
/// the Wallpaper default.
pub fn read() -> BgConfig {
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return BgConfig::default();
    };
    let mut cfg = BgConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        match key {
            "background" => {
                cfg.mode = match val {
                    "color" => BgMode::Color,
                    "image" => BgMode::Image,
                    _ => BgMode::Wallpaper,
                }
            }
            "background_color" => {
                if let Some(c) = parse_hex6(val) {
                    cfg.color = c;
                }
            }
            "background_image" if !val.is_empty() => {
                cfg.image = Some(expand_home(val));
            }
            _ => {}
        }
    }
    cfg
}

fn parse_hex6(s: &str) -> Option<(f64, f64, f64)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    }
}
