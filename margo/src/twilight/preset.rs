//! Sunsetr-compatible preset + schedule loader.
//!
//! Sunsetr's preset model:
//!   * `~/.config/sunsetr/presets/<name>/sunsetr.toml` — each
//!     preset pins `static_temp` (Kelvin) and `static_gamma`
//!     (percent). Anything else in the TOML is ignored (we only
//!     care about the colour-temp targets).
//!   * `~/.config/sunsetr/schedule.conf` — line-based,
//!     `HH:MM PRESET_NAME` per line. Sunsetr drives this with
//!     systemd timers; we instead read it directly and
//!     interpolate.
//!
//! Loading is best-effort: missing files / unparseable lines log
//! a warning and produce an empty schedule. The caller falls back
//! to neutral gamma when the schedule is empty.

use std::path::{Path, PathBuf};

/// One scheduled colour-temp sample. Sorted by `time_sec` at load
/// time so the tick logic can binary-search.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledPreset {
    /// Seconds since local midnight. Range 0..86400.
    pub time_sec: u32,
    /// Kelvin temperature this preset pins to.
    pub temp_k: u32,
    /// Brightness percentage this preset pins to.
    pub gamma_pct: u32,
}

/// All presets loaded from disk, in chronological order.
#[derive(Debug, Default, Clone)]
pub struct ScheduleData {
    pub entries: Vec<ScheduledPreset>,
}

impl ScheduleData {
    /// Load presets + schedule from `dir`. `dir` is expected to
    /// hold `schedule.conf` and a `presets/` subdirectory.
    ///
    /// Empty `dir` ⇒ default sunsetr location
    /// (`$XDG_CONFIG_HOME/sunsetr` or `~/.config/sunsetr`). Tilde
    /// is expanded.
    pub fn load(dir: &str) -> Self {
        let root = resolve_dir(dir);
        let schedule_path = root.join("schedule.conf");
        let presets_dir = root.join("presets");

        let schedule = match std::fs::read_to_string(&schedule_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    path = %schedule_path.display(),
                    error = %e,
                    "twilight schedule: schedule.conf not readable; schedule empty"
                );
                return Self::default();
            }
        };

        let mut entries: Vec<ScheduledPreset> = Vec::new();
        for (lineno, line) in schedule.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let Some(time_str) = parts.next() else { continue };
            let Some(name) = parts.next() else { continue };
            let Some(time_sec) = parse_hhmm(time_str) else {
                tracing::warn!(
                    path = %schedule_path.display(),
                    line = lineno + 1,
                    "twilight schedule: bad HH:MM {time_str:?}; skipping"
                );
                continue;
            };

            let preset_path = presets_dir.join(name).join("sunsetr.toml");
            match load_preset_file(&preset_path) {
                Some((temp_k, gamma_pct)) => entries.push(ScheduledPreset {
                    time_sec,
                    temp_k,
                    gamma_pct,
                }),
                None => tracing::warn!(
                    path = %preset_path.display(),
                    "twilight schedule: preset {name:?} not loadable; skipping line {}",
                    lineno + 1
                ),
            }
        }

        entries.sort_by_key(|e| e.time_sec);
        Self { entries }
    }

    /// Look up the sample to apply at `now_sec` (seconds since
    /// local midnight). Returns `None` when the schedule is empty;
    /// otherwise returns an interpolated `(temp_k, gamma_pct)` in
    /// mired space (temp) + linear (gamma) between the bracketing
    /// presets. Wraps at midnight — the last preset of the day
    /// blends into the first preset of the next day across the
    /// wrap boundary.
    pub fn sample(&self, now_sec: u32) -> Option<(u32, u32)> {
        if self.entries.is_empty() {
            return None;
        }
        if self.entries.len() == 1 {
            let p = self.entries[0];
            return Some((p.temp_k, p.gamma_pct));
        }

        // Find the latest entry ≤ now. If now is before the first
        // entry, the "current" is the last entry of the previous
        // day (i.e. `entries.last()`).
        let (current, next, span) = {
            let idx_after = self.entries.iter().position(|p| p.time_sec > now_sec);
            match idx_after {
                Some(0) => {
                    // Before the first entry. Wrap: current = last
                    // of yesterday, next = first of today.
                    let current = *self.entries.last().unwrap();
                    let next = self.entries[0];
                    let span = (24 * 3600 - current.time_sec) + next.time_sec;
                    (current, next, span)
                }
                Some(i) => {
                    let current = self.entries[i - 1];
                    let next = self.entries[i];
                    let span = next.time_sec - current.time_sec;
                    (current, next, span)
                }
                None => {
                    // After the last entry. Wrap forward: current
                    // = last of today, next = first of tomorrow.
                    let current = *self.entries.last().unwrap();
                    let next = self.entries[0];
                    let span = (24 * 3600 - current.time_sec) + next.time_sec;
                    (current, next, span)
                }
            }
        };

        let elapsed = if next.time_sec > current.time_sec {
            now_sec - current.time_sec
        } else {
            // Wrap case
            if now_sec >= current.time_sec {
                now_sec - current.time_sec
            } else {
                (24 * 3600 - current.time_sec) + now_sec
            }
        };
        let progress = if span == 0 {
            0.0
        } else {
            (elapsed as f32 / span as f32).clamp(0.0, 1.0)
        };

        let temp_k = lerp_mired(current.temp_k, next.temp_k, progress);
        let gamma_pct = lerp_linear(current.gamma_pct, next.gamma_pct, progress);
        Some((temp_k, gamma_pct))
    }
}

