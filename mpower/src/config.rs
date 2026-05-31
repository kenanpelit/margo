//! mpower config — `~/.config/margo/mpower.toml`.
//!
//! Single source of truth shared between the `mpower` daemon and the shell's
//! Settings → Power → Automatic Power Profile section (`mshell-settings`
//! depends on this crate for the struct, so the two never drift).
//!
//! `#[serde(default)]` on the struct means a missing or partial file is
//! filled from [`Config::default`] — so hand-edited fragments and
//! version-upgrades that add a field both stay valid. The daemon re-reads
//! this file every tick, so edits (from the settings page or by hand) go
//! live without a restart.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Every mpower knob. All durations are seconds; all loads are whole percent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Master switch. When false the daemon idles (never changes the profile).
    pub enabled: bool,
    /// How often the daemon samples + decides, in seconds.
    pub tick_seconds: u32,

    // ── AC policy: CPU-load thresholds (busy %) ──────────────────────────
    /// Switch to **performance** when the aggregate CPU busy% reaches this…
    pub high_avg_percent: u32,
    /// …or when the single hottest core reaches this.
    pub high_max_percent: u32,
    /// Switch back to **balanced** when aggregate busy% is at or below this…
    pub low_avg_percent: u32,
    /// …and the hottest core is at or below this.
    pub low_max_percent: u32,
    /// Consecutive high-load samples required before going to performance.
    pub high_streak: u32,
    /// Consecutive low-load samples required before dropping to balanced.
    pub low_streak: u32,
    /// Minimum seconds between profile changes (anti-flap).
    pub cooldown_seconds: u32,

    // ── Battery policy ───────────────────────────────────────────────────
    /// On battery, drop to **power-saver** at or below this charge %.
    /// `0` disables it (battery stays on balanced).
    pub battery_saver_below: u32,

    // ── Feedback ─────────────────────────────────────────────────────────
    /// Emit a desktop notification on each profile change.
    pub notify: bool,
}

impl Default for Config {
    fn default() -> Self {
        // Defaults mirror the retired `ppp-auto-profile` script so behaviour
        // is unchanged out of the box, plus the new low-battery power-saver.
        Self {
            enabled: true,
            tick_seconds: 5,
            high_avg_percent: 35,
            high_max_percent: 85,
            low_avg_percent: 18,
            low_max_percent: 70,
            high_streak: 2,
            low_streak: 3,
            cooldown_seconds: 20,
            battery_saver_below: 20,
            notify: false,
        }
    }
}

impl Config {
    /// Read `mpower.toml`, falling back to [`Config::default`] when the file
    /// is missing or unparseable (never panics — a bad edit must not brick
    /// the daemon).
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Serialise to `mpower.toml`, creating the parent dir if needed.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)?;
        let doc = format!(
            "# mpower — automatic power-profile manager.\n\
             # Managed by mpower + Settings → Power → Automatic Power Profile.\n\
             # Hand-edits are honoured; mpower re-reads this file every tick.\n\n{body}"
        );
        std::fs::write(&path, doc)?;
        Ok(())
    }
}

/// Absolute path to `mpower.toml` under the margo config dir, honouring
/// `XDG_CONFIG_HOME` then `HOME`.
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config")
        });
    base.join("margo").join("mpower.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_yields_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_file_fills_missing_from_default() {
        // Only one key set; everything else must come from Default.
        let cfg: Config = toml::from_str("high_avg_percent = 50\n").unwrap();
        assert_eq!(cfg.high_avg_percent, 50);
        assert_eq!(cfg.low_avg_percent, Config::default().low_avg_percent);
        assert!(cfg.enabled);
    }

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config {
            enabled: false,
            tick_seconds: 10,
            battery_saver_below: 0,
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }
}
