//! `pass <query>` — GNU pass (password-store) launcher provider.
//!
//! Ported from noctalia's `launcher-pass` plugin, rebuilt native: scans
//! the password store directly (no `find` subprocess), AND-token fuzzy
//! filtering ("spaces act as wildcards"), and two actions per entry:
//!
//! | Key | Action |
//! |---|---|
//! | Enter | **Copy** the password — `pass -c`, which auto-clears the clipboard after `PASSWORD_STORE_CLIP_TIME` (45 s default). |
//! | Ctrl+Enter | **Type** the password — `pass show … \| wtype`, so the secret never touches the clipboard (no history trail). |
//!
//! Store location follows pass itself: `$PASSWORD_STORE_DIR`, else
//! `~/.password-store`. Entries are listed by their store-relative path
//! without the `.gpg` suffix (e.g. `web/github`, `email/work`).
//!
//! Security note: `pass -c` does land the secret in the wl-clipboard,
//! which margo's own clipboard history (mshell-clipboard) watches — use
//! Ctrl+Enter (type) when you'd rather keep it out of history, or add
//! `pass` to the clipboard's sensitive-skip list.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

/// Cap the rows built per query so a multi-hundred-entry store stays
/// responsive on the empty/browse view; a filter narrows well below this.
const MAX_RESULTS: usize = 100;

pub struct PassProvider {
    /// Current store directory; re-resolved from `resolver` on each open
    /// so a Settings change to the path applies without a restart.
    store_dir: RefCell<PathBuf>,
    /// Live path resolver (the shell closes over config + env). `None`
    /// for fixed-path construction (standalone / tests).
    resolver: Option<Box<dyn Fn() -> PathBuf>>,
    /// Store-relative entry names (sans `.gpg`), alphabetically sorted.
    entries: RefCell<Vec<String>>,
}

/// `$PASSWORD_STORE_DIR` (if absolute), else `~/.password-store` — pass's
/// own resolution order, with no config layer on top.
pub fn default_store_dir() -> PathBuf {
    std::env::var_os("PASSWORD_STORE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".password-store")))
        .unwrap_or_else(|| PathBuf::from("/home/nobody/.password-store"))
}

impl PassProvider {
    pub fn new() -> Self {
        Self::with_dir(default_store_dir())
    }

    pub fn with_dir(store_dir: PathBuf) -> Self {
        let me = Self {
            store_dir: RefCell::new(store_dir),
            resolver: None,
            entries: RefCell::new(Vec::new()),
        };
        me.refresh();
        me
    }

    /// Construct with a live path resolver, re-read on every launcher open.
    pub fn with_resolver(resolver: Box<dyn Fn() -> PathBuf>) -> Self {
        let me = Self {
            store_dir: RefCell::new(resolver()),
            resolver: Some(resolver),
            entries: RefCell::new(Vec::new()),
        };
        me.refresh();
        me
    }

    pub fn refresh(&self) {
        let dir = self.store_dir.borrow().clone();
        let mut out = Vec::new();
        scan(&dir, &dir, &mut out, 0);
        out.sort();
        *self.entries.borrow_mut() = out;
    }
}

impl Default for PassProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively collect `*.gpg` entries as store-relative paths without the
/// `.gpg` suffix. Skips dotfiles/dirs (`.git`, `.gpg-id`, …) and bounds the
/// recursion depth so a symlink loop or pathological tree can't hang.
fn scan(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 16 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);
        if is_dir {
            scan(root, &path, out, depth + 1);
        } else if let Some(stem) = name.strip_suffix(".gpg") {
            // Rebuild the relative path with the stripped stem so nested
            // entries read as `web/github`, not `web/github.gpg`.
            let rel = path
                .strip_prefix(root)
                .ok()
                .and_then(|r| r.parent().map(|p| p.to_path_buf()))
                .filter(|p| !p.as_os_str().is_empty())
                .map(|parent| format!("{}/{stem}", parent.to_string_lossy()))
                .unwrap_or_else(|| stem.to_string());
            out.push(rel);
        }
    }
}

/// Single-quote a string for `sh -c`, escaping embedded quotes — entry
/// names can contain spaces and shell metacharacters.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Spawn `pass -c <entry>` (decrypt + copy + auto-clear). Detached; gpg
/// pinentry, if needed, is handled by the agent.
fn copy_password(entry: &str) {
    if let Err(err) = Command::new("pass").arg("-c").arg(entry).spawn() {
        tracing::warn!(?err, entry, "pass: copy spawn failed");
    }
}

/// Type the password into the focused surface via wtype, never touching
/// the clipboard. The short sleep lets the launcher close and keyboard
/// focus return to the target window first.
fn type_password(entry: &str) {
    let cmd = format!(
        "sleep 0.25; pass show {} 2>/dev/null | head -n1 | tr -d '\\n' | wtype -",
        shell_quote(entry)
    );
    if let Err(err) = Command::new("sh").arg("-c").arg(cmd).spawn() {
        tracing::warn!(?err, entry, "pass: type spawn failed");
    }
}

