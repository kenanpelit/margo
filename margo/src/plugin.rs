//! Rhai plugin packaging — W3.3 from the catch-and-surpass-niri plan.
//!
//! Lets users drop `~/.config/margo/plugins/<name>/` directories
//! that the compositor auto-loads at startup. Each plugin is a
//! manifest file + a Rhai script:
//!
//! ```text
//!   ~/.config/margo/plugins/
//!     ├── smart-tag-routing/
//!     │   ├── plugin.toml      ← name, version, description
//!     │   └── init.rhai        ← compositor hooks + dispatch
//!     └── ai-window-grouping/
//!         ├── plugin.toml
//!         └── init.rhai
//! ```
//!
//! Same engine + binding surface as the user's existing
//! `~/.config/margo/init.rhai`. The compositor doesn't sandbox
//! per-plugin (Rhai is already sandboxed against host code) — all
//! plugins share one engine + AST state. Hook registrations from
//! a plugin survive in the same `ScriptingHooks` lists; events
//! fire every plugin's handler.
//!
//! ## Why packaging matters
//!
//! `init.rhai` is one file the user authors. Plugins make
//! sharing possible: drop someone else's plugin directory in,
//! restart margo, get their behaviour. Standard package layout
//! (manifest + body) lets a future `mctl plugin install <url>`
//! / `mctl plugin list` / `mctl plugin enable / disable`
//! workflow plug into this discovery.
//!
//! ## Manifest format
//!
//! ```toml
//! # ~/.config/margo/plugins/<name>/plugin.toml
//! name = "smart-tag-routing"
//! version = "0.1.0"
//! description = "Auto-tag windows based on title patterns"
//! enabled = true                # default true; flip to disable
//! # Optional: minimum margo version. Future-proofing for when
//! # the binding surface evolves and old plugins need a guard.
//! min-margo-version = "0.1.0"
//! ```
//!
//! Bare strings only — no nested tables, no arrays. Keeps the
//! parser tiny (one `serde::Deserialize` derive) and the
//! manifest readable.

use std::path::{Path, PathBuf};

use tracing::{info, warn};

/// What a plugin's `plugin.toml` deserialises into. All fields
/// required-but-trivially-defaulted so a manifest with just a
/// `name` line still parses. Hand-parsed in [`parse_manifest`]
/// — full TOML dep is overkill for four fields.
#[derive(Debug)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// `false` skips loading without a config-side opt-out — lets
    /// users pin a plugin to a directory but disable it
    /// transiently while debugging. Default `true`.
    pub enabled: bool,
    /// Reserved for future "your plugin needs margo ≥ X.Y" guards.
    /// Not enforced today; surface logged at info level so users
    /// see when a plugin declares one. TOML key is
    /// `min-margo-version` (the parser reads the dashed form).
    pub min_margo_version: Option<String>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            description: String::new(),
            enabled: true,
            min_margo_version: None,
        }
    }
}

/// One discovered plugin. Held by `MargoState::plugins` (when the
/// scripting feature is on) so `mctl plugin list` can enumerate.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub manifest: PluginManifestSlim,
    /// Path to the plugin's directory.
    pub dir: PathBuf,
    /// Path to its init.rhai (`dir.join("init.rhai")`).
    pub script: PathBuf,
    /// `true` if the script compiled + ran cleanly at startup.
    /// `false` for compile errors / disabled / missing init.rhai.
    pub loaded: bool,
}

/// Cheap copy of [`PluginManifest`]'s public fields — Plugin
/// holds this rather than the original so `Plugin: Clone` is
/// cheap and serde doesn't need to derive Clone.
#[derive(Debug, Clone)]
pub struct PluginManifestSlim {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
}

impl From<PluginManifest> for PluginManifestSlim {
    fn from(m: PluginManifest) -> Self {
        Self {
            name: m.name,
            version: m.version,
            description: m.description,
            enabled: m.enabled,
        }
    }
}

