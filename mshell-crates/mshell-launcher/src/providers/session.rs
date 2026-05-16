//! Power / session actions — lock, suspend, hibernate, reboot,
//! logout, shutdown, etc.
//!
//! Each action carries a command vector that the UI dispatches via
//! `Command::spawn`. The session widget elsewhere in mshell calls
//! the same underlying tools (`loginctl`, `systemctl`, the
//! compositor's lockscreen IPC) so behaviour stays consistent
//! regardless of how the user triggered the action.

use crate::{item::LauncherItem, provider::Provider};
use std::process::Command;
use std::rc::Rc;

/// Canonical id for each session action — matches the strings the
/// existing `mshell-session` config schema already uses, so a
/// provider activation is wire-compatible with the session menu's
/// dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActionId {
    Lock,
    Suspend,
    Hibernate,
    Reboot,
    RebootToUefi,
    UserspaceReboot,
    Logout,
    Shutdown,
}

impl SessionActionId {
    /// Stable string id used in config + on-disk frecency keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Suspend => "suspend",
            Self::Hibernate => "hibernate",
            Self::Reboot => "reboot",
            Self::RebootToUefi => "reboot-to-uefi",
            Self::UserspaceReboot => "userspace-reboot",
            Self::Logout => "logout",
            Self::Shutdown => "shutdown",
        }
    }
}

/// One concrete session action — id + display metadata + the
/// command to spawn. Constructed by the UI from config (so the user
/// can override the lock command) and handed to the provider as a
/// flat slice.
#[derive(Debug, Clone)]
pub struct SessionAction {
    pub id: SessionActionId,
    pub label: String,
    pub icon: String,
    pub keywords: Vec<String>,
    pub command: Vec<String>,
}

impl SessionAction {
    /// Sensible defaults for the eight actions noctalia/DMS ship.
    /// The UI replaces these with the user's config values where
    /// they exist.
    pub fn defaults() -> Vec<SessionAction> {
        use SessionActionId::*;
        vec![
            Self {
                id: Lock,
                label: "Lock".into(),
                icon: "system-lock-screen-symbolic".into(),
                keywords: vec!["lock".into(), "screen".into(), "secure".into()],
                command: vec!["loginctl".into(), "lock-session".into()],
            },
            Self {
                id: Suspend,
                label: "Suspend".into(),
                icon: "system-suspend-symbolic".into(),
                keywords: vec!["suspend".into(), "sleep".into(), "standby".into()],
                command: vec!["systemctl".into(), "suspend".into()],
            },
            Self {
                id: Hibernate,
                label: "Hibernate".into(),
                icon: "system-hibernate-symbolic".into(),
                keywords: vec!["hibernate".into(), "disk".into()],
                command: vec!["systemctl".into(), "hibernate".into()],
            },
            Self {
                id: Reboot,
                label: "Reboot".into(),
                icon: "system-reboot-symbolic".into(),
                keywords: vec!["reboot".into(), "restart".into(), "reload".into()],
                command: vec!["systemctl".into(), "reboot".into()],
            },
            Self {
                id: RebootToUefi,
                label: "Reboot to UEFI".into(),
                icon: "system-reboot-symbolic".into(),
                keywords: vec![
                    "reboot".into(),
                    "uefi".into(),
                    "firmware".into(),
                    "bios".into(),
                ],
                command: vec![
                    "systemctl".into(),
                    "reboot".into(),
                    "--firmware-setup".into(),
                ],
            },
            Self {
                id: UserspaceReboot,
                label: "Userspace reboot".into(),
                icon: "view-refresh-symbolic".into(),
                keywords: vec!["reboot".into(), "restart".into(), "userspace".into()],
                command: vec!["systemctl".into(), "soft-reboot".into()],
            },
            Self {
                id: Logout,
                label: "Log out".into(),
                icon: "system-log-out-symbolic".into(),
                keywords: vec![
                    "logout".into(),
                    "log".into(),
                    "out".into(),
                    "sign".into(),
                    "exit".into(),
                ],
                command: vec!["loginctl".into(), "terminate-session".into(), "self".into()],
            },
            Self {
                id: Shutdown,
                label: "Shut down".into(),
                icon: "system-shutdown-symbolic".into(),
                keywords: vec![
                    "shutdown".into(),
                    "power".into(),
                    "off".into(),
                    "poweroff".into(),
                ],
                command: vec!["systemctl".into(), "poweroff".into()],
            },
        ]
    }
}

