//! Persistent AI settings.
//!
//! Non-secret knobs live in `~/.config/margo/ai.json`; the API key is stored
//! in the OS keyring (Secret Service) under the `margo-ai` service, never on
//! disk. Both the Settings → AI page and the bar widget / menu read through
//! here, so they always agree.

use crate::{AiConfig, Provider};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        })
        .join("margo")
}

fn settings_path() -> PathBuf {
    config_dir().join("ai.json")
}

/// Load settings (defaults when the file is missing or unparseable).
pub fn load() -> AiSettings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist settings to `ai.json` (best-effort; creates the dir).
pub fn save(s: &AiSettings) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(settings_path(), json);
    }
}

fn keyring_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

/// The stored API key (empty when unset).
pub fn api_key() -> String {
    keyring_entry()
        .and_then(|e| e.get_password().ok())
        .unwrap_or_default()
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
