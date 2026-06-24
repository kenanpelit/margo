//! `>run <expression>` — run a PATH executable or a one-shot shell
//! command, and remember it.
//!
//! The provider activates when the query starts with `>run`. A
//! free-form expression is dispatched to `sh -c` via a detached child so
//! the launcher can close without leaving a zombie, and each run appends to
//! a persistent MRU [`CommandHistory`] (see
//! `~/.cache/margo/launcher_command_history.json`) so the launcher learns
//! the user's vocabulary and can offer prior commands again without
//! retyping. A single bare token additionally completes against `$PATH`
//! executables (cached in [`CommandProvider::entries`], refreshed on open),
//! so `>run fire` surfaces `firefox`/`firejail` without retyping the full
//! name.
//!
//! ## Query → result behaviour
//!
//! | Query | What the user sees |
//! |---|---|
//! | `>run` (bare) | The whole command history, MRU first |
//! | `>run <token>` | Matching `$PATH` executables + a "Run: <token>" live row + history hits |
//! | `>run <name>` | The exact-named executable scores highest (Enter launches it directly) |
//! | `>run <expr with spaces>` | Free-form shell line: the "Run: …" live row + history hits (no PATH completion) |
//!
//! Scores: an exact PATH-executable name is 250 (Enter spawns the binary by
//! absolute path); the live "Run: …" shell row is 200; partial PATH matches
//! are 100 (the runtime's frecency boost lifts frequently-launched binaries
//! above the rest); history rows score 40..140 by recency.

use crate::{history::CommandHistory, item::LauncherItem, notify::toast, provider::Provider};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

/// One indexed `$PATH` executable, cached after [`CommandProvider::refresh`]
/// so each keystroke is a pure in-memory filter — no `readdir` in the hot
/// path.
#[derive(Debug, Clone)]
struct ExecEntry {
    /// Bare command name as the user types it (`firefox`).
    name: String,
    /// Resolved absolute path — spawning by absolute path means a custom
    /// `$PATH` at launch time doesn't matter.
    path: PathBuf,
}

pub struct CommandProvider {
    history: Rc<RefCell<CommandHistory>>,
    /// Every executable on `$PATH`, refreshed on `on_opened()`. Drives the
    /// single-token completion in [`CommandProvider::search`]
    /// (`>run fire` → `firefox`, `firejail`, …).
    entries: RefCell<Vec<ExecEntry>>,
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
        Self {
            history,
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Shared handle to the underlying history store. The UI uses
    /// this to flush on launcher close.
    pub fn history(&self) -> Rc<RefCell<CommandHistory>> {
        self.history.clone()
    }

    /// Walk every `$PATH` directory and cache the executables, deduped by
    /// name (first hit wins, matching shell lookup). Sorted by name for a
    /// stable order before scoring + frecency reorder.
    fn refresh(&self) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut execs: Vec<ExecEntry> = Vec::new();
        let path = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    continue;
                }
                let name = match entry.file_name().into_string() {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if !seen.insert(name.clone()) {
                    continue;
                }
                if !is_executable(&entry.path()) {
                    continue;
                }
                let path = entry.path();
                execs.push(ExecEntry { name, path });
            }
        }
        execs.sort_by(|a, b| a.name.cmp(&b.name));
        *self.entries.borrow_mut() = execs;
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
        query.trim_start().starts_with(">run")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "cmd:cmd".into(),
            name: ">run".into(),
            description: "Run a PATH executable or shell command".into(),
            icon: "utilities-terminal-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Command".into(),
            usage_key: None,
            // No-op on its own — picking this from the bare ">"
            // palette is a discoverability cue; the UI is
            // expected to set the search text to ">run " so the
            // user can type their expression.
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();
        if !trimmed.starts_with(">run") {
            return Vec::new();
        }
        let expression = trimmed.trim_start_matches(">run").trim();
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

        // PATH completion: a single bare token (`>run fire`) is a command
        // *name* the user is still typing, so offer matching $PATH
        // executables (`firefox`, `firejail`, …). A multi-token expression
        // (`>run firefox --private-window`) is a free-form shell line — leave
        // it to the literal Run row above and skip completion.
        if !expression.is_empty() && !expression.contains(char::is_whitespace) {
            let needle = expression.to_ascii_lowercase();
            let entries = self.entries.borrow();
            let mut matches: Vec<&ExecEntry> = entries
                .iter()
                .filter(|e| e.name.to_ascii_lowercase().contains(&needle))
                .collect();
            // Exact name first, then alphabetical; cap so a 1-char filter
            // can't spill thousands of rows into the list.
            matches.sort_by(|a, b| {
                let ae = a.name.eq_ignore_ascii_case(expression);
                let be = b.name.eq_ignore_ascii_case(expression);
                be.cmp(&ae).then_with(|| a.name.cmp(&b.name))
            });
            for entry in matches.into_iter().take(80) {
                // Exact match scores 250 — above the literal Run row (200) —
                // so typing a full binary name and pressing Enter launches it
                // directly (a reliable abs-path spawn) instead of `sh -c`.
                // Partial matches score 100; the runtime's frecency boost
                // surfaces frequently-launched binaries above the rest.
                let score = if entry.name.eq_ignore_ascii_case(expression) {
                    250.0
                } else {
                    100.0
                };
                let path = entry.path.clone();
                let name = entry.name.clone();
                let bin_name = entry.name.clone();
                results.push(LauncherItem {
                    id: format!("run-bin:{}", entry.name),
                    name: entry.name.clone(),
                    description: entry.path.display().to_string(),
                    icon: "application-x-executable-symbolic".into(),
                    icon_is_path: false,
                    score,
                    provider_name: "Command".into(),
                    usage_key: Some(format!("run-bin:{name}")),
                    on_activate: Rc::new(move || {
                        if let Err(err) = Command::new(&path).spawn() {
                            tracing::warn!(?err, bin = %bin_name, "run provider spawn failed");
                        } else {
                            toast(format!("Launched {bin_name}"), "");
                        }
                    }),
                });
            }
        }

        // History rows. Filter by case-insensitive substring
        // when the user has typed an expression; show everything
        // when bare `>run` was typed. Score tapers from 1.4 down
        // by position so MRU stays at the top.
        let history = self.history.borrow();
        let query_lower = expression.to_ascii_lowercase();
        for (idx, entry) in history.entries().iter().enumerate() {
            if !expression.is_empty() && !entry.to_ascii_lowercase().contains(&query_lower) {
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

    /// Run tab — surface the command history (and, while typing a single
    /// token, matching PATH executables) without the `>run` prefix;
    /// non-empty `filter` also becomes the live "Run: …" row (so the Run tab
    /// doubles as a command-line shell entry).
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search(">run")
        } else {
            self.search(&format!(">run {filter}"))
        }
    }

    fn on_opened(&mut self) {
        // Re-scan $PATH on every open — a handful of readdirs — so an
        // executable the user just dropped on PATH (or chmod +x'd) shows up
        // in `>run` completion immediately.
        self.refresh();
    }
}

