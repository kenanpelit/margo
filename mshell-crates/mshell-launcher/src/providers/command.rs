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
//! | `>run ~/Doc` / `/etc/fs` / `./x` | **Path completion**: "Open …" for the typed path, then matching entries of its folder (folders first) |
//!
//! ## Path mode
//!
//! An expression starting with `~`, `/`, `./` or `../` completes like a
//! shell: rows list the entries of the typed folder whose names start
//! with the typed basename (hidden ones only once a `.` is typed).
//! Enter on a folder row descends (`>run ~/Documents/`), Enter on a file
//! opens it with `xdg-open`; the top "Open …" row opens exactly what was
//! typed. Relative paths resolve against `$HOME` — the launcher has no
//! working directory of its own.
//!
//! ## Terminal
//!
//! Interactive programs (`htop`, `nvim`, `ssh`, `sudo …`, REPLs, …) can't
//! run detached, so their Run rows launch inside `$TERMINAL`
//! automatically. Ctrl+Enter on any Run / history / executable row forces
//! a terminal and keeps it open (drops into `$SHELL`) after the command
//! exits so its output stays readable.
//!
//! Scores: an exact PATH-executable name is 250 (Enter spawns the binary by
//! absolute path); the live "Run: …" shell row is 200; partial PATH matches
//! are 100 (the runtime's frecency boost lifts frequently-launched binaries
//! above the rest); history rows score 40..140 by recency.

use crate::providers::tools::tildify;
use crate::{history::CommandHistory, item::LauncherItem, notify::toast, provider::Provider};
use mshell_process::SpawnReaped;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
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
    /// Search-entry setter for descending into folders in path mode.
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
}

/// Programs that need a terminal (TUIs, REPLs, prompts). Matched on the
/// first word's basename.
const INTERACTIVE: &[&str] = &[
    "sudo",
    "su",
    "doas",
    "ssh",
    "mosh",
    "top",
    "htop",
    "btop",
    "atop",
    "nvtop",
    "vim",
    "nvim",
    "vi",
    "nano",
    "micro",
    "hx",
    "helix",
    "emacs",
    "less",
    "more",
    "man",
    "mc",
    "ranger",
    "yazi",
    "nnn",
    "lf",
    "lazygit",
    "tig",
    "ncdu",
    "gdu",
    "python",
    "python3",
    "ipython",
    "node",
    "irb",
    "ghci",
    "psql",
    "mysql",
    "sqlite3",
    "redis-cli",
    "tmux",
    "screen",
    "mtm",
    "bash",
    "zsh",
    "fish",
    "sh",
    "watch",
    "journalctl",
    "nmtui",
    "bluetoothctl",
    "pacman",
    "paru",
    "yay",
    "qalc",
    "bc",
    "claude",
    "codex",
    "k9s",
    "iftop",
    "nethogs",
    "cmus",
    "ncmpcpp",
    "weechat",
    "newsboat",
];

/// Does `expr` need a terminal? Skips leading `VAR=value` assignments
/// and `env`. Pipelines/compound lines are judged by their first command.
pub fn needs_terminal(expr: &str) -> bool {
    let first = expr
        .split_whitespace()
        .find(|w| (w.starts_with('=') || !w.contains('=')) && *w != "env");
    let Some(first) = first else {
        return false;
    };
    let base = first.rsplit('/').next().unwrap_or(first);
    INTERACTIVE.contains(&base)
}

/// Shell line run in the terminal. `hold` keeps it open afterwards in an
/// interactive shell so the output can be read.
pub fn terminal_command(expr: &str, hold: bool) -> Vec<String> {
    let line = if hold {
        format!("{expr}\nstatus=$?\nprintf '\\n[exit %s]\\n' \"$status\"\nexec \"${{SHELL:-sh}}\"")
    } else {
        expr.to_string()
    };
    vec!["sh".into(), "-c".into(), line]
}

fn spawn_in_terminal(expr: &str, hold: bool) {
    let cmd = terminal_command(expr, hold);
    let (terminal, mut command) = match mshell_process::terminal::command(&cmd, "run") {
        Ok(found) => found,
        Err(err) => {
            toast("Run in terminal failed", err.to_string());
            return;
        }
    };
    if let Err(err) = command.spawn_reaped() {
        tracing::warn!(?err, %terminal, "run in terminal failed");
        toast("Run in terminal failed", format!("{terminal}: {err}"));
    }
}

/// Is `expr` a path the user is typing?
pub fn is_path_like(expr: &str) -> bool {
    expr == "~" || ["~/", "/", "./", "../"].iter().any(|p| expr.starts_with(p))
}

