//! Persistent AI settings.
//!
//! Non-secret knobs live in `~/.config/margo/ai.json`; the API key is stored
//! in the OS keyring (Secret Service) under the `margo-ai` service, never on
//! disk. Both the Settings → AI page and the bar widget / menu read through
//! here, so they always agree.

use crate::{AiConfig, Provider};
use margo_fs::side_file::{self, SaveError, SideLoad};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "margo-ai";
const KEYRING_USER: &str = "api_key";

/// On-disk (non-secret) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// Provider id (`gemini` / `openai` / `anthropic` / `ollama` / `custom`).
    pub provider: String,
    pub model: String,
    /// Endpoint override; blank = provider default.
    pub endpoint: String,
    pub temperature: f64,
    pub max_tokens: u32,
    pub system_prompt: String,
    /// Persist the conversation across restarts.
    pub persist_history: bool,
    /// Chat transcript font size in px (UI only; not sent to the API).
    pub font_size: u32,
    /// Chat transcript font family; blank = inherit the shell font.
    pub font_family: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: "gemini".into(),
            model: String::new(),
            endpoint: String::new(),
            temperature: 0.7,
            max_tokens: 2048,
            system_prompt: String::new(),
            persist_history: true,
            font_size: 14,
            font_family: String::new(),
        }
    }
}

fn config_dir() -> PathBuf {
    margo_fs::xdg::margo_config_dir()
}

/// Where the settings file lives.
pub fn settings_path() -> PathBuf {
    config_dir().join("ai.json")
}

/// Load settings. Defaults when the file is missing, and also when it does
/// not parse — then with a warning and a `ai.json.bak` copy, and [`save`]
/// refuses to write over it until it is fixed.
pub fn load() -> AiSettings {
    load_from(&settings_path())
}

fn load_from(path: &Path) -> AiSettings {
    let loaded = side_file::load(path, |raw| serde_json::from_str::<AiSettings>(raw));
    if let SideLoad::Broken { error, backup } = &loaded {
        tracing::warn!(
            path = %path.display(),
            backup = ?backup,
            error,
            "settings file does not parse; using defaults and not saving over it"
        );
    }
    loaded.or_default()
}

/// Persist settings to `ai.json` (atomic; creates the dir). Refuses while
/// the file on disk does not parse, so a typo never costs the whole file.
pub fn save(s: &AiSettings) -> Result<(), SaveError> {
    save_to(&settings_path(), s)
}

fn save_to(path: &Path, s: &AiSettings) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(s).map_err(|e| SaveError::Io {
        path: path.to_path_buf(),
        error: std::io::Error::other(e),
    })?;
    side_file::save(path, json.as_bytes(), |raw| {
        serde_json::from_str::<AiSettings>(raw).map(|_| ())
    })
}

fn keyring_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

/// The stored API key (empty when unset). Zeroized on drop.
pub fn api_key() -> zeroize::Zeroizing<String> {
    keyring_entry()
        .and_then(|e| e.get_password().ok())
        .unwrap_or_default()
        .into()
}

/// Store (or, with an empty value, clear) the API key in the keyring.
pub fn set_api_key(value: &str) {
    let Some(entry) = keyring_entry() else {
        return;
    };
    if value.is_empty() {
        let _ = entry.delete_credential();
    } else {
        let _ = entry.set_password(value);
    }
}

/// Build a ready-to-use [`AiConfig`] from the stored settings + keyring key.
pub fn resolved() -> AiConfig {
    let s = load();
    AiConfig {
        provider: Provider::parse(&s.provider),
        model: s.model,
        api_key: api_key(),
        endpoint: s.endpoint,
        temperature: s.temperature,
        max_tokens: s.max_tokens,
        system_prompt: s.system_prompt,
    }
}

#[cfg(test)]
mod side_file_tests {
    use super::*;

    /// A ai.json with a typo must survive a Settings change: load falls
    /// back to defaults, but save refuses to write those over the file.
    #[test]
    fn unparseable_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        let broken = "{\"provider\": \"x\", oops}";
        std::fs::write(&path, broken).unwrap();

        let s = load_from(&path);
        assert!(save_to(&path, &s).is_err(), "save is refused");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        assert!(dir.path().join("ai.json.bak").exists(), "a copy is kept");
    }

    #[test]
    fn round_trips_through_an_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/ai.json");
        let s = AiSettings::default();
        save_to(&path, &s).unwrap();
        let back = load_from(&path);
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&s).unwrap()
        );
    }
}
