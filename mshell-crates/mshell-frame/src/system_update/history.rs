//! Update history + per-package rollback.
//!
//! History comes from `/var/log/pacman.log` when the user can read it
//! (grouped by pacman transaction). Some setups ship it `root:root
//! 0640`; then the local package DB (`/var/lib/pacman/local/*/desc`,
//! world-readable) is used instead: packages grouped by install time,
//! with the previous version taken from the pacman cache.
//!
//! Rollback reinstalls the old file straight from the cache with
//! `pkexec pacman -U` (run through the background runner, so it gets
//! the same log/progress/notification). `--nodeps` is never passed: a
//! downgrade that breaks another package's versioned dependency makes
//! pacman refuse the transaction before anything changes.

use super::size::PKG_CACHE;
use std::path::Path;

const PACMAN_LOG: &str = "/var/log/pacman.log";
const LOCAL_DB: &str = "/var/lib/pacman/local";
/// How many transactions the panel lists.
pub(crate) const MAX_RUNS: usize = 15;
/// Two installs this close together belong to the same run (DB mode).
const GROUP_GAP_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Upgraded,
    Downgraded,
    Installed,
    Removed,
    Reinstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) name: String,
    pub(crate) action: Action,
    /// Version before the transaction (`None` for a fresh install).
    pub(crate) old: Option<String>,
    /// Version after (`None` for a removal).
    pub(crate) new: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Transaction {
    /// Unix seconds.
    pub(crate) time: i64,
    pub(crate) changes: Vec<Change>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistorySource {
    PacmanLog,
    LocalDb,
}

/// Parse one `[ALPM] <action> name (versions)` line.
fn parse_alpm(msg: &str) -> Option<Change> {
    let (verb, rest) = msg.split_once(' ')?;
    let action = match verb {
        "upgraded" => Action::Upgraded,
        "downgraded" => Action::Downgraded,
        "installed" => Action::Installed,
        "removed" => Action::Removed,
        "reinstalled" => Action::Reinstalled,
        _ => return None,
    };
    let (name, vers) = rest.split_once(" (")?;
    let vers = vers.strip_suffix(')')?;
    let (old, new) = match vers.split_once(" -> ") {
        Some((o, n)) => (Some(o.to_string()), Some(n.to_string())),
        None => match action {
            Action::Removed => (Some(vers.to_string()), None),
            _ => (None, Some(vers.to_string())),
        },
    };
    Some(Change {
        name: name.to_string(),
        action,
        old,
        new,
    })
}

/// Transactions from pacman.log, oldest first. Changes outside a
/// `transaction started` / `completed` frame (very old logs) form
/// their own transaction per timestamp.
pub(crate) fn parse_pacman_log(log: &str) -> Vec<Transaction> {
    let mut out: Vec<Transaction> = Vec::new();
    let mut open: Option<Transaction> = None;
    for line in log.lines() {
        let Some(time) = super::news::log_line_time(line) else {
            continue;
        };
        let Some((_, msg)) = line.split_once("] [ALPM] ") else {
            continue;
        };
        let msg = msg.trim_end();
        if msg == "transaction started" {
            if let Some(t) = open.take().filter(|t| !t.changes.is_empty()) {
                out.push(t);
            }
            open = Some(Transaction {
                time,
                changes: Vec::new(),
            });
        } else if msg == "transaction completed" || msg == "transaction interrupted" {
            if let Some(t) = open.take().filter(|t| !t.changes.is_empty()) {
                out.push(t);
            }
        } else if let Some(change) = parse_alpm(msg) {
            match open.as_mut() {
                Some(t) => t.changes.push(change),
                None => match out.last_mut() {
                    Some(t) if t.time == time => t.changes.push(change),
                    _ => out.push(Transaction {
                        time,
                        changes: vec![change],
                    }),
                },
            }
        }
    }
    if let Some(t) = open.filter(|t| !t.changes.is_empty()) {
        out.push(t);
    }
    out
}

/// `(name, version, install date)` from a local DB `desc` file.
pub(crate) fn parse_desc(desc: &str) -> Option<(String, String, i64)> {
    let mut name = None;
    let mut version = None;
    let mut date = None;
    let mut lines = desc.lines();
    while let Some(l) = lines.next() {
        match l {
            "%NAME%" => name = lines.next().map(str::to_string),
            "%VERSION%" => version = lines.next().map(str::to_string),
            "%INSTALLDATE%" => date = lines.next().and_then(|d| d.trim().parse().ok()),
            _ => {}
        }
    }
    Some((name?, version?, date?))
}

/// The version encoded in a cache file name for `name`
/// (`name-ver-rel-arch.pkg.tar.zst` → `ver-rel`), or `None` when the
/// file belongs to another package / is a signature.
pub(crate) fn cache_file_version(name: &str, file: &str) -> Option<String> {
    let rest = file.strip_prefix(name)?.strip_prefix('-')?;
    let (stem, _) = rest.split_once(".pkg.tar")?;
    if file.ends_with(".sig") {
        return None;
    }
    // stem = ver-rel-arch: exactly two dashes.
    let mut parts = stem.rsplitn(3, '-');
    let _arch = parts.next()?;
    let rel = parts.next()?;
    let ver = parts.next()?;
    if ver.contains('-') || ver.is_empty() {
        return None;
    }
    Some(format!("{ver}-{rel}"))
}

/// The cache file for `name` at exactly `version`.
pub(crate) fn find_cached<'a>(name: &str, version: &str, files: &'a [String]) -> Option<&'a str> {
    files
        .iter()
        .map(String::as_str)
        .find(|f| cache_file_version(name, f).as_deref() == Some(version))
}

