//! `>cmd <expression>` — run a one-shot shell command, remember
//! it.
//!
//! The provider activates when the query starts with `>cmd`. The
//! remainder is dispatched to `sh -c` via a detached child so the
//! launcher can close without leaving a zombie. Each run appends
//! to a persistent MRU [`CommandHistory`] (see
//! `~/.cache/margo/launcher_command_history.json`) so the
//! launcher learns the user's vocabulary and can offer prior
//! commands again without retyping.
//!
//! ## Query → result behaviour
//!
//! | Query | What the user sees |
//! |---|---|
//! | `>cmd` (bare) | The whole history list, MRU first |
//! | `>cmd <partial>` | History entries containing `<partial>` + a "Run: <partial>" live row |
//! | `>cmd <full>` | Same — even an exact match still shows the run row at the top |
//!
//! The live "Run" row is always present (score 1.5) so pressing
//! Enter on a brand-new command runs it; history rows score
//! 1.0..1.4 so recent reuse outranks deeper history without
//! beating the live row.

use crate::{history::CommandHistory, item::LauncherItem, notify::toast, provider::Provider};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

pub struct CommandProvider {
    history: Rc<RefCell<CommandHistory>>,
}

impl CommandProvider {
    /// Use the on-disk history file (default location). For
    /// tests, prefer [`CommandProvider::with_history`].
    pub fn new() -> Self {
        Self::with_history(Rc::new(RefCell::new(CommandHistory::load())))
    }

    /// Inject a history store — used by tests to pin the backing
    /// file and by the UI to share the store with other
    /// provider/runtime code that wants to flush it on close.
    pub fn with_history(history: Rc<RefCell<CommandHistory>>) -> Self {
        Self { history }
    }

    /// Shared handle to the underlying history store. The UI uses
    /// this to flush on launcher close.
    pub fn history(&self) -> Rc<RefCell<CommandHistory>> {
        self.history.clone()
    }
}

impl Default for CommandProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CommandProvider {
    fn name(&self) -> &str {
        "Command"
    }

    fn category(&self) -> &str {
        "Run"
    }

    fn handles_search(&self) -> bool {
        // Only contributes through commands() / handles_command —
        // a stray ">" inside an app name shouldn't trigger shell
        // execution.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with(">cmd")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "cmd:cmd".into(),
            name: ">cmd".into(),
            description: "Run a shell command".into(),
            icon: "utilities-terminal-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Command".into(),
            usage_key: None,
            // No-op on its own — picking this from the bare ">"
            // palette is a discoverability cue; the UI is
            // expected to set the search text to ">cmd " so the
            // user can type their expression.
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();
        if !trimmed.starts_with(">cmd") {
            return Vec::new();
        }
        let expression = trimmed.trim_start_matches(">cmd").trim();
        let mut results: Vec<LauncherItem> = Vec::new();

        // Live "Run: …" row first when the user actually typed
        // an expression. Score 1.5 keeps it above history (1.0+).
        if !expression.is_empty() {
            let payload = expression.to_string();
            let history = self.history.clone();
            let history_payload = expression.to_string();
            results.push(LauncherItem {
                id: "cmd:run".into(),
                name: format!("Run: {expression}"),
                description: "Press Enter to execute".into(),
                icon: "utilities-terminal-symbolic".into(),
                icon_is_path: false,
                score: 200.0,
                provider_name: "Command".into(),
                usage_key: None,
                on_activate: Rc::new(move || {
                    spawn_shell(&payload);
                    history.borrow_mut().bump(&history_payload);
                    history.borrow_mut().flush();
                    toast("Command launched", format!("$ {payload}"));
                }),
            });
        }

        // History rows. Filter by case-insensitive substring
        // when the user has typed an expression; show everything
        // when bare `>cmd` was typed. Score tapers from 1.4 down
        // by position so MRU stays at the top.
        let history = self.history.borrow();
        let query_lower = expression.to_ascii_lowercase();
        for (idx, entry) in history.entries().iter().enumerate() {
            if !expression.is_empty()
                && !entry.to_ascii_lowercase().contains(&query_lower)
            {
                continue;
            }
            // Don't duplicate the live row if the history entry
            // exactly matches what the user typed — the live row
            // is canonical.
            if entry == expression {
                continue;
            }
            let payload = entry.clone();
            let history_handle = self.history.clone();
            let history_payload = entry.clone();
            // MRU starts at 140 (just below the live row at 200),
            // dropping by 1 per position so the 100th history
            // entry still scores 40 (well above pure-fuzzy app
            // matches that didn't even share a keyword).
            let score = (140.0 - idx as f64).max(40.0);
            results.push(LauncherItem {
                id: format!("cmd:history:{idx}"),
                name: format!("Run: {entry}"),
                description: "From history — press Enter to re-run".into(),
                icon: "document-open-recent-symbolic".into(),
                icon_is_path: false,
                score,
                provider_name: "Command".into(),
                usage_key: None,
                on_activate: Rc::new(move || {
                    spawn_shell(&payload);
                    history_handle.borrow_mut().bump(&history_payload);
                    history_handle.borrow_mut().flush();
                    toast("Command launched", format!("$ {payload}"));
                }),
            });
        }

        results
    }

