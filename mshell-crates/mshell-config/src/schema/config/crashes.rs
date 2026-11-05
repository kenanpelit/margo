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

/// The `crashes.journal_priority` values the Errors tab understands, in
/// syslog order (index = priority number), with their Settings labels.
pub const JOURNAL_PRIORITIES: &[(&str, &str)] = &[
    ("emerg", "Emergency"),
    ("alert", "Alert and worse"),
    ("crit", "Critical and worse"),
    ("err", "Error and worse (default)"),
    ("warning", "Warning and worse"),
];

/// A configured priority normalised to its journalctl `-p` value, or `None`
/// when it is not one of [`JOURNAL_PRIORITIES`] (a typo such as `warn`).
pub fn known_journal_priority(value: &str) -> Option<&'static str> {
    let want = value.trim().to_ascii_lowercase();
    JOURNAL_PRIORITIES
        .iter()
        .map(|(v, _)| *v)
        .find(|v| *v == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_priorities_normalise_and_typos_are_rejected() {
        assert_eq!(known_journal_priority("warning"), Some("warning"));
        assert_eq!(known_journal_priority(" CRIT "), Some("crit"));
        assert_eq!(known_journal_priority("warn"), None);
        assert_eq!(known_journal_priority(""), None);
        assert_eq!(
            known_journal_priority(&CrashesConfig::default().journal_priority),
            Some("err")
        );
    }
}
