//! `global_menu.*` — the GlobalMenu bar widget: the focused window's
//! application menu (AppMenu registrar + dbusmenu) in the bar.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct GlobalMenuConfig {
    /// Serve the AppMenu registrar and show menus. Off by default: apps
    /// only export menus when their appmenu module is enabled (see the
    /// widget docs), and owning the registrar name changes how they behave.
    pub enabled: bool,
    /// Show the focused app's name in front of its menus.
    pub show_app_name: bool,
    /// At most this many top-level menus in the bar (the rest go under »).
    /// 0 shows all.
    pub max_menus: u32,
}

impl Default for GlobalMenuConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_app_name: true,
            max_menus: 0,
        }
    }
}
