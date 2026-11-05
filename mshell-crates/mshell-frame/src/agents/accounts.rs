//! Claude Code accounts: every config dir (`CLAUDE_CONFIG_DIR`, default
//! `~/.claude`) that sessions run under. Found from the configured list
//! (`bars.widgets.agents.claude_config_dirs`), the `CLAUDE_CONFIG_DIR` of
//! every running `claude` process (its `/proc/<pid>/environ`, the same
//! file the jump-to-terminal code reads), and remembered once seen so an
//! account's recent sessions stay listed after its last session closes.
//! Each account has its own sessions, history and plan limits; only the
//! files the single-account code already read are read per account.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Account {
    pub(crate) dir: PathBuf,
    /// `default` for `~/.claude`, `work` for `~/.claude-work`.
    pub(crate) label: String,
    /// The account mshell's own environment points at; it comes first.
    pub(crate) primary: bool,
}

/// `~/.claude` → `default`, `~/.claude-work` / `~/.claude_work` →
/// `work`, `~/Claudes/client` → `client`.
pub(crate) fn account_label(dir: &Path, home: Option<&Path>) -> String {
    if home.is_some_and(|h| dir == h.join(".claude")) {
        return "default".into();
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let trimmed = name
        .strip_prefix(".claude")
        .map(|r| r.trim_start_matches(['-', '_', '.']))
        .unwrap_or(name.trim_start_matches('.'));
    if trimmed.is_empty() {
        name
    } else {
        trimmed.to_string()
    }
}

/// `~/x` → `$HOME/x`; anything not absolute after that is dropped.
pub(crate) fn expand(dir: &str, home: Option<&Path>) -> Option<PathBuf> {
    let dir = dir.trim();
    let p = match (dir.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if dir == "~" => home?.to_path_buf(),
        _ => PathBuf::from(dir),
    };
    p.is_absolute().then_some(p)
}

/// The accounts to show: `primary` first, then `configured` and `seen`
/// dirs, de-duplicated, keeping only dirs that exist (`exists`).
pub(crate) fn merge(
    primary: &Path,
    configured: &[PathBuf],
    seen: &[PathBuf],
    home: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Account> {
    let mut out: Vec<Account> = vec![Account {
        dir: primary.to_path_buf(),
        label: account_label(primary, home),
        primary: true,
    }];
    for d in configured.iter().chain(seen) {
        if out.iter().any(|a| a.dir == *d) || !exists(d) {
            continue;
        }
        out.push(Account {
            dir: d.clone(),
            label: account_label(d, home),
            primary: false,
        });
    }
    // Two dirs with the same name (`~/a/.claude`, `~/b/.claude`) get their
    // parent folded in so the labels stay apart.
    let labels: Vec<String> = out.iter().map(|a| a.label.clone()).collect();
    for a in out.iter_mut() {
        if labels.iter().filter(|l| **l == a.label).count() > 1
            && let Some(parent) = a.dir.parent().and_then(Path::file_name)
        {
            a.label = format!("{}/{}", parent.to_string_lossy(), a.label);
        }
    }
    out
}

/// Config dirs of the running `claude` processes (`CLAUDE_CONFIG_DIR`,
/// unset = `default`). Reads `/proc/<pid>/comm`, and the environ of the
/// processes named `claude` only.
fn running_config_dirs(default: &Path) -> Vec<PathBuf> {
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for e in procs.flatten() {
        let name = e.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let is_claude = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .is_ok_and(|c| c.trim_end() == "claude");
        if !is_claude {
            continue;
        }
        let env = std::fs::read(format!("/proc/{pid}/environ"))
            .map(|b| super::sessions::parse_environ(&b))
            .unwrap_or_default();
        let dir = env
            .get("CLAUDE_CONFIG_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| default.to_path_buf());
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// How often running processes are rescanned for new config dirs.
const SCAN_EVERY_MS: i64 = 30_000;

#[derive(Default)]
struct Known {
    seen: Vec<PathBuf>,
    last_scan_ms: i64,
    configured: Vec<String>,
}

static KNOWN: LazyLock<Mutex<Known>> = LazyLock::new(Mutex::default);

/// Set the configured extra dirs (from the GTK side, on config change).
pub(crate) fn set_configured(dirs: Vec<String>) {
    if let Ok(mut k) = KNOWN.lock() {
        k.configured = dirs;
    }
}

/// Every account, primary first. Blocking (reads `/proc`); call off the
/// GTK thread.
pub(crate) fn accounts(now_ms: i64) -> Vec<Account> {
    let primary = super::limits::claude_config_dir();
    let home = margo_fs::xdg::home();
    let Ok(mut k) = KNOWN.lock() else {
        return merge(&primary, &[], &[], home.as_deref(), |_| true);
    };
    if now_ms - k.last_scan_ms >= SCAN_EVERY_MS {
        k.last_scan_ms = now_ms;
        let default = home.clone().unwrap_or_default().join(".claude");
        for d in running_config_dirs(&default) {
            if !k.seen.contains(&d) {
                k.seen.push(d);
            }
        }
    }
    let configured: Vec<PathBuf> = k
        .configured
        .iter()
        .filter_map(|d| expand(d, home.as_deref()))
        .collect();
    merge(&primary, &configured, &k.seen, home.as_deref(), |d| {
        d.join("projects").is_dir() || d.join("sessions").is_dir()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_from_dir_names() {
        let home = Path::new("/home/u");
        let l = |d: &str| account_label(Path::new(d), Some(home));
        assert_eq!(l("/home/u/.claude"), "default");
        assert_eq!(l("/home/u/.claude-work"), "work");
        assert_eq!(l("/home/u/.claude_client"), "client");
        assert_eq!(l("/home/u/Claudes/personal"), "personal");
        assert_eq!(l("/srv/.claude"), ".claude");
    }

    #[test]
    fn expands_tilde_and_drops_relative_dirs() {
        let home = Some(Path::new("/home/u"));
        assert_eq!(
            expand(" ~/.claude-work ", home),
            Some(PathBuf::from("/home/u/.claude-work"))
        );
        assert_eq!(expand("/abs", home), Some(PathBuf::from("/abs")));
        assert_eq!(expand("relative", home), None);
        assert_eq!(expand("~/x", None), None);
    }

    #[test]
    fn primary_first_then_existing_unique_dirs() {
        let home = Path::new("/home/u");
        let primary = home.join(".claude");
        let work = home.join(".claude-work");
        let gone = home.join(".claude-gone");
        let v = merge(
            &primary,
            std::slice::from_ref(&work),
            &[primary.clone(), work.clone(), gone.clone()],
            Some(home),
            |d| d != gone,
        );
        let got: Vec<_> = v.iter().map(|a| (a.label.as_str(), a.primary)).collect();
        assert_eq!(got, [("default", true), ("work", false)]);
    }

    #[test]
    fn same_named_dirs_get_their_parent() {
        let v = merge(
            Path::new("/a/.claude"),
            &[PathBuf::from("/b/.claude")],
            &[],
            None,
            |_| true,
        );
        let labels: Vec<_> = v.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["a/.claude", "b/.claude"]);
    }
}
