//! Local manager state, persisted to `plugins.toml` in the mshell config
//! dir. Holds the configured sources and the set of enabled plugin keys.
//! The user's hand-edited profile YAML is left untouched — this file is
//! owned by the plugin manager.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A plugin source: a name + the git repo URL holding its `registry.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Source {
    pub name: String,
    pub url: String,
}

/// Persisted manager state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct PluginsState {
    #[serde(default, rename = "sources")]
    pub sources: Vec<Source>,
    /// Composite keys of plugins the user has enabled.
    #[serde(default)]
    pub enabled: Vec<String>,
    /// Per-plugin setting values: `{ composite-key: { setting-key: value } }`.
    /// Substituted into the plugin's commands via `{{setting-key}}`.
    #[serde(default)]
    pub settings: BTreeMap<String, BTreeMap<String, String>>,
}

impl PluginsState {
    /// Ensure `url` is present in `sources` (adds it with `name` if absent).
    pub fn ensure_source(&mut self, name: &str, url: &str) {
        if !self.sources.iter().any(|s| s.url.trim() == url.trim()) {
            self.sources.push(Source {
                name: name.to_string(),
                url: url.to_string(),
            });
        }
    }

    pub fn is_enabled(&self, key: &str) -> bool {
        self.enabled.iter().any(|k| k == key)
    }

    pub fn set_enabled(&mut self, key: &str, on: bool) {
        let present = self.is_enabled(key);
        if on && !present {
            self.enabled.push(key.to_string());
        } else if !on && present {
            self.enabled.retain(|k| k != key);
        }
    }

    /// A plugin's stored value for a setting, if the user has set one.
    pub fn setting(&self, plugin: &str, key: &str) -> Option<&String> {
        self.settings.get(plugin).and_then(|m| m.get(key))
    }

    pub fn set_setting(&mut self, plugin: &str, key: &str, value: &str) {
        self.settings
            .entry(plugin.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
    }

    /// Drop all state for a plugin (on uninstall).
    pub fn forget(&mut self, key: &str) {
        self.enabled.retain(|k| k != key);
        self.settings.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut s = PluginsState::default();
        s.ensure_source("Official", "https://example/repo");
        s.set_enabled("weather", true);
        s.set_enabled("a1b2c3:cpu", true);

        let text = toml::to_string(&s).unwrap();
        let back: PluginsState = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn ensure_source_is_idempotent() {
        let mut s = PluginsState::default();
        s.ensure_source("Official", "https://example/repo");
        s.ensure_source("Dup", "https://example/repo");
        assert_eq!(s.sources.len(), 1);
    }

    #[test]
    fn enable_toggle() {
        let mut s = PluginsState::default();
        s.set_enabled("x", true);
        assert!(s.is_enabled("x"));
        s.set_enabled("x", true); // no duplicate
        assert_eq!(s.enabled.len(), 1);
        s.set_enabled("x", false);
        assert!(!s.is_enabled("x"));
    }

    #[test]
    fn defaults_when_empty() {
        let s: PluginsState = toml::from_str("").unwrap();
        assert!(s.sources.is_empty());
        assert!(s.enabled.is_empty());
        assert!(s.settings.is_empty());
    }

    #[test]
    fn settings_storage_and_forget() {
        let mut s = PluginsState::default();
        s.set_setting("weather", "city", "Istanbul");
        s.set_enabled("weather", true);
        assert_eq!(s.setting("weather", "city").map(String::as_str), Some("Istanbul"));

        // Survives a TOML round-trip.
        let back: PluginsState = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.setting("weather", "city").map(String::as_str), Some("Istanbul"));

        // forget() drops both enabled + settings.
        s.forget("weather");
        assert!(!s.is_enabled("weather"));
        assert!(s.setting("weather", "city").is_none());
    }
}
