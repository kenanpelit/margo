//! `flatpak_bg.*` — the Flatpak Background pill + menu: Flatpak apps that
//! run without a window, with a Stop button.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct FlatpakBgConfig {
    /// Seconds between `flatpak ps` scans while the pill or the menu is
    /// shown (5–600).
    pub interval_seconds: u32,
    /// Flatpak app ids never listed (e.g. a sync client meant to run in
    /// the background). Toggled from the menu.
    pub ignored_apps: Vec<String>,
}

impl Default for FlatpakBgConfig {
    fn default() -> Self {
        Self {
            interval_seconds: 10,
            ignored_apps: Vec::new(),
        }
    }
}
