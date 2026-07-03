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
