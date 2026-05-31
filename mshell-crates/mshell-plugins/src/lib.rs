//! mshell-plugins — the **mplugins** manager core.
//!
//! Installs *declarative* widget plugins from external git repositories.
//! mshell is compiled, so plugins ship no code: each is a `manifest.toml`
//! ([`Manifest`]) describing bar widgets ([`WidgetDef`]) that the shell
//! renders with its built-in custom-widget engine. This crate owns the
//! formats, the on-disk layout, the git fetch/install plumbing, and the
//! local enabled/sources state — but not any UI.

pub mod git;
pub mod keybinds;
pub mod keys;
pub mod manifest;
pub mod secrets;
pub mod state;

pub use manifest::{
    Keybind, Manifest, MenuRow, Registry, RegistryEntry, Setting, WidgetDef, is_newer,
    meets_min_mshell, substitute, validate,
};
pub use state::{PanelLayout, PluginsState, Source};

use std::path::{Path, PathBuf};

/// Official source repo. Plugins from here keep plain ids (no hash prefix).
pub const OFFICIAL_SOURCE: &str = "https://github.com/kenanpelit/margo-plugins";

/// Display name seeded for the official source.
pub const OFFICIAL_SOURCE_NAME: &str = "Official";

/// The mshell version this build reports for plugin `min_mshell` checks. The
/// workspace version is shared across every crate, so this crate's
/// `CARGO_PKG_VERSION` is the running shell's version.
pub const MSHELL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `true` if a plugin declaring `min_mshell = min` may run on this build.
pub fn compatible(min: &str) -> bool {
    meets_min_mshell(min, MSHELL_VERSION)
}

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
    #[error("requires mshell ≥ {required} (you have {current})")]
    Incompatible { required: String, current: String },
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
        let path = self.state_path();
        std::fs::write(&path, text)?;
        // The file can hold secret setting values (API keys), so keep it
        // owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
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
    pub fn install(&self, source_url: &str, entry: &RegistryEntry) -> Result<String, PluginError> {
        // Gate on the registry's declared min_mshell before downloading.
        if !compatible(&entry.min_mshell) {
            return Err(PluginError::Incompatible {
                required: entry.min_mshell.clone(),
                current: MSHELL_VERSION.to_string(),
            });
        }

        let key = self.key_for(&entry.id, source_url);
        let dest = self.plugin_dir(&key);
        git::install_plugin(source_url, &entry.dir, &dest)?;

        let manifest = match read_manifest(&dest) {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(e);
            }
        };
        if let Err(e) = validate(&manifest) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(PluginError::Invalid(e));
        }
        // Re-check against the manifest (it's authoritative; the registry
        // entry can lag).
        if !compatible(&manifest.min_mshell) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(PluginError::Incompatible {
                required: manifest.min_mshell.clone(),
                current: MSHELL_VERSION.to_string(),
            });
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

    /// One-shot: for every installed plugin, move any setting value that
    /// lives in `plugins.toml` but is marked `type = "secret"` by the
    /// manifest into the system keyring, and drop it from state. Returns
    /// the number moved. Safe to call on every startup — idempotent once
    /// state has caught up.
    pub fn migrate_plaintext_secrets(&self) -> usize {
        let mut state = self.load_state();
        let mut moved = 0;
        let mut dirty = false;
        for plugin in self.installed() {
            for setting in &plugin.manifest.settings {
                if !setting.is_secret() {
                    continue;
                }
                let Some(value) = state.setting(&plugin.key, &setting.key).cloned() else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                if let Err(e) = secrets::write(&plugin.key, &setting.key, &value) {
                    tracing::warn!(
                        plugin = %plugin.key,
                        setting = %setting.key,
                        "secret migration failed: {e}"
                    );
                    continue;
                }
                if let Some(m) = state.settings.get_mut(&plugin.key) {
                    m.remove(&setting.key);
                    if m.is_empty() {
                        state.settings.remove(&plugin.key);
                    }
                }
                moved += 1;
                dirty = true;
            }
        }
        if dirty && let Err(e) = self.save_state(&state) {
            tracing::warn!("secret migration: save_state failed: {e}");
        }
        moved
    }

    /// Fetch every configured source's registry and reinstall each installed,
    /// *enabled* plugin that has a newer version available. Blocking (git +
    /// network) — run off the GTK main thread. Returns the composite keys that
    /// were updated; the caller reloads config so the new versions take effect.
    pub fn run_update_pass(&self) -> UpdateOutcome {
        let state = self.load_state();
        let installed = self.installed();
        let mut out = UpdateOutcome::default();

        // Fetch every source's registry once.
        let registries: Vec<(String, Registry)> = state
            .sources
            .iter()
            .filter_map(|src| match self.fetch_registry(&src.url) {
                Ok(reg) => Some((src.url.clone(), reg)),
                Err(e) => {
                    out.errors.push(format!("{}: {e}", src.name));
                    None
                }
            })
            .collect();

        for p in &installed {
            if !state.is_enabled(&p.key) {
                continue;
            }
            // The newest registry entry for this plugin that beats the installed
            // version, across all sources.
            let mut best: Option<(&str, &RegistryEntry)> = None;
            for (url, reg) in &registries {
                for entry in &reg.plugins {
                    if self.key_for(&entry.id, url) == p.key
                        && is_newer(&entry.version, &p.manifest.version)
                        && best.is_none_or(|(_, b)| is_newer(&entry.version, &b.version))
                    {
                        best = Some((url.as_str(), entry));
                    }
                }
            }
            if let Some((url, entry)) = best {
                match self.install(url, entry) {
                    Ok(_) => out.updated.push(p.key.clone()),
                    Err(e) => out.errors.push(format!("{}: {e}", p.key)),
                }
            }
        }
        out
    }
}

/// Result of one [`PluginStore::run_update_pass`].
#[derive(Debug, Default, Clone)]
pub struct UpdateOutcome {
    /// Composite keys that were updated to a newer version.
    pub updated: Vec<String>,
    /// Per-source / per-plugin error messages (non-fatal — the pass continues).
    pub errors: Vec<String>,
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
    fn compatibility_gate() {
        assert!(compatible("")); // no floor
        assert!(compatible(MSHELL_VERSION)); // exactly this build
        assert!(compatible("0.0.1")); // older floor
        assert!(!compatible("999.0.0")); // far-future floor
    }

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