/// Discover the plugins directory. Returns `None` if no
/// `~/.config/margo/plugins/` exists — most installs run
/// without plugins, this is the silent default.
pub fn plugins_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("XDG_CONFIG_HOME").map(|h| PathBuf::from(h).join("margo/plugins")),
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/margo/plugins")),
    ];
    candidates.into_iter().flatten().find(|c| c.is_dir())
}

/// Walk the plugins directory and return a description of every
/// candidate. Doesn't load / eval — that happens later via
/// [`load_plugin`] once the scripting engine is parked on
/// MargoState.
pub fn discover() -> Vec<Plugin> {
    let Some(root) = plugins_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) => {
            warn!("plugins: read_dir({}): {e}", root.display());
            return Vec::new();
        }
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("plugin.toml");
        let script_path = dir.join("init.rhai");
        if !manifest_path.is_file() {
            // Skip — not a plugin, just some random dir.
            continue;
        }
        let manifest = match parse_manifest(&manifest_path) {
            Ok(m) => m,
            Err(e) => {
                warn!("plugins: skipping {} — {e}", dir.display());
                continue;
            }
        };
        out.push(Plugin {
            manifest: manifest.into(),
            dir: dir.clone(),
            script: script_path,
            loaded: false,
        });
    }
    out.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    out
}

fn parse_manifest(path: &Path) -> anyhow::Result<PluginManifest> {
    let body = std::fs::read_to_string(path)?;
    // Hand-rolled, since we don't want to pull a full TOML
    // dependency for a 4-field manifest. Simple `key = value`
    // line format with `=` separator + quoted string values.
    // Strict-ish: comments allowed (#); empty lines ignored;
    // unknown keys logged.
    let mut m = PluginManifest::default();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            anyhow::bail!("line {}: expected `key = value`", lineno + 1);
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k {
            "name" => m.name = v.to_string(),
            "version" => m.version = v.to_string(),
            "description" => m.description = v.to_string(),
            "enabled" => {
                m.enabled = matches!(v, "true" | "yes" | "1" | "on");
            }
            "min-margo-version" => {
                m.min_margo_version = Some(v.to_string());
            }
            other => {
                info!("plugins: unknown manifest key `{other}` (ignored)");
            }
        }
    }
    if m.name.is_empty() {
        anyhow::bail!("manifest missing required `name` field");
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_full() {
        let toml = r#"
            # margo plugin manifest
            name = "test-plugin"
            version = "1.2.3"
            description = "A test"
            enabled = true
            min-margo-version = "0.1.0"
        "#;
        let tmp = std::env::temp_dir().join("margo-plugin-test-full.toml");
        std::fs::write(&tmp, toml).unwrap();
        let m = parse_manifest(&tmp).unwrap();
        assert_eq!(m.name, "test-plugin");
        assert_eq!(m.version, "1.2.3");
        assert_eq!(m.description, "A test");
        assert!(m.enabled);
        assert_eq!(m.min_margo_version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn manifest_minimal_just_name() {
        let toml = "name = bare\n";
        let tmp = std::env::temp_dir().join("margo-plugin-test-min.toml");
        std::fs::write(&tmp, toml).unwrap();
        let m = parse_manifest(&tmp).unwrap();
        assert_eq!(m.name, "bare");
        assert!(m.enabled, "enabled defaults to true");
    }

    #[test]
    fn manifest_missing_name_errors() {
        let toml = "version = 1.0\n";
        let tmp = std::env::temp_dir().join("margo-plugin-test-noname.toml");
        std::fs::write(&tmp, toml).unwrap();
        let err = parse_manifest(&tmp).unwrap_err().to_string();
        assert!(err.contains("name"), "error should mention `name`: {err}");
    }

    #[test]
    fn manifest_disabled_flag() {
        let toml = r#"
            name = "off"
            enabled = false
        "#;
        let tmp = std::env::temp_dir().join("margo-plugin-test-off.toml");
        std::fs::write(&tmp, toml).unwrap();
        let m = parse_manifest(&tmp).unwrap();
        assert!(!m.enabled);
    }
}
