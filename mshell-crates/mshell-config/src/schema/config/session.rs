//! `session.*` — the session menu: per-action command overrides and the "Reboot into…" list.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Session actions (the power menu). Each field is the command
/// run for that action; an empty string falls back to the
/// built-in default: Logout / Reboot / Shutdown go through margo's
/// `session_end` (every window is closed gracefully first, then
/// systemd), Suspend runs `systemctl suspend`, Lock the in-process
/// lock. Set a command to route the button through your own script
/// instead. Non-empty commands run via `sh -c`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Session {
    pub lock_command: String,
    pub logout_command: String,
    pub suspend_command: String,
    pub reboot_command: String,
    pub shutdown_command: String,
    /// Show the "Reboot into…" list in the session menu: pick the UEFI
    /// entry the next boot starts (`BootNext`, one boot only) and reboot.
    /// Needs `efibootmgr`; off by default.
    pub reboot_into: bool,
    /// Offer every active firmware entry ("USB HDD", network boot, …), not
    /// only the ones that boot a loader from a disk partition.
    pub reboot_into_all: bool,
    /// Entries whose label contains one of these words (any case) are left
    /// out of the "Reboot into…" list.
    pub reboot_into_hidden: Vec<String>,
}

/// The firmware entries a "Reboot into…" list leaves out by default
/// (network boot, shells, setup and diagnostics apps).
pub fn default_reboot_into_hidden() -> Vec<String> {
    [
        "pxe",
        "uefi os",
        "uefi shell",
        "shell",
        "ipv4",
        "ipv6",
        "network",
        "diagnostics",
        "setup",
        "recovery",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for Session {
    fn default() -> Self {
        Self {
            lock_command: String::new(),
            logout_command: String::new(),
            suspend_command: String::new(),
            reboot_command: String::new(),
            shutdown_command: String::new(),
            reboot_into: false,
            reboot_into_all: false,
            reboot_into_hidden: default_reboot_into_hidden(),
        }
    }
}
