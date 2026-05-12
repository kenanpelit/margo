//! TOML config for midle. RUNE (stasis's native config language) is
//! powerful but adding a separate parser for one daemon is hard to
//! justify when serde+toml works fine.
//!
//! Example:
//! ```toml
//! [settings]
//! notify_on_unpause = true
//!
//! [[step]]
//! name    = "dim"
//! timeout = "5m"
//! command = "brightnessctl --save && brightnessctl set 20%"
//! resume_command = "brightnessctl --restore"
//!
//! [[step]]
//! name    = "lock"
//! timeout = "10m"
//! command = "mlock"
//!
//! [[step]]
//! name    = "suspend"
//! timeout = "15m"
//! command = "systemctl suspend"
//! ```

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,

    #[serde(default, rename = "step")]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// When `pause <duration>` expires, send a notify-send pulse.
    pub notify_on_unpause: bool,
    /// Per-step notification — if a step has `notify = true` it sends
    /// `notify-send -a midle <name>` before firing the command.
    pub notify_before_action: bool,
    /// Regex list of process command-lines (`/proc/<pid>/cmdline`)
    /// that, when matched by ANY running process, inhibit idle
    /// firing. E.g. `["mpv", "vlc", r"firefox.*", r"steam_app_.*"]`.
    pub inhibit_apps: Vec<String>,
    /// How often to scan `/proc` for inhibit_apps matches.
    #[serde(deserialize_with = "deserialize_duration", default = "default_scan")]
    pub inhibit_scan_interval: Duration,
    /// Watch PipeWire/PulseAudio sink inputs — when any stream is
    /// RUNNING, inhibit idle.
    pub monitor_media: bool,
    /// Shell command run immediately when logind announces
    /// `PrepareForSleep(true)` (system is about to suspend).
    /// Common use: pre-emptively `mlock` so the screen is locked
    /// before the lid falls / suspend happens.
    pub prepare_sleep_command: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notify_on_unpause: false,
            notify_before_action: false,
            inhibit_apps: Vec::new(),
            inhibit_scan_interval: default_scan(),
            monitor_media: false,
            prepare_sleep_command: None,
        }
    }
}

fn default_scan() -> Duration {
    Duration::from_secs(5)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub name: String,
    /// Duration string: `30s`, `5m`, `1h`. Plain integer is seconds.
    #[serde(deserialize_with = "deserialize_duration")]
    pub timeout: Duration,
    /// Shell command run when the timeout fires.
    pub command: String,
    /// Optional command run when the user becomes active again.
    #[serde(default)]
    pub resume_command: Option<String>,
    /// If `true` and `settings.notify_before_action`, fire a
    /// notify-send right before the command.
    #[serde(default)]
    pub notify: bool,
}

pub fn default_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("midle").join("config.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("midle").join("config.toml");
    }
    PathBuf::from("/etc/midle/config.toml")
}

pub fn load(path: Option<&Path>) -> Result<Config> {
    let owned;
    let resolved: &Path = match path {
        Some(p) => p,
        None => {
            owned = default_path();
            &owned
        }
    };

    if !resolved.exists() {
        // Allow no-config startup — empty plan, daemon stays idle-clean.
        tracing::warn!(
            path = %resolved.display(),
            "config file missing — running with empty plan"
        );
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(resolved)
        .with_context(|| format!("read config: {}", resolved.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .with_context(|| format!("parse config: {}", resolved.display()))?;
    Ok(cfg)
}

pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty duration"));
    }

    let (digits, suffix) = match s.chars().position(|c| !c.is_ascii_digit() && c != '.') {
        Some(idx) => (&s[..idx], &s[idx..]),
        None => (s, "s"),
    };

    let n: f64 = digits
        .parse()
        .map_err(|_| anyhow!("not a number: {digits:?}"))?;
    let secs = match suffix.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => n * 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => n * 3600.0,
        other => return Err(anyhow!("unknown duration unit: {other:?}")),
    };
    if secs < 0.0 || !secs.is_finite() {
        return Err(anyhow!("invalid duration: {s}"));
    }
    Ok(Duration::from_secs_f64(secs))
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let s = String::deserialize(deserializer)?;
    parse_duration(&s).map_err(D::Error::custom)
}