fn spawn_shell(expression: &str) {
    if let Err(err) = Command::new("sh").arg("-c").arg(expression).spawn() {
        tracing::warn!(?err, "command provider sh -c spawn failed");
    }
}

/// Whether `path` has at least one executable bit set — the final gate so a
/// non-`+x` file symlinked onto `$PATH` doesn't show up as a runnable. (Twin
/// of the same helper in the scripts provider.)
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
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
        assert!(p.search(">run").is_empty());
        assert!(p.search(">run  ").is_empty());
    }

    #[test]
    fn expression_produces_live_run_item_first() {
        let p = CommandProvider::with_history(ephemeral_history("liverun"));
        let items = p.search(">run echo hi");
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
        let items = p.search(">run");
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
        let items = p.search(">run git");
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
        let items = p.search(">run echo hi");
        // Only the live "Run: echo hi" row — no duplicate
        // history entry.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "cmd:run");
    }

    #[test]
    fn handles_command_only_for_cmd_prefix() {
        let p = CommandProvider::with_history(ephemeral_history("prefix"));
        assert!(p.handles_command(">run ls"));
        assert!(!p.handles_command(">clip"));
        assert!(!p.handles_command("ls"));
    }

    #[test]
    fn run_completes_path_executables() {
        let mut p = CommandProvider::with_history(ephemeral_history("pathcomplete"));
        // No scan yet → no completion rows even for a single token.
        assert!(
            p.search(">run sh")
                .iter()
                .all(|i| !i.id.starts_with("run-bin:"))
        );
        p.on_opened(); // scan $PATH
        // Pick a real executable off PATH and confirm it completes, with the
        // exact-name row scoring 250 so Enter launches it directly.
        let sample = p.entries.borrow().first().cloned();
        if let Some(sample) = sample {
            let items = p.search(&format!(">run {}", sample.name));
            assert!(
                items
                    .iter()
                    .any(|i| i.id == format!("run-bin:{}", sample.name) && i.score == 250.0),
                "exact PATH match should appear at score 250"
            );
        }
        // A multi-token expression is a free-form shell line — no completion.
        assert!(
            p.search(">run sh -c true")
                .iter()
                .all(|i| !i.id.starts_with("run-bin:"))
        );
    }
}