/// DB-mode history: installed packages grouped into runs by install
/// time; the old version is the most recently downloaded cached file
/// of another version that predates the install. `files` is
/// `(file name, mtime)` of the cache.
pub(crate) fn history_from_db(
    pkgs: &[(String, String, i64)],
    files: &[(String, i64)],
) -> Vec<Transaction> {
    let mut sorted: Vec<&(String, String, i64)> = pkgs.iter().collect();
    sorted.sort_by_key(|p| p.2);
    let mut out: Vec<Transaction> = Vec::new();
    for (name, version, date) in sorted {
        let old = files
            .iter()
            .filter(|(f, mtime)| {
                *mtime <= *date && cache_file_version(name, f).is_some_and(|v| v != *version)
            })
            .max_by_key(|(_, mtime)| *mtime)
            .and_then(|(f, _)| cache_file_version(name, f));
        let change = Change {
            name: name.clone(),
            action: if old.is_some() {
                Action::Upgraded
            } else {
                Action::Installed
            },
            old,
            new: Some(version.clone()),
        };
        match out.last_mut() {
            Some(t) if date - t.time <= GROUP_GAP_SECS => {
                t.time = *date;
                t.changes.push(change);
            }
            _ => out.push(Transaction {
                time: *date,
                changes: vec![change],
            }),
        }
    }
    out
}

/// The `pacman -U` step that puts `change.old` back.
pub(crate) fn rollback_step(file: &str) -> Option<String> {
    let path = Path::new(PKG_CACHE).join(file);
    let p = path.to_str()?;
    // Cache names are `[A-Za-z0-9@._+:-]`; refuse anything else
    // rather than quote it into a root shell.
    if !file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@._+:-".contains(c))
    {
        return None;
    }
    Some(format!(
        "pkexec pacman -U --noconfirm --noprogressbar --color never {p}"
    ))
}

// ── IO ───────────────────────────────────────────────────────────

/// Cache listing: `(file name, mtime secs)`.
pub(crate) fn cache_listing() -> Vec<(String, i64)> {
    let Ok(rd) = std::fs::read_dir(PKG_CACHE) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let mtime = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some((name, mtime))
        })
        .collect()
}

