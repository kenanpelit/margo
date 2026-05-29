//! Plugin manifest + source registry formats (both TOML).
//!
//! A *source* is a git repo with a `registry.toml` at its root listing the
//! plugins it offers ([`Registry`]). Each plugin lives in its own folder
//! with a `manifest.toml` ([`Manifest`]) describing its metadata and one or
//! more declarative bar widgets ([`WidgetDef`]).

use serde::{Deserialize, Serialize};

/// A plugin's `manifest.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Manifest {
    /// Stable id, unique within its source. No `:` or `/`.
    pub id: String,
    /// Human-readable name shown in the plugin list.
    #[serde(default)]
    pub name: String,
    /// Plugin version (`x.y.z`); should match the registry entry.
    pub version: String,
    #[serde(default)]
    pub author: String,
    /// Minimum mshell version required (`x.y.z`); empty = no floor.
    #[serde(default)]
    pub min_mshell: String,
    #[serde(default)]
    pub description: String,
    /// Declarative widgets this plugin contributes.
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetDef>,
}

/// One declarative widget. Mirrors the shell's custom-widget vocabulary:
/// a templated, optionally-polling label with an icon and click commands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct WidgetDef {
    /// Key unique within the plugin; placed in a bar as `plugin:<id>:<key>`.
    pub key: String,
    #[serde(default)]
    pub icon: String,
    /// Image path relative to the plugin dir — takes precedence over `icon`.
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub tooltip: String,
    /// Command (`sh -c`) whose stdout fills the label.
    #[serde(default)]
    pub exec: String,
    /// Label template; `{output}` = trimmed `exec` stdout.
    #[serde(default)]
    pub template: String,
    /// `exec` refresh cadence in seconds (0 = run once).
    #[serde(default)]
    pub interval: u64,
    #[serde(default)]
    pub on_click: String,
    #[serde(default)]
    pub on_click_right: String,
    /// Truncate the rendered label to this many chars (0 = no cap).
    #[serde(default)]
    pub max_chars: u32,
    /// Optional dropdown menu shown on click (a popover of command rows).
    /// When present, a left-click opens this menu instead of running
    /// `on_click`.
    #[serde(default, rename = "menu")]
    pub menu: Vec<MenuRow>,
}

/// One row of a widget's dropdown menu: an icon + label that runs a command.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct MenuRow {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: String,
    /// Command (`sh -c`) run when the row is activated.
    #[serde(default)]
    pub exec: String,
}

/// A source's root `registry.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Registry {
    #[serde(default, rename = "plugins")]
    pub plugins: Vec<RegistryEntry>,
}

/// One `[[plugins]]` row in a `registry.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct RegistryEntry {
    pub id: String,
    /// Folder in the source repo holding this plugin's `manifest.toml`.
    pub dir: String,
    pub version: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub min_mshell: String,
    #[serde(default)]
    pub description: String,
}

/// Validate a parsed manifest before it's trusted/installed.
pub fn validate(m: &Manifest) -> Result<(), String> {
    if m.id.trim().is_empty() {
        return Err("manifest id is empty".into());
    }
    if m.id.contains(':') || m.id.contains('/') {
        return Err(format!("manifest id `{}` must not contain ':' or '/'", m.id));
    }
    if m.version.trim().is_empty() {
        return Err("manifest version is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for w in &m.widgets {
        if w.key.trim().is_empty() {
            return Err("a widget has an empty key".into());
        }
        if !seen.insert(w.key.as_str()) {
            return Err(format!("duplicate widget key `{}`", w.key));
        }
    }
    Ok(())
}

/// `true` if `current` mshell version satisfies `min` (`x.y.z` compare).
/// An empty `min` means "no floor". Unparseable parts count as 0.
pub fn meets_min_mshell(min: &str, current: &str) -> bool {
    if min.trim().is_empty() {
        return true;
    }
    parse_version(current) >= parse_version(min)
}

/// `true` if `candidate` is a strictly newer `x.y.z` version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn parse_version(v: &str) -> (u64, u64, u64) {
    let mut it = v
        .trim()
        .split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id = "weather-tr"
name = "Türkiye Weather"
version = "1.2.0"
author = "kenanpelit"
min_mshell = "0.8.8"
description = "wttr.in pill"

[[widget]]
key = "current"
icon = "weather-few-clouds-symbolic"
exec = "curl -s wttr.in"
template = "{output}"
interval = 900
on_click = "xdg-open https://wttr.in"
"#;

    #[test]
    fn parses_manifest() {
        let m: Manifest = toml::from_str(SAMPLE).unwrap();
        assert_eq!(m.id, "weather-tr");
        assert_eq!(m.version, "1.2.0");
        assert_eq!(m.widgets.len(), 1);
        assert_eq!(m.widgets[0].key, "current");
        assert_eq!(m.widgets[0].interval, 900);
        assert_eq!(m.widgets[0].max_chars, 0); // defaulted
    }

    #[test]
    fn parses_registry() {
        let reg: Registry = toml::from_str(
            r#"
[[plugins]]
id = "weather-tr"
dir = "weather-tr"
version = "1.2.0"
name = "Türkiye Weather"
min_mshell = "0.8.8"
"#,
        )
        .unwrap();
        assert_eq!(reg.plugins.len(), 1);
        assert_eq!(reg.plugins[0].dir, "weather-tr");
    }

    #[test]
    fn empty_registry_ok() {
        let reg: Registry = toml::from_str("# nothing yet\n").unwrap();
        assert!(reg.plugins.is_empty());
    }

    #[test]
    fn validate_accepts_good_manifest() {
        let m: Manifest = toml::from_str(SAMPLE).unwrap();
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn validate_rejects_bad_ids_and_dupes() {
        let mut m = Manifest {
            id: String::new(),
            version: "1.0.0".into(),
            ..Default::default()
        };
        assert!(validate(&m).is_err()); // empty id

        m.id = "a:b".into();
        assert!(validate(&m).is_err()); // colon in id

        m.id = "ok".into();
        m.version = String::new();
        assert!(validate(&m).is_err()); // empty version

        m.version = "1.0.0".into();
        m.widgets = vec![
            WidgetDef { key: "x".into(), ..Default::default() },
            WidgetDef { key: "x".into(), ..Default::default() },
        ];
        assert!(validate(&m).is_err()); // duplicate keys
    }

    #[test]
    fn version_gate() {
        assert!(meets_min_mshell("", "0.0.1"));
        assert!(meets_min_mshell("0.8.8", "0.8.8"));
        assert!(meets_min_mshell("0.8.8", "0.9.0"));
        assert!(meets_min_mshell("0.8.8", "1.0.0"));
        assert!(!meets_min_mshell("0.9.0", "0.8.8"));
        assert!(!meets_min_mshell("1.0.0", "0.8.20"));
    }
}
