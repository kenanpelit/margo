//! mshell-plugins — the **mplugins** manager core.
//!
//! Installs *declarative* widget plugins from external git repositories.
//! mshell is compiled, so plugins ship no code: each is a `manifest.toml`
//! ([`Manifest`]) describing bar widgets ([`WidgetDef`]) that the shell
//! renders with its built-in custom-widget engine. This crate owns the
//! formats, the on-disk layout, the git fetch/install plumbing, and the
//! local enabled/sources state — but not any UI.

pub mod git;
pub mod keys;
pub mod manifest;
pub mod state;

pub use manifest::{Manifest, Registry, RegistryEntry, WidgetDef, meets_min_mshell, validate};
pub use state::{PluginsState, Source};

use std::path::{Path, PathBuf};

/// Official source repo. Plugins from here keep plain ids (no hash prefix).
pub const OFFICIAL_SOURCE: &str = "https://github.com/kenanpelit/margo-plugins";

/// Display name seeded for the official source.
pub const OFFICIAL_SOURCE_NAME: &str = "Official";

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("git: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("invalid plugin: {0}")]
    Invalid(String),
}

/// A plugin found on disk under the plugins dir.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    /// Composite key (= the folder name under the plugins dir).
    pub key: String,
    pub manifest: Manifest,
    pub dir: PathBuf,
}

/// Filesystem-facing manager: resolves paths and brokers state + git ops.
#[derive(Debug, Clone)]
pub struct PluginStore {
    config_dir: PathBuf,
}

impl Default for PluginStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginStore {
    /// Anchored at `~/.config/margo/mshell` (falls back to `./` if `$HOME`
    /// can't be resolved, which only happens in degenerate environments).
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .map(|c| c.join("margo").join("mshell"))
            .unwrap_or_else(|| PathBuf::from("."));
        Self { config_dir }
    }

    /// For tests / non-standard roots.
    pub fn with_config_dir(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir.join("plugins")
    }

    pub fn state_path(&self) -> PathBuf {
        self.config_dir.join("plugins.toml")
    }

    pub fn plugin_dir(&self, key: &str) -> PathBuf {
        self.plugins_dir().join(key)
    }

    /// Load `plugins.toml`, always guaranteeing the official source is
    /// present. Missing / unparseable file → defaults (logged).
    pub fn load_state(&self) -> PluginsState {
        let mut st = match std::fs::read_to_string(self.state_path()) {
            Ok(text) => toml::from_str::<PluginsState>(&text).unwrap_or_else(|e| {
                tracing::warn!("plugins.toml parse error ({e}); using defaults");
                PluginsState::default()
            }),
            Err(_) => PluginsState::default(),
        };
        st.ensure_source(OFFICIAL_SOURCE_NAME, OFFICIAL_SOURCE);
        st
    }

    pub fn save_state(&self, st: &PluginsState) -> Result<(), PluginError> {
        std::fs::create_dir_all(&self.config_dir)?;
        let text = toml::to_string_pretty(st).map_err(|e| PluginError::Parse(e.to_string()))?;
        std::fs::write(self.state_path(), text)?;
        Ok(())
    }

    /// Composite key a plugin id would get under `source_url`.
    pub fn key_for(&self, id: &str, source_url: &str) -> String {
        keys::composite_key(id, source_url, OFFICIAL_SOURCE)
    }

    /// Scan the plugins dir for installed plugins with a valid manifest.
    pub fn installed(&self) -> Vec<InstalledPlugin> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.plugins_dir()) else {
            return out;
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let key = entry.file_name().to_string_lossy().into_owned();
            match read_manifest(&dir) {
                Ok(manifest) => out.push(InstalledPlugin { key, manifest, dir }),
                Err(e) => tracing::warn!("skipping plugin `{key}`: {e}"),
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Fetch a source's registry (network).
    pub fn fetch_registry(&self, source_url: &str) -> Result<Registry, PluginError> {
        git::fetch_registry(source_url)
    }

    /// Install `entry` from `source_url`, validate its manifest, and return
    /// the composite key it was stored under. Does NOT enable it.
    pub fn install(
        &self,
        source_url: &str,
        entry: &RegistryEntry,
    ) -> Result<String, PluginError> {
        let key = self.key_for(&entry.id, source_url);
        let dest = self.plugin_dir(&key);
        git::install_plugin(source_url, &entry.dir, &dest)?;

        match read_manifest(&dest) {
            Ok(m) => {
                if let Err(e) = validate(&m) {
                    let _ = std::fs::remove_dir_all(&dest);
                    return Err(PluginError::Invalid(e));
                }
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(e);
            }
        }
        Ok(key)
    }

    /// Remove an installed plugin's folder. Caller drops it from `enabled`.
    pub fn uninstall(&self, key: &str) -> Result<(), PluginError> {
        let dir = self.plugin_dir(key);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Read + parse `<dir>/manifest.toml`.
fn read_manifest(dir: &Path) -> Result<Manifest, PluginError> {
    let path = dir.join("manifest.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| PluginError::Invalid(format!("manifest.toml unreadable: {e}")))?;
    toml::from_str::<Manifest>(&text).map_err(|e| PluginError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_for_official_vs_custom() {
        let store = PluginStore::with_config_dir("/tmp/nonexistent-mplugins-test");
        assert_eq!(store.key_for("weather", OFFICIAL_SOURCE), "weather");
        let custom = store.key_for("weather", "https://example/other");
        assert!(custom.ends_with(":weather") && custom.len() > "weather".len());
    }

    #[test]
    fn load_state_seeds_official_source() {
        // A config dir with no plugins.toml → defaults + official source.
        let store = PluginStore::with_config_dir("/tmp/nonexistent-mplugins-xyz");
        let st = store.load_state();
        assert!(st.sources.iter().any(|s| s.url == OFFICIAL_SOURCE));
    }

    #[test]
    fn state_save_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "mplugins-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = PluginStore::with_config_dir(&tmp);
        let mut st = store.load_state();
        st.set_enabled("weather", true);
        store.save_state(&st).unwrap();

        let back = store.load_state();
        assert!(back.is_enabled("weather"));
        assert!(back.sources.iter().any(|s| s.url == OFFICIAL_SOURCE));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn installed_is_empty_without_dir() {
        let store = PluginStore::with_config_dir("/tmp/nonexistent-mplugins-none");
        assert!(store.installed().is_empty());
    }
}