/// Newest-first history (at most [`MAX_RUNS`]) and where it came from.
pub(crate) fn load_blocking() -> (HistorySource, Vec<Transaction>) {
    let (source, mut runs) = match std::fs::read_to_string(PACMAN_LOG) {
        Ok(log) => (HistorySource::PacmanLog, parse_pacman_log(&log)),
        Err(_) => {
            let pkgs: Vec<_> = std::fs::read_dir(LOCAL_DB)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter_map(|e| std::fs::read_to_string(e.path().join("desc")).ok())
                        .filter_map(|d| parse_desc(&d))
                        .collect()
                })
                .unwrap_or_default();
            (
                HistorySource::LocalDb,
                history_from_db(&pkgs, &cache_listing()),
            )
        }
    };
    runs.reverse();
    runs.truncate(MAX_RUNS);
    (source, runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    // pacman.log shape (pacman 7.x); this machine's log is root-only,
    // so the lines follow pacman's own format strings.
    const LOG: &str = "\
[2026-11-01T10:00:00+0300] [PACMAN] Running 'pacman -Syu'
[2026-11-01T10:00:01+0300] [PACMAN] starting full system upgrade
[2026-11-01T10:00:20+0300] [ALPM] transaction started
[2026-11-01T10:00:21+0300] [ALPM] upgraded electron43 (43.8.6-1 -> 43.8.7-1)
[2026-11-01T10:00:21+0300] [ALPM] upgraded firefox (159.0-1.1 -> 159.0.1-1.1)
[2026-11-01T10:00:22+0300] [ALPM] installed python-foo (1.2-1)
[2026-11-01T10:00:22+0300] [ALPM-SCRIPTLET] some hook output
[2026-11-01T10:00:23+0300] [ALPM] transaction completed
[2026-11-02T09:00:00+0300] [ALPM] transaction started
[2026-11-02T09:00:01+0300] [ALPM] removed python-foo (1.2-1)
[2026-11-02T09:00:02+0300] [ALPM] downgraded firefox (159.0.1-1.1 -> 159.0-1.1)
[2026-11-02T09:00:03+0300] [ALPM] transaction completed
[2026-11-03T09:00:00+0300] [ALPM] transaction started
[2026-11-03T09:00:03+0300] [ALPM] transaction completed
";

    #[test]
    fn groups_pacman_log_by_transaction() {
        let runs = parse_pacman_log(LOG);
        assert_eq!(runs.len(), 2, "the empty transaction is dropped");
        assert_eq!(runs[0].changes.len(), 3);
        assert_eq!(
            runs[0].changes[1],
            Change {
                name: "firefox".into(),
                action: Action::Upgraded,
                old: Some("159.0-1.1".into()),
                new: Some("159.0.1-1.1".into()),
            }
        );
        assert_eq!(runs[0].changes[2].action, Action::Installed);
        assert_eq!(runs[0].changes[2].old, None);
        assert_eq!(runs[1].changes[0].action, Action::Removed);
        assert_eq!(runs[1].changes[0].old.as_deref(), Some("1.2-1"));
        assert_eq!(runs[1].changes[0].new, None);
        assert_eq!(runs[1].changes[1].action, Action::Downgraded);
    }

    #[test]
    fn unframed_lines_group_by_timestamp() {
        let log = "[2019-01-01 12:00] [ALPM] upgraded a (1-1 -> 2-1)\n\
                   [2019-01-01 12:00] [ALPM] upgraded b (1-1 -> 2-1)\n\
                   [2019-01-02 12:00] [ALPM] upgraded c (1-1 -> 2-1)\n";
        let runs = parse_pacman_log(log);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].changes.len(), 2);
    }

    #[test]
    fn cache_file_names() {
        // Real names from /var/cache/pacman/pkg on this machine.
        assert_eq!(
            cache_file_version(
                "glibc",
                "glibc-2.44+r50+g1848099f063e-3-x86_64_v3.pkg.tar.zst"
            )
            .as_deref(),
            Some("2.44+r50+g1848099f063e-3")
        );
        assert_eq!(
            cache_file_version("mkinitcpio", "mkinitcpio-42.1-1-any.pkg.tar.zst").as_deref(),
            Some("42.1-1")
        );
        assert_eq!(
            cache_file_version("7zip", "7zip-26.03-1.1-x86_64_v3.pkg.tar.zst").as_deref(),
            Some("26.03-1.1")
        );
        assert_eq!(
            cache_file_version("7zip", "7zip-26.03-1.1-x86_64_v3.pkg.tar.zst.sig"),
            None
        );
        // A different package sharing the prefix.
        assert_eq!(
            cache_file_version("firefox", "firefox-i18n-de-159.0-1-any.pkg.tar.zst"),
            None
        );
        assert_eq!(
            cache_file_version("python", "python-foo-1.2-1-any.pkg.tar.zst"),
            None
        );
    }

    #[test]
    fn desc_parses() {
        let desc = "%NAME%\nmkinitcpio\n\n%VERSION%\n42.2-1\n\n%BASE%\nmkinitcpio\n\n\
                    %INSTALLDATE%\n1792000000\n\n%SIZE%\n1\n";
        assert_eq!(
            parse_desc(desc),
            Some(("mkinitcpio".into(), "42.2-1".into(), 1_792_000_000))
        );
        assert_eq!(parse_desc("%NAME%\nx\n"), None);
    }

    #[test]
    fn db_history_groups_and_finds_the_previous_cached_version() {
        let pkgs = vec![
            ("mkinitcpio".to_string(), "42.2-1".to_string(), 1000),
            ("glibc".to_string(), "2.44-3".to_string(), 1100),
            ("newpkg".to_string(), "1-1".to_string(), 5000),
        ];
        let files = vec![
            ("mkinitcpio-42-1-any.pkg.tar.zst".to_string(), 100),
            ("mkinitcpio-42.1-1-any.pkg.tar.zst".to_string(), 500),
            ("mkinitcpio-42.2-1-any.pkg.tar.zst".to_string(), 990),
            ("glibc-2.44-1-x86_64.pkg.tar.zst".to_string(), 50),
            // Downloaded after the install: not the previous version.
            ("glibc-2.45-1-x86_64.pkg.tar.zst".to_string(), 2000),
        ];
        let runs = history_from_db(&pkgs, &files);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].changes[0].old.as_deref(), Some("42.1-1"));
        assert_eq!(runs[0].changes[1].old.as_deref(), Some("2.44-1"));
        assert_eq!(runs[1].changes[0].action, Action::Installed);
    }

    #[test]
    fn rollback_command() {
        let files = vec!["mkinitcpio-42.1-1-any.pkg.tar.zst".to_string()];
        let f = find_cached("mkinitcpio", "42.1-1", &files).unwrap();
        assert_eq!(
            rollback_step(f).unwrap(),
            "pkexec pacman -U --noconfirm --noprogressbar --color never \
             /var/cache/pacman/pkg/mkinitcpio-42.1-1-any.pkg.tar.zst"
        );
        assert!(find_cached("mkinitcpio", "41-1", &files).is_none());
        assert!(rollback_step("x;reboot.pkg.tar.zst").is_none());
    }
}
