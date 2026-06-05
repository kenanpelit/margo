//! `~/.config/margo/mkeys.toml` — written by mshell's On-Screen Keyboard
//! settings page, re-read by mkeys on each launch (a fresh process per show).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Bottom,
    Top,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default = "default_layout")]
    pub layout: String,
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default)]
    pub position: Position,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_margin")]
    pub margin: i32,
    #[serde(default = "default_true")]
    pub show_pill: bool,
}

fn default_layout() -> String {
    "en".into()
}
fn default_scale() -> f32 {
    1.0
}
fn default_opacity() -> f32 {
    0.95
}
fn default_margin() -> i32 {
    8
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: default_layout(),
            scale: default_scale(),
            position: Position::default(),
            opacity: default_opacity(),
            margin: default_margin(),
            show_pill: default_true(),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/margo/mkeys.toml` (falls back to `~/.config`).
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let mut home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
                home.push(".config");
                home
            });
        base.join("margo").join("mkeys.toml")
    }

    /// Load from disk; any error → compiled defaults (never panics).
    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("mkeys.toml parse error ({e}); using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_yields_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.layout, "en");
        assert_eq!(c.position, Position::Bottom);
        assert!((c.scale - 1.0).abs() < f32::EPSILON);
        assert!((c.opacity - 0.95).abs() < f32::EPSILON);
        assert_eq!(c.margin, 8);
        assert!(c.show_pill);
    }

    #[test]
    fn partial_toml_overrides_only_present_fields() {
        let c: Config = toml::from_str("layout = \"tr\"\nposition = \"top\"").unwrap();
        assert_eq!(c.layout, "tr");
        assert_eq!(c.position, Position::Top);
        assert_eq!(c.margin, 8); // still default
    }
}