/// Resolve a typed path: `~` → home, relative → under home.
pub fn expand_path(expr: &str, home: &Path) -> PathBuf {
    if expr == "~" {
        home.to_path_buf()
    } else if let Some(rest) = expr.strip_prefix("~/") {
        home.join(rest)
    } else if expr.starts_with('/') {
        PathBuf::from(expr)
    } else {
        home.join(expr)
    }
}

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCompletion {
    /// Text to put back after `>run ` (keeps the user's `~/` / `./` form).
    pub typed: String,
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Completions for a typed path: entries of the folder part whose names
/// start (case-insensitively) with the basename part. Hidden entries only
/// when the basename starts with `.`. Folders first, then by name.
pub fn complete_path(expr: &str, home: &Path, limit: usize) -> Vec<PathCompletion> {
    let (dir_typed, base) = match expr.rfind('/') {
        Some(i) => (&expr[..=i], &expr[i + 1..]),
        None => ("~/", ""), // bare `~`
    };
    let dir_typed = if expr == "~" { "~/" } else { dir_typed };
    let dir = expand_path(dir_typed, home);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let base_lower = base.to_lowercase();
    let mut out: Vec<PathCompletion> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !base.starts_with('.') {
                return None;
            }
            if !name.to_lowercase().starts_with(&base_lower) {
                return None;
            }
            let path = e.path();
            let is_dir = path.is_dir(); // follows symlinks
            Some(PathCompletion {
                typed: format!("{dir_typed}{name}{}", if is_dir { "/" } else { "" }),
                name,
                path,
                is_dir,
            })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

fn open_path(path: &Path) {
    if let Err(err) = mshell_process::desktop::open(path) {
        tracing::warn!(?err, path = %path.display(), "run: xdg-open failed");
    }
}

/// Run `expr` the right way: a terminal for interactive programs, else
/// detached `sh -c`.
fn run_expression(expr: &str) {
    if needs_terminal(expr) {
        spawn_in_terminal(expr, false);
    } else {
        spawn_shell(expr);
    }
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
            set_search: None,
        }
    }

    /// Wire the search-entry setter used to descend into folders.
    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
    }

    /// Path-mode rows for `expr` (see module docs).
    fn path_rows(&self, expr: &str) -> Vec<LauncherItem> {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let target = expand_path(expr.trim_end_matches('/'), &home);
        if target.exists() {
            let shown = tildify(&target, &home);
            let open = target.clone();
            rows.push(LauncherItem {
                id: format!("run-path:open:{}", target.display()),
                name: format!("Open {shown}"),
                description: if target.is_dir() {
                    "Open folder in the file manager".into()
                } else {
                    "Open with the default application".into()
                },
                icon: if target.is_dir() {
                    "folder-symbolic"
                } else {
                    "document-open-symbolic"
                }
                .into(),
                icon_is_path: false,
                score: 260.0,
                provider_name: "Command".into(),
                usage_key: None,
                on_activate: Rc::new(move || open_path(&open)),
            });
        }
        // `~/Documents/` lists the folder; `~/Doc` completes the name.
        for (idx, c) in complete_path(expr, &home, 60).into_iter().enumerate() {
            let setter = self.set_search.clone();
            let typed = c.typed.clone();
            let path = c.path.clone();
            let is_dir = c.is_dir;
            rows.push(LauncherItem {
                id: format!("run-path:{}", c.path.display()),
                name: if c.is_dir {
                    format!("{}/", c.name)
                } else {
                    c.name.clone()
                },
                description: c.typed.clone(),
                icon: if c.is_dir {
                    "folder-symbolic"
                } else {
                    "text-x-generic-symbolic"
                }
                .into(),
                icon_is_path: false,
                score: 180.0 - idx as f64,
                provider_name: "Command".into(),
                usage_key: None,
                on_activate: Rc::new(move || match (&setter, is_dir) {
                    (Some(set), true) => set(&format!(">run {typed}")),
                    _ => open_path(&path),
                }),
            });
        }
        rows
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

        if is_path_like(expression) {
            results.extend(self.path_rows(expression));
        }

        // Live "Run: …" row first when the user actually typed
        // an expression. Score 1.5 keeps it above history (1.0+).
        if !expression.is_empty() {
            let interactive = needs_terminal(expression);
            let payload = expression.to_string();
            let history = self.history.clone();
            let history_payload = expression.to_string();
            results.push(LauncherItem {
                id: "cmd:run".into(),
                name: format!("Run: {expression}"),
                description: if interactive {
                    "Interactive — runs in a terminal".into()
                } else {
                    "Enter: execute · Ctrl+Enter: in a terminal".into()
                },
                icon: "utilities-terminal-symbolic".into(),
                icon_is_path: false,
                score: 200.0,
                provider_name: "Command".into(),
                usage_key: None,
                on_activate: Rc::new(move || {
                    run_expression(&payload);
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
                        if needs_terminal(&bin_name) {
                            spawn_in_terminal(&bin_name, false);
                            return;
                        }
                        if let Err(err) = Command::new(&path).spawn_reaped() {
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
                    run_expression(&payload);
                    history_handle.borrow_mut().bump(&history_payload);
                    history_handle.borrow_mut().flush();
                    toast("Command launched", format!("$ {payload}"));
                }),
            });
        }

        results
    }

    /// Ctrl+Enter → run in a terminal that stays open afterwards.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let expr = if item.id == "cmd:run" || item.id.starts_with("cmd:history:") {
            item.name.strip_prefix("Run: ")?.to_string()
        } else {
            item.id.strip_prefix("run-bin:")?.to_string()
        };
        let history = self.history.clone();
        let record = !item.id.starts_with("run-bin:");
        Some(Rc::new(move || {
            spawn_in_terminal(&expr, true);
            if record {
                history.borrow_mut().bump(&expr);
                history.borrow_mut().flush();
            }
        }))
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
    if let Err(err) = Command::new("sh").arg("-c").arg(expression).spawn_reaped() {
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

    #[test]
    fn interactive_detection() {
        for e in [
            "htop",
            "sudo pacman -Syu",
            "/usr/bin/nvim foo",
            "FOO=1 python3",
            "env TERM=x ssh vhay",
        ] {
            assert!(needs_terminal(e), "{e}");
        }
        for e in ["firefox", "notify-send hi", "", "kitty -e htop"] {
            assert!(!needs_terminal(e), "{e}");
        }
    }

    #[test]
    fn terminal_command_holds_only_on_request() {
        assert_eq!(terminal_command("htop", false), vec!["sh", "-c", "htop"]);
        let held = terminal_command("ls -l", true);
        assert!(held[2].starts_with("ls -l\n"));
        assert!(held[2].ends_with("exec \"${SHELL:-sh}\""));
    }

    #[test]
    fn path_like_and_expansion() {
        for e in ["~", "~/Doc", "/etc", "./x", "../y"] {
            assert!(is_path_like(e), "{e}");
        }
        for e in ["ls", "~user", ".bashrc", "echo /etc"] {
            assert!(!is_path_like(e), "{e}");
        }
        let home = Path::new("/home/u");
        assert_eq!(expand_path("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand_path("~/a", home), PathBuf::from("/home/u/a"));
        assert_eq!(expand_path("./a", home), PathBuf::from("/home/u/./a"));
        assert_eq!(expand_path("/etc", home), PathBuf::from("/etc"));
    }

    fn temp_tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mshell-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.join("Downloads")).unwrap();
        std::fs::write(dir.join("doc.txt"), b"x").unwrap();
        std::fs::write(dir.join(".dotfile"), b"x").unwrap();
        dir
    }

    #[test]
    fn completes_folders_first_keeping_typed_form() {
        let home = temp_tree("complete");
        let c = complete_path("~/do", &home, 10);
        let typed: Vec<&str> = c.iter().map(|c| c.typed.as_str()).collect();
        assert_eq!(typed, vec!["~/Documents/", "~/Downloads/", "~/doc.txt"]);
        assert!(
            complete_path("~/", &home, 10)
                .iter()
                .all(|c| !c.name.starts_with('.'))
        );
        assert_eq!(complete_path("~/.d", &home, 10)[0].name, ".dotfile");
        assert_eq!(complete_path("./Doc", &home, 10)[0].typed, "./Documents/");
        assert!(complete_path("~/nope/x", &home, 10).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn absolute_path_rows_descend_into_folders() {
        let dir = temp_tree("rows");
        let captured = Rc::new(RefCell::new(String::new()));
        let cap = captured.clone();
        let p = CommandProvider::with_history(ephemeral_history("pathrows"))
            .with_search_setter(Rc::new(move |t| *cap.borrow_mut() = t.to_string()));
        let rows = p.search(&format!(">run {}/Do", dir.display()));
        let docs = rows.iter().find(|r| r.name == "Documents/").unwrap();
        (docs.on_activate)();
        assert_eq!(
            *captured.borrow(),
            format!(">run {}/Documents/", dir.display())
        );
        let rows = p.search(&format!(">run {}/Documents", dir.display()));
        assert!(rows[0].name.starts_with("Open "));
        // Ctrl+Enter on the live row → terminal action exists.
        let run = rows.iter().find(|r| r.id == "cmd:run").unwrap();
        assert!(p.alt_action(run).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
