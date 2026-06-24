//! `>start <name>` — index user-defined start-* scripts on $PATH
//! and run them by short name.
//!
//! Scans every directory on `$PATH` for executables whose names
//! match a configurable prefix (default `start-`) so the user can
//! reach their own launcher scripts (`start-brave-ai`,
//! `start-kitty-single`, …) without remembering the full name or
//! the directory.
//!
//! ## Behaviour
//!
//! | Query | What the user sees |
//! |---|---|
//! | `>start` (bare) | Every matching script, sorted by frecency |
//! | `>start <partial>` | Filtered to names containing the substring |
//!
//! The provider's `handles_search` returns `false` so empty-query
//! browse is left to Apps + Windows — a flood of 40+ scripts at
//! the top of the everyday list would drown the useful entries.
//! Surface them deliberately via the `>start` prefix.
//!
//! Each script gets `usage_key = "scripts:<name>"` so the
//! frecency store ranks frequently-launched scripts to the top
//! of the `>start` palette.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::{cell::RefCell, collections::HashSet, path::PathBuf, process::Command, rc::Rc};

/// One indexed script. Cached after `refresh()` so subsequent
/// `>start` keystrokes are a pure in-memory filter — no `readdir`
/// in the search hot path.
#[derive(Debug, Clone)]
struct ScriptEntry {
    /// Short name as the user types it (`start-brave-ai`).
    name: String,
    /// Resolved absolute path (`~/.local/share/zinit/polaris/bin/start-brave-ai`).
    /// Spawning by absolute path means a custom $PATH at launch
    /// doesn't matter.
    path: PathBuf,
}

pub struct ScriptsProvider {
    /// Prefix every script name must start with. Lives behind a
    /// `Cell<String>` so the Settings page can swap it without
    /// rebuilding the provider; for now it's set at construction
    /// time and never changed.
    prefix: String,
    /// Cached entries — refreshed on every `on_opened()`.
    entries: RefCell<Vec<ScriptEntry>>,
}

impl ScriptsProvider {
    /// Default prefix the provider scans for. Surfaced as a
    /// public constant so the Settings UI can show the active
    /// value without instantiating the provider.
    pub const DEFAULT_PREFIX: &'static str = "start-";

    /// Use the default `start-` prefix.
    pub fn new() -> Self {
        Self::with_prefix(Self::DEFAULT_PREFIX)
    }

    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        let me = Self {
            prefix: prefix.into(),
            entries: RefCell::new(Vec::new()),
        };
        me.refresh();
        me
    }

    /// Snapshot of the currently-indexed scripts as
    /// `(name, path)` pairs. The Settings UI uses this to list
    /// what's discoverable through `>start`.
    pub fn indexed(&self) -> Vec<(String, PathBuf)> {
        self.entries
            .borrow()
            .iter()
            .map(|e| (e.name.clone(), e.path.clone()))
            .collect()
    }

    /// Just the script names, for compact display. Cheap because
    /// it just clones already-cached strings.
    pub fn indexed_names(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    /// Walk every directory on `$PATH` and collect executables
    /// matching the configured prefix. Names are deduplicated
    /// (first hit wins) so the same script reachable through two
    /// PATH entries doesn't show twice.
    pub fn refresh(&self) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut scripts: Vec<ScriptEntry> = Vec::new();

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
                if !name.starts_with(&self.prefix) {
                    continue;
                }
                if !seen.insert(name.clone()) {
                    continue;
                }
                if !is_executable(&entry.path()) {
                    continue;
                }
                scripts.push(ScriptEntry {
                    name,
                    path: entry.path(),
                });
            }
        }
        scripts.sort_by(|a, b| a.name.cmp(&b.name));
        *self.entries.borrow_mut() = scripts;
    }

    fn make_item(&self, entry: &ScriptEntry, score: f64) -> LauncherItem {
        let path = entry.path.clone();
        let name = entry.name.clone();
        let display_name = entry.name.clone();
        LauncherItem {
            id: format!("scripts:{}", entry.name),
            name: entry.name.clone(),
            description: entry.path.display().to_string(),
            icon: "utilities-terminal-symbolic".into(),
            icon_is_path: false,
            score,
            provider_name: "Scripts".into(),
            usage_key: Some(format!("scripts:{name}")),
            on_activate: Rc::new(move || {
                if let Err(err) = Command::new(&path).spawn() {
                    tracing::warn!(?err, script = %display_name, "scripts provider spawn failed");
                } else {
                    toast(format!("Launched {display_name}"), "");
                }
            }),
        }
    }
}