const ICON: &str = "dialog-password-symbolic";

impl Provider for PassProvider {
    fn name(&self) -> &str {
        "Pass"
    }

    fn category(&self) -> &str {
        "Connect"
    }

    fn handles_search(&self) -> bool {
        // Password entry names must never surface in the regular,
        // unprefixed search — only via the explicit `pass ` invocation.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "pass" || q.starts_with("pass ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "pass:palette".into(),
            name: "pass".into(),
            description: "Copy (Enter) or type (Ctrl+Enter) a password from your store".into(),
            icon: ICON.into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Pass".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "pass" || q.starts_with("pass ")) {
            return Vec::new();
        }
        let filter = q.trim_start_matches("pass").trim().to_ascii_lowercase();
        let tokens: Vec<&str> = filter.split_whitespace().collect();

        let entries = self.entries.borrow();
        if entries.is_empty() {
            return vec![LauncherItem {
                id: "pass:none".into(),
                name: "No password entries found".into(),
                description: format!("Looked in {}", self.store_dir.borrow().display()),
                icon: "dialog-warning-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Pass".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        entries
            .iter()
            .filter(|e| {
                // AND-token match: every space-separated token must appear
                // (the noctalia "spaces as wildcards" behaviour).
                let lower = e.to_ascii_lowercase();
                tokens.iter().all(|t| lower.contains(t))
            })
            .take(MAX_RESULTS)
            .enumerate()
            .map(|(idx, e)| {
                let entry = e.clone();
                let entry_for_toast = e.clone();
                LauncherItem {
                    id: format!("pass:entry:{e}"),
                    name: e.clone(),
                    description: "Enter: copy · Ctrl+Enter: type".into(),
                    icon: ICON.into(),
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "Pass".into(),
                    usage_key: Some(format!("pass:{e}")),
                    on_activate: Rc::new(move || {
                        copy_password(&entry);
                        toast("Pass", format!("Copied {entry_for_toast} (clears in 45s)"));
                    }),
                }
            })
            .collect()
    }

    /// Ctrl+Enter on an entry → type it via wtype (no clipboard).
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let entry = item.id.strip_prefix("pass:entry:")?.to_string();
        if entry.is_empty() {
            return None;
        }
        let entry_for_toast = entry.clone();
        Some(Rc::new(move || {
            type_password(&entry);
            toast("Pass", format!("Typing {entry_for_toast}"));
        }))
    }

    fn on_opened(&mut self) {
        // Re-resolve the store path (a Settings change applies here) and
        // pick up new/removed entries. Cheap.
        if let Some(resolver) = &self.resolver {
            *self.store_dir.borrow_mut() = resolver();
        }
        self.refresh();
    }

    /// Connect tab — list every entry without the `pass ` prefix;
    /// `filter` narrows by AND-token substring.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("pass")
        } else {
            self.search(&format!("pass {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dcli-pass-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("email.gpg"), b"x").unwrap();
        std::fs::write(dir.join("web/github.gpg"), b"x").unwrap();
        std::fs::write(dir.join("web/gitlab.gpg"), b"x").unwrap();
        std::fs::write(dir.join(".git/HEAD"), b"x").unwrap();
        std::fs::write(dir.join(".gpg-id"), b"KEY").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        dir
    }

    #[test]
    fn scans_entries_relative_without_gpg_skipping_dotdirs() {
        let dir = temp_store();
        let p = PassProvider::with_dir(dir.clone());
        let entries = p.entries.borrow().clone();
        assert!(entries.contains(&"email".to_string()));
        assert!(entries.contains(&"web/github".to_string()));
        assert!(entries.contains(&"web/gitlab".to_string()));
        // .git/ contents and non-.gpg files are excluded.
        assert!(!entries.iter().any(|e| e.contains("HEAD")));
        assert!(!entries.iter().any(|e| e.contains("notes")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn and_token_filter_treats_spaces_as_wildcards() {
        let dir = temp_store();
        let p = PassProvider::with_dir(dir.clone());
        let names: Vec<String> = p.search("pass web git").iter().map(|i| i.name.clone()).collect();
        // "web" + "git" both present in web/github and web/gitlab.
        assert!(names.contains(&"web/github".to_string()));
        assert!(names.contains(&"web/gitlab".to_string()));
        assert!(!names.contains(&"email".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handles_command_only_for_pass_prefix() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent"));
        assert!(p.handles_command("pass"));
        assert!(p.handles_command("pass github"));
        assert!(!p.handles_command("passwd"));
        assert!(!p.handles_command(":pass"));
    }

    #[test]
    fn missing_store_yields_warning_row() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent/store"));
        assert!(p.search("pass").iter().any(|i| i.name == "No password entries found"));
    }

    #[test]
    fn alt_action_only_for_real_entries() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent"));
        let mut palette = p.commands();
        assert!(p.alt_action(&palette.remove(0)).is_none());
    }
}