fn resolve_dir(dir: &str) -> PathBuf {
    let raw = if dir.trim().is_empty() {
        default_dir()
    } else {
        expand_tilde(dir)
    };
    raw
}

fn default_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("sunsetr");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("sunsetr");
    }
    PathBuf::from(".config/sunsetr")
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(stripped) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    PathBuf::from(s)
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h >= 24 || m >= 60 {
        return None;
    }
    Some(h * 3600 + m * 60)
}

/// Pull `static_temp` and `static_gamma` out of a sunsetr-style
/// TOML preset file. We don't pull in a full TOML parser for this
/// — the format is tiny and stable, so two `key = value` line
/// matchers cover it.
fn load_preset_file(path: &Path) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut temp_k: Option<u32> = None;
    let mut gamma_pct: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches('"');
        match key {
            "static_temp" => {
                if let Ok(n) = val.parse::<u32>() {
                    temp_k = Some(n.clamp(1000, 25000));
                }
            }
            "static_gamma" => {
                if let Ok(n) = val.parse::<u32>() {
                    gamma_pct = Some(n.clamp(10, 200));
                }
            }
            _ => {}
        }
    }
    Some((temp_k?, gamma_pct?))
}

fn lerp_mired(from_k: u32, to_k: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let m_from = 1_000_000.0 / from_k.max(1) as f32;
    let m_to = 1_000_000.0 / to_k.max(1) as f32;
    let m = m_from + (m_to - m_from) * t;
    (1_000_000.0 / m.max(1.0)).round() as u32
}

fn lerp_linear(from: u32, to: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    (from as f32 + (to as f32 - from as f32) * t).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(time_sec: u32, temp: u32, gamma: u32) -> ScheduledPreset {
        ScheduledPreset {
            time_sec,
            temp_k: temp,
            gamma_pct: gamma,
        }
    }

    #[test]
    fn empty_schedule_returns_none() {
        let s = ScheduleData::default();
        assert!(s.sample(0).is_none());
    }

    #[test]
    fn single_entry_constant() {
        let s = ScheduleData {
            entries: vec![pre(8 * 3600, 4000, 95)],
        };
        for h in 0..24 {
            let r = s.sample(h * 3600).unwrap();
            assert_eq!(r.0, 4000);
            assert_eq!(r.1, 95);
        }
    }

    #[test]
    fn between_two_entries_interpolates() {
        // 08:00 → 6000K/100%, 20:00 → 3000K/90%.
        let s = ScheduleData {
            entries: vec![pre(8 * 3600, 6000, 100), pre(20 * 3600, 3000, 90)],
        };
        // 14:00 = midpoint
        let (t, g) = s.sample(14 * 3600).unwrap();
        // Mired midpoint of 6000K/3000K ≈ 4000K.
        assert!(
            (3900..=4100).contains(&t),
            "midpoint temp = {t}, want ~4000K"
        );
        assert_eq!(g, 95);
    }

    #[test]
    fn wrap_around_midnight() {
        // Last preset 22:00 → 2500K/85%; first preset 06:00 → 6500K/100%.
        // 02:00 should be mid-wrap.
        let s = ScheduleData {
            entries: vec![pre(6 * 3600, 6500, 100), pre(22 * 3600, 2500, 85)],
        };
        let (t, _) = s.sample(2 * 3600).unwrap();
        // Mired midpoint of 2500K/6500K ≈ 3611K.
        assert!(
            (3300..=3900).contains(&t),
            "wrap midpoint temp = {t}, want ~3600K"
        );
    }

    #[test]
    fn parse_hhmm_basics() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("06:30"), Some(6 * 3600 + 30 * 60));
        assert_eq!(parse_hhmm("23:59"), Some(23 * 3600 + 59 * 60));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("xx"), None);
    }
}
