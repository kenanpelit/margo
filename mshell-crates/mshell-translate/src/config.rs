//! Persistent translate settings.
//!
//! Non-secret knobs live in `~/.config/margo/translate.json`; the DeepL key
//! is stored in the OS keyring (Secret Service) under the `margo-translate`
//! service, never on disk. Both the Settings → Translate page and the bar
//! widget / menu read through here, so they always agree.

use crate::{Provider, TranslateConfig};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "margo-translate";
const KEYRING_USER: &str = "deepl_api_key";

/// On-disk (non-secret) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslateSettings {
    /// Provider id (`google` / `deepl`).
    pub provider: String,
    pub target_lang: String,
    /// Source language override; blank = auto-detect.
    pub source_lang: String,
    /// Capture-by-selection-or-copy: only consider a selection fresher than
    /// this many seconds.
    pub selection_max_age_secs: u32,
    /// Same, for the regular clipboard.
    pub clipboard_max_age_secs: u32,
    /// Keep a persistent history of past translations (shown in the menu).
    pub keep_history: bool,
}

impl Default for TranslateSettings {
    fn default() -> Self {
        TranslateSettings {
            provider: "google".into(),
            target_lang: "tr".into(),
            source_lang: String::new(),
            selection_max_age_secs: 15,
            clipboard_max_age_secs: 60,
            keep_history: true,
        }
    }
}

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        })
        .join("margo")
}

fn settings_path() -> PathBuf {
    config_dir().join("translate.json")
}

/// Load settings (defaults when the file is missing or unparseable).
pub fn load() -> TranslateSettings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist settings to `translate.json` (best-effort; creates the dir).
pub fn save(s: &TranslateSettings) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(settings_path(), json);
    }
}

fn keyring_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

/// The stored DeepL API key (empty when unset). Zeroized on drop.
pub fn deepl_api_key() -> zeroize::Zeroizing<String> {
    keyring_entry()
        .and_then(|e| e.get_password().ok())
        .unwrap_or_default()
        .into()
}

/// Store (or, with an empty value, clear) the DeepL API key in the keyring.
pub fn set_deepl_api_key(value: &str) {
    let Some(entry) = keyring_entry() else {
        return;
    };
    if value.is_empty() {
        let _ = entry.delete_credential();
    } else {
        let _ = entry.set_password(value);
    }
}

/// Build a ready-to-use [`TranslateConfig`] from the stored settings + keyring key.
pub fn resolved() -> TranslateConfig {
    let s = load();
    TranslateConfig {
        provider: Provider::parse(&s.provider),
        target_lang: s.target_lang,
        source_lang: (!s.source_lang.is_empty()).then_some(s.source_lang),
        deepl_api_key: deepl_api_key(),
    }
}
