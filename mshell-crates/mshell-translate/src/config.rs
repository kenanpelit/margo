//! Persistent translate settings.
//!
//! Non-secret knobs live in `~/.config/margo/translate.json`; the DeepL key
//! is stored in the OS keyring (Secret Service) under the `margo-translate`
//! service, never on disk. Both the Settings → Translate page and the bar
//! widget / menu read through here, so they always agree.

use crate::{Provider, TranslateConfig};
use margo_fs::side_file::{self, SaveError, SideLoad};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Auto-flip target: if the detected source language turns out to
    /// *be* `target_lang` (e.g. you selected Turkish text while
    /// `target_lang` is `tr`), translate to this language instead —
    /// see `translate_auto`. Blank disables the flip.
    pub secondary_lang: String,
    /// Capture-by-selection-or-copy: only consider a selection fresher than
    /// this many seconds.
    pub selection_max_age_secs: u32,
    /// Same, for the regular clipboard.
    pub clipboard_max_age_secs: u32,
    /// Keep a persistent history of past translations (shown in the menu).
    pub keep_history: bool,
    /// Microsoft Edge neural voice used to speak an English result aloud
    /// (`speak::EN_VOICES`).
    pub voice_en: String,
    /// Same, for a Turkish result (`speak::TR_VOICES`).
    pub voice_tr: String,
}

impl Default for TranslateSettings {
    fn default() -> Self {
        TranslateSettings {
            provider: "google".into(),
            target_lang: "tr".into(),
            source_lang: String::new(),
            secondary_lang: "en".into(),
            selection_max_age_secs: 15,
            clipboard_max_age_secs: 60,
            keep_history: true,
            voice_en: "en-US-GuyNeural".into(),
            voice_tr: "tr-TR-AhmetNeural".into(),
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

/// Where the settings file lives.
pub fn settings_path() -> PathBuf {
    config_dir().join("translate.json")
}

/// Load settings. Defaults when the file is missing, and also when it does
/// not parse — then with a warning and a `translate.json.bak` copy, and [`save`]
/// refuses to write over it until it is fixed.
pub fn load() -> TranslateSettings {
    load_from(&settings_path())
}

fn load_from(path: &Path) -> TranslateSettings {
    let loaded = side_file::load(path, |raw| serde_json::from_str::<TranslateSettings>(raw));
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

/// Persist settings to `translate.json` (atomic; creates the dir). Refuses while
/// the file on disk does not parse, so a typo never costs the whole file.
pub fn save(s: &TranslateSettings) -> Result<(), SaveError> {
    save_to(&settings_path(), s)
}

fn save_to(path: &Path, s: &TranslateSettings) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(s).map_err(|e| SaveError::Io {
        path: path.to_path_buf(),
        error: std::io::Error::other(e),
    })?;
    side_file::save(path, json.as_bytes(), |raw| {
        serde_json::from_str::<TranslateSettings>(raw).map(|_| ())
    })
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

#[cfg(test)]
mod side_file_tests {
    use super::*;

    /// A translate.json with a typo must survive a Settings change: load falls
    /// back to defaults, but save refuses to write those over the file.
    #[test]
    fn unparseable_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("translate.json");
        let broken = "{\"provider\": \"x\", oops}";
        std::fs::write(&path, broken).unwrap();

        let s = load_from(&path);
        assert!(save_to(&path, &s).is_err(), "save is refused");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        assert!(
            dir.path().join("translate.json.bak").exists(),
            "a copy is kept"
        );
    }

    #[test]
    fn round_trips_through_an_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/translate.json");
        let s = TranslateSettings::default();
        save_to(&path, &s).unwrap();
        let back = load_from(&path);
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&s).unwrap()
        );
    }
}
