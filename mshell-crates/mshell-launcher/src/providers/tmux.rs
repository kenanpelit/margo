//! `mtm <query>` — fuzzy-search running tmux sessions and attach to one
//! from the launcher. Forked from the `dunarand/tmux-provider` Noctalia
//! plugin's idea (`/tm <query>` → session list → attach), reworked to
//! match this launcher's prefix convention (no `/`, see the `ssh`
//! provider) and to hand off to `mtm session create <name>` — our own
//! create-or-attach session manager — instead of a bare `tmux attach`, so
//! a session that no longer exists still does something sensible.
//!
//! `search()` runs `tmux list-sessions` directly rather than shelling out
//! to `mtm` on every keystroke (same reasoning as the `ssh` provider
//! reading `assh.yml` directly instead of going through another binary);
//! `mtm` is only invoked on activation, inside the spawned terminal.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

/// One running session, projected from `tmux list-sessions -F …`.
#[derive(Debug, Clone)]
struct Session {
    name: String,
    windows: String,
    attached: bool,
}

pub struct TmuxProvider {
    terminal: String,
}

impl TmuxProvider {
    pub fn new() -> Self {
        let terminal = std::env::var("TERMINAL")
            .ok()
            .or_else(|| {
                ["kitty", "alacritty", "foot", "wezterm"]
                    .iter()
                    .find(|t| which_exists(t))
                    .map(|t| t.to_string())
            })
            .unwrap_or_else(|| "kitty".into());
        Self { terminal }
    }

    fn list_sessions(&self) -> Vec<Session> {
        let out = Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_windows}\t#{?session_attached,1,0}",
            ])
            .output();
        let Ok(out) = out else { return Vec::new() };
        if !out.status.success() {
            // Non-zero covers both "tmux not installed" and "no server
            // running" (an empty session list, not an error condition).
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let mut cols = line.splitn(3, '\t');
                let name = cols.next()?.to_string();
                let windows = cols.next()?.to_string();
                let attached = cols.next() == Some("1");
                Some(Session {
                    name,
                    windows,
                    attached,
                })
            })
            .collect()
    }
}

impl Default for TmuxProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn which_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

impl Provider for TmuxProvider {
    fn name(&self) -> &str {
        "Tmux"
    }

    fn category(&self) -> &str {
        // Collapses into the shared "Actions" tab alongside ssh (Connect).
        "Connect"
    }

    fn handles_search(&self) -> bool {
        // "mtm" could plausibly be an app/window name; stay out of the
        // regular search path and require the explicit prefix.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "mtm" || q.starts_with("mtm ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "mtm:palette".into(),
            name: "mtm".into(),
            description: "Attach to a running tmux session".into(),
            icon: "utilities-terminal-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Tmux".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "mtm" || q.starts_with("mtm ")) {
            return Vec::new();
        }
        let filter = q.trim_start_matches("mtm").trim().to_ascii_lowercase();

        let sessions = self.list_sessions();
        if sessions.is_empty() {
            return vec![LauncherItem {
                id: "mtm:none".into(),
                name: "No tmux sessions running".into(),
                description: "Enter creates a new session with this name".into(),
                icon: "dialog-information-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Tmux".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        let terminal = self.terminal.clone();
        sessions
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_ascii_lowercase().contains(&filter))
            .enumerate()
            .map(|(idx, s)| {
                let name = s.name.clone();
                let description = format!(
                    "{} window(s), {}",
                    s.windows,
                    if s.attached { "attached" } else { "detached" }
                );
                let terminal_clone = terminal.clone();
                let name_for_toast = name.clone();
                LauncherItem {
                    id: format!("mtm:{}", s.name),
                    name: format!("mtm {}", s.name),
                    description,
                    icon: "utilities-terminal-symbolic".into(),
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "Tmux".into(),
                    usage_key: Some(format!("mtm:{}", s.name)),
                    on_activate: Rc::new(move || {
                        spawn_terminal_attach(&terminal_clone, &name);
                        toast("Tmux", format!("Attaching to {name_for_toast}"));
                    }),
                }
            })
            .collect()
    }

    /// Connect tab — list every running session without requiring the
    /// `mtm ` prefix; `filter` narrows by session name.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("mtm")
        } else {
            self.search(&format!("mtm {filter}"))
        }
    }
}

/// Spawn `<terminal> -e mtm session create <name>` — create-or-attach, so
/// a session that raced closed between listing and activation still does
/// something sensible instead of erroring.
fn spawn_terminal_attach(terminal: &str, session: &str) {
    let result = if terminal == "wezterm" {
        Command::new(terminal)
            .args(["start", "--", "mtm", "session", "create", session])
            .spawn()
    } else {
        Command::new(terminal)
            .args(["-e", "mtm", "session", "create", session])
            .spawn()
    };
    if let Err(err) = result {
        tracing::warn!(?err, terminal, session, "tmux provider spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_command_only_for_mtm_prefix() {
        let p = TmuxProvider::new();
        assert!(p.handles_command("mtm"));
        assert!(p.handles_command("mtm work"));
        assert!(!p.handles_command("mtmux"));
        assert!(!p.handles_command(":mtm"));
    }

    #[test]
    fn does_not_handle_regular_search() {
        let p = TmuxProvider::new();
        assert!(!p.handles_search());
    }

    #[test]
    fn category_is_connect() {
        let p = TmuxProvider::new();
        assert_eq!(p.category(), "Connect");
    }

    #[test]
    fn non_command_query_returns_nothing() {
        let p = TmuxProvider::new();
        assert!(p.search("firefox").is_empty());
    }
}
