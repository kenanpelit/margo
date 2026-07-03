//! What mcal needs to know to load calendars.
//!
//! This is mcal's *own* config input — deliberately decoupled from the shell's
//! YAML schema (`mshell-config`). The UI maps its profile onto this struct, so
//! `mcal` stays free of any shell dependency and is unit-testable in isolation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A remote `.ics` calendar the user subscribes to (read-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub name: String,
    pub url: String,
    /// Optional CSS colour for the calendar's events (`#RRGGBB`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Everything a load needs: where local `.ics` live, which URLs to fetch, and
/// how often to refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarConfig {
    /// Directory of local `.ics` files / sub-directories.
    pub local_dir: PathBuf,
    /// Remote subscriptions.
    pub subscriptions: Vec<Subscription>,
    /// Seconds between background refreshes.
    pub refresh_secs: u64,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            local_dir: default_local_dir(),
            subscriptions: Vec::new(),
            refresh_secs: 900,
        }
    }
}

/// `$XDG_CONFIG_HOME/margo/calendars` (the canonical local calendar root).
pub fn default_local_dir() -> PathBuf {
    dirs::config_dir()
        .map(|c| c.join("margo").join("calendars"))
        .unwrap_or_else(|| PathBuf::from("~/.config/margo/calendars"))
}

/// mcal's own config/state directory: `$XDG_CONFIG_HOME/margo/mcal` (holds
/// `credentials.toml` + `accounts.toml`). Kept under the shared `margo/` tree
/// like the rest of the config (see `docs/config-conventions.md`), not a
/// top-level `~/.config/mcal`.
///
/// Side effect: a one-time migration renames a legacy `~/.config/mcal` here on
/// first access, so setups created before this move keep working. Keyring
/// tokens are keyed by account id and are unaffected.
pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("margo").join("mcal");
    let legacy = base.join("mcal");
    if !dir.exists() && legacy.is_dir() {
        if let Some(parent) = dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(&legacy, &dir);
    }
    dir
}
