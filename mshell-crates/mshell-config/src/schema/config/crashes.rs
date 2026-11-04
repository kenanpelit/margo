//! `crashes.*` — the crash watcher (systemd-coredump) and its menu.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct CrashesConfig {
    /// Raise a critical notification when one of your own processes dumps
    /// core (read from systemd-coredump via `coredumpctl`). Off by default.
    pub notify: bool,
    /// Program names (the executable's file name, case-insensitive) whose
    /// crashes are not announced. Toggled from the Crashes menu.
    pub muted: Vec<String>,
    /// Lowest journal priority the menu's Errors tab lists: `emerg`,
    /// `alert`, `crit`, `err` (default) or `warning`.
    pub journal_priority: String,
}

impl Default for CrashesConfig {
    fn default() -> Self {
        Self {
            notify: false,
            muted: Vec::new(),
            journal_priority: "err".to_string(),
        }
    }
}