    /// Only history rows can be deleted — the live "Run: …" row
    /// the user is currently composing isn't a stored entry.
    fn can_delete(&self, item: &LauncherItem) -> bool {
        item.id.starts_with("cmd:history:")
    }

    /// Drop the matching history entry. The row's display name is
    /// `"Run: <expression>"` — strip the prefix to recover the
    /// original expression and ask the history store to forget it.
    fn delete_item(&mut self, item: &LauncherItem) {
        if let Some(expr) = item.name.strip_prefix("Run: ") {
            self.history.borrow_mut().forget(expr);
        }
    }

    /// Run tab — surface the command history without the `>cmd`
    /// prefix so the user can browse past commands and rerun them
    /// with a single keystroke.
    fn browse(&self) -> Vec<LauncherItem> {
        self.search(">cmd")
    }
}

fn spawn_shell(expression: &str) {
    if let Err(err) = Command::new("sh").arg("-c").arg(expression).spawn() {
        tracing::warn!(?err, "command provider sh -c spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ephemeral_history(name: &str) -> Rc<RefCell<CommandHistory>> {
        let path: PathBuf = std::env::temp_dir().join(format!(
            "mshell_launcher_cmdprov_{}_{}.json",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_file(&path);
        Rc::new(RefCell::new(CommandHistory::load_from(path)))
    }

    #[test]
    fn does_not_handle_regular_search() {
        let p = CommandProvider::with_history(ephemeral_history("regsearch"));
        assert!(p.search("hello").is_empty());
    }

    #[test]
    fn bare_cmd_with_empty_history_returns_nothing() {
        let p = CommandProvider::with_history(ephemeral_history("emptyhist"));
        assert!(p.search(">cmd").is_empty());
        assert!(p.search(">cmd  ").is_empty());
    }

    #[test]
    fn expression_produces_live_run_item_first() {
        let p = CommandProvider::with_history(ephemeral_history("liverun"));
        let items = p.search(">cmd echo hi");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "cmd:run");
        assert_eq!(items[0].name, "Run: echo hi");
        assert_eq!(items[0].score, 200.0);
    }

    #[test]
    fn bare_cmd_after_runs_lists_history() {
        let history = ephemeral_history("listhist");
        history.borrow_mut().bump("ls");
        history.borrow_mut().bump("vim");
        let p = CommandProvider::with_history(history);
        let items = p.search(">cmd");
        // MRU order: vim first, ls second.
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Run: vim");
        assert_eq!(items[1].name, "Run: ls");
    }

    #[test]
    fn partial_filter_matches_history_substring() {
        let history = ephemeral_history("partialhist");
        history.borrow_mut().bump("git status");
        history.borrow_mut().bump("git log");
        history.borrow_mut().bump("ls -la");
        let p = CommandProvider::with_history(history);
        let items = p.search(">cmd git");
        // Live "Run: git" row + two git history rows. ls-la
        // shouldn't show.
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Run: git"));
        assert!(names.contains(&"Run: git log"));
        assert!(names.contains(&"Run: git status"));
        assert!(!names.iter().any(|n| n.contains("ls -la")));
    }

    #[test]
    fn exact_match_does_not_duplicate_history_row() {
        let history = ephemeral_history("nodup");
        history.borrow_mut().bump("echo hi");
        let p = CommandProvider::with_history(history);
        let items = p.search(">cmd echo hi");
        // Only the live "Run: echo hi" row — no duplicate
        // history entry.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "cmd:run");
    }

    #[test]
    fn handles_command_only_for_cmd_prefix() {
        let p = CommandProvider::with_history(ephemeral_history("prefix"));
        assert!(p.handles_command(">cmd ls"));
        assert!(!p.handles_command(">clip"));
        assert!(!p.handles_command("ls"));
    }
}
