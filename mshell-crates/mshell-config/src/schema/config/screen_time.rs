//! `screen_time.*` — the Screen Time pill + menu: how long each app had
//! the keyboard focus today and on the last days.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ScreenTimeConfig {
    /// Count focused-window time per app. Off by default: nothing is
    /// recorded (and the pill shows "off") until this is on.
    pub enabled: bool,
    /// Seconds without keyboard / mouse input after which time stops
    /// counting (a playing video holds an idle inhibitor and keeps
    /// counting). A locked session never counts.
    pub away_seconds: u32,
    /// Apps with less than this many minutes on a day are left out of the
    /// menu's lists (they still count towards the total). 0 lists all.
    pub hide_under_minutes: u32,
    /// Days of history kept in `$XDG_STATE_HOME/margo/screen-time.json`;
    /// older days are dropped at the next save.
    pub retention_days: u32,
}

impl Default for ScreenTimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            away_seconds: 300,
            hide_under_minutes: 1,
            retention_days: 30,
        }
    }
}