pub struct SessionProvider {
    actions: Vec<SessionAction>,
}

impl SessionProvider {
    /// Build with the default action set. Use
    /// [`SessionProvider::with_actions`] in production code to
    /// honour the user's session config.
    pub fn new() -> Self {
        Self::with_actions(SessionAction::defaults())
    }

    pub fn with_actions(actions: Vec<SessionAction>) -> Self {
        Self { actions }
    }
}

impl Default for SessionProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Score an action against a query. Returns scores in nucleo's
/// 0..~200 range so the runtime's global sort interleaves session
/// hits with Apps fuzzy matches naturally — see the table in
/// [`crate::providers::mctl::match_score`] for the constants.
fn match_score(action: &SessionAction, query: &str) -> f64 {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return 0.0;
    }
    let label = action.label.to_ascii_lowercase();
    if label.starts_with(&q) {
        return 180.0;
    }
    let mut best: f64 = -1.0;
    if label.contains(&q) {
        best = best.max(130.0);
    }
    for kw in &action.keywords {
        let lower = kw.to_ascii_lowercase();
        if lower.starts_with(&q) {
            best = best.max(150.0);
        } else if lower.contains(&q) {
            best = best.max(90.0);
        }
    }
    best
}

impl Provider for SessionProvider {
    fn name(&self) -> &str {
        "Session"
    }

    fn commands(&self) -> Vec<LauncherItem> {
        // Advertised in the bare `>` palette so users discover
        // the session shortcuts without remembering keyword
        // queries. Doesn't actually do anything when activated
        // from the palette — the user types the keyword next.
        vec![LauncherItem {
            id: "session:palette".into(),
            name: "Session actions".into(),
            description: "Type lock / suspend / reboot / shutdown / …".into(),
            icon: "system-shutdown-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Session".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim();
        if q.is_empty() {
            // Don't pollute the empty-query browse list with eight
            // power actions — surface them only when the user
            // actually types.
            return Vec::new();
        }
        self.actions
            .iter()
            .filter_map(|action| {
                let score = match_score(action, q);
                if score < 0.0 {
                    return None;
                }
                let command = action.command.clone();
                let id_str = action.id.as_str();
                Some(LauncherItem {
                    id: format!("session:{id_str}"),
                    name: action.label.clone(),
                    description: "Session action".into(),
                    icon: action.icon.clone(),
                    icon_is_path: false,
                    score,
                    provider_name: "Session".into(),
                    usage_key: Some(format!("session:{id_str}")),
                    on_activate: Rc::new(move || run(&command)),
                })
            })
            .collect()
    }
}

fn run(command: &[String]) {
    let Some((bin, args)) = command.split_first() else {
        return;
    };
    if let Err(err) = Command::new(bin).args(args).spawn() {
        tracing::warn!(?err, ?command, "session action spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_eight_actions() {
        assert_eq!(SessionAction::defaults().len(), 8);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let p = SessionProvider::new();
        assert!(p.search("").is_empty());
    }

    #[test]
    fn label_prefix_beats_keyword_match() {
        let p = SessionProvider::new();
        let lock_items = p.search("lock");
        // "lock" is the prefix of label "Lock" → score 180 on
        // the nucleo-comparable scale.
        assert!(lock_items.iter().any(|i| i.name == "Lock"));
        assert!(lock_items[0].score >= 150.0);
    }

    #[test]
    fn keyword_match_finds_action_with_distinct_label() {
        let p = SessionProvider::new();
        let items = p.search("sleep");
        // "sleep" only appears as a keyword on Suspend.
        assert!(items.iter().any(|i| i.name == "Suspend"));
    }

    #[test]
    fn nonmatching_query_returns_empty() {
        let p = SessionProvider::new();
        assert!(p.search("xylophone").is_empty());
    }
}