impl Default for ScriptsProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether `path` has at least one executable bit set.
/// Used as the final gate so a binary symlinked into a PATH
/// directory but lacking +x doesn't end up in the launcher.
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

impl Provider for ScriptsProvider {
    fn name(&self) -> &str {
        "Scripts"
    }

    fn category(&self) -> &str {
        "Run"
    }

    fn handles_search(&self) -> bool {
        // Don't pollute the empty-query browse: a 40-script
        // flood at the top of the apps list would bury Firefox
        // & Kitty. Reach scripts deliberately via `>start`.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with(">start")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "scripts:palette".into(),
            name: ">start".into(),
            description: format!("Run a {}* script", self.prefix),
            icon: "utilities-terminal-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Scripts".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();
        if !trimmed.starts_with(">start") {
            return Vec::new();
        }
        let filter = trimmed
            .trim_start_matches(">start")
            .trim()
            .to_ascii_lowercase();

        let entries = self.entries.borrow();
        // Score MRU-ish: substring matches start at 100, the
        // frecency boost the runtime adds (5*log2(1+count)) is what
        // ultimately surfaces most-used scripts to the top.
        // Without a non-zero base score the runtime's stable
        // sort would keep the alphabetical refresh() order which
        // would dilute the frecency signal at small counts.
        //
        // An *exact* name match scores 250 — above the Command
        // provider's generic "Run: <text>" shell row (200) — so when
        // the user types a script's full name and hits Enter it runs
        // the script directly (a reliable absolute-path spawn) instead
        // of the shell-command fallback shadowing it from the top slot.
        entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.name.to_ascii_lowercase().contains(&filter))
            .map(|entry| {
                let score = if !filter.is_empty() && entry.name.to_ascii_lowercase() == filter {
                    250.0
                } else {
                    100.0
                };
                self.make_item(entry, score)
            })
            .collect()
    }

    fn on_opened(&mut self) {
        // Re-scan PATH on every open — cheap enough (handful of
        // readdirs) and ensures new scripts the user added with
        // `chmod +x` between opens show up immediately.
        self.refresh();
    }

    /// Allow Delete on script rows so a mis-bumped frecency entry
    /// (typo'd script name, accidental Enter) can be cleared. The
    /// runtime handles the actual frecency forget; we have no
    /// provider-owned state to drop (scripts come from PATH).
    fn can_delete(&self, item: &crate::LauncherItem) -> bool {
        item.id.starts_with("scripts:")
    }

    /// Run tab — surface every `start-*` script even without the
    /// `>start` prefix; `filter` narrows by name substring.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search(">start")
        } else {
            self.search(&format!(">start {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = ScriptsProvider::with_prefix("start-");
        assert!(p.search("anything").is_empty());
    }

    #[test]
    fn handles_command_only_for_start_prefix() {
        let p = ScriptsProvider::with_prefix("start-");
        assert!(p.handles_command(">start"));
        assert!(p.handles_command(">start brave"));
        assert!(!p.handles_command(">run"));
        assert!(!p.handles_command("start"));
    }

    #[test]
    fn commands_returns_one_palette_entry() {
        let p = ScriptsProvider::with_prefix("start-");
        let cmds = p.commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, ">start");
    }

    #[test]
    fn exact_name_match_outranks_generic_run_row() {
        let p = ScriptsProvider::with_prefix("start-");
        // Pin a deterministic index (refresh() scanned the real PATH).
        *p.entries.borrow_mut() = vec![
            ScriptEntry {
                name: "start-brave-kenp".into(),
                path: "/x/start-brave-kenp".into(),
            },
            ScriptEntry {
                name: "start-brave-ai".into(),
                path: "/x/start-brave-ai".into(),
            },
        ];

        // Exact full-name query → boosted above the Command provider's
        // live "Run: …" row (200) so Enter runs the script directly.
        let exact = p.search(">start start-brave-kenp");
        let hit = exact
            .iter()
            .find(|i| i.name == "start-brave-kenp")
            .expect("exact script present");
        assert!(hit.score > 200.0, "exact match must outrank the Run: row");

        // Substring-only query stays at the base score (below the Run:
        // row), so an ambiguous partial doesn't hijack the shell entry.
        let partial = p.search(">start brave");
        assert!(partial.iter().all(|i| i.score <= 200.0));
    }
}
