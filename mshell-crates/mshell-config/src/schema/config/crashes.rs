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
    /// AI agents the Crashes menu's diagnose button tries, in order:
    /// `claude` (Claude Code) and/or `codex`. Unknown names are ignored; an
    /// empty or all-unknown list means the default `[claude, codex]`.
    pub agent_order: Vec<String>,
    /// Before opening an agent, ask it a throwaway question and move on to
    /// the next one when it cannot answer (out of quota, logged out,
    /// offline). Costs one tiny request per diagnosis.
    pub agent_probe: bool,
    /// Extra instructions appended to every diagnosis prompt, e.g.
    /// "Answer in Turkish". Empty: none.
    pub agent_instructions: String,
}

impl Default for CrashesConfig {
    fn default() -> Self {
        Self {
            notify: false,
            muted: Vec::new(),
            journal_priority: "err".to_string(),
            agent_order: DIAGNOSIS_AGENTS
                .iter()
                .map(|(n, _)| n.to_string())
                .collect(),
            agent_probe: true,
            agent_instructions: String::new(),
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

/// The AI agents the diagnose button knows how to start, in the default
/// order, with their display names.
pub const DIAGNOSIS_AGENTS: &[(&str, &str)] = &[("claude", "Claude Code"), ("codex", "Codex")];

/// `crashes.agent_order` normalised: known names only (case-insensitive),
/// first occurrence wins, and the default order when nothing is left.
pub fn diagnosis_agents(order: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for want in order {
        let want = want.trim().to_ascii_lowercase();
        if let Some((name, _)) = DIAGNOSIS_AGENTS.iter().find(|(n, _)| *n == want)
            && !out.contains(name)
        {
            out.push(name);
        }
    }
    if out.is_empty() {
        out = DIAGNOSIS_AGENTS.iter().map(|(n, _)| *n).collect();
    }
    out
}

/// The agent orders offered in Settings (a dropdown, index = position).
pub const AGENT_ORDER_CHOICES: &[(&[&str], &str)] = &[
    (&["claude", "codex"], "Claude Code, then Codex"),
    (&["codex", "claude"], "Codex, then Claude Code"),
    (&["claude"], "Claude Code only"),
    (&["codex"], "Codex only"),
];

/// The [`AGENT_ORDER_CHOICES`] index matching a configured order (after
/// [`diagnosis_agents`]); a hand-written order that matches none shows as
/// the default.
pub fn agent_order_index(order: &[String]) -> u32 {
    let agents = diagnosis_agents(order);
    AGENT_ORDER_CHOICES
        .iter()
        .position(|(names, _)| *names == agents.as_slice())
        .unwrap_or(0) as u32
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

    #[test]
    fn diagnosis_agents_normalise_dedupe_and_default() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            diagnosis_agents(&v(&["Codex", " claude "])),
            ["codex", "claude"]
        );
        assert_eq!(diagnosis_agents(&v(&["codex", "codex"])), ["codex"]);
        assert_eq!(diagnosis_agents(&v(&["opencode"])), ["claude", "codex"]);
        assert_eq!(diagnosis_agents(&[]), ["claude", "codex"]);
        assert_eq!(
            diagnosis_agents(&CrashesConfig::default().agent_order),
            ["claude", "codex"]
        );
        assert!(CrashesConfig::default().agent_probe);
    }

    #[test]
    fn agent_order_choices_round_trip() {
        for (i, (names, _)) in AGENT_ORDER_CHOICES.iter().enumerate() {
            let order: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            assert_eq!(agent_order_index(&order), i as u32);
        }
        assert_eq!(agent_order_index(&["bogus".to_string()]), 0);
    }
}
