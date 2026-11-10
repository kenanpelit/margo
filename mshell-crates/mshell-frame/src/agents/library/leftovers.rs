//! Leftovers of deleted sessions in an account's config dir:
//! `session-env/<id>` and `file-history/<id>` folders whose id has no
//! transcript in any project folder, and project folders holding no
//! session (nothing, or only an empty `memory/`). Only entries untouched
//! for a day count (a session may be starting). Listing is read-only;
//! removal re-checks every entry first.

use super::guard::{account_dir, plain_name, remove_entry};
use super::meta::is_uuid;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Entries younger than this are never leftovers.
pub(crate) const MIN_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// Entries walked to size one leftover.
const SIZE_WALK: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LeftoverKind {
    SessionEnv,
    FileHistory,
    Project,
}

impl LeftoverKind {
    pub(crate) fn dir(self) -> &'static str {
        match self {
            LeftoverKind::SessionEnv => "session-env",
            LeftoverKind::FileHistory => "file-history",
            LeftoverKind::Project => "projects",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Leftover {
    pub(crate) kind: LeftoverKind,
    pub(crate) config_dir: PathBuf,
    /// Its name in [`LeftoverKind::dir`].
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) mtime_ms: i64,
}

fn mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// Bytes of the files under `p`, not following symlinks (bounded walk).
fn size_of(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    let mut seen = 0;
    while let Some(d) = stack.pop() {
        let Ok(m) = std::fs::symlink_metadata(&d) else {
            continue;
        };
        if m.is_file() {
            total += m.len();
        }
        seen += 1;
        if seen > SIZE_WALK {
            break;
        }
        if m.is_dir()
            && let Ok(rd) = std::fs::read_dir(&d)
        {
            stack.extend(rd.flatten().map(|e| e.path()));
        }
    }
    total
}

/// A project folder that holds no session: empty, or only an empty
/// real `memory/` folder.
fn project_is_empty(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in rd.flatten() {
        if e.file_name() != "memory" {
            return false;
        }
        let ok = std::fs::symlink_metadata(e.path())
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            && std::fs::read_dir(e.path()).is_ok_and(|mut r| r.next().is_none());
        if !ok {
            return false;
        }
    }
    true
}

/// Whether the real folder `projects/<name>` is the target of a symlink
/// alias in `projects/` (removing it would leave the alias dangling).
fn aliased(projects: &Path, name: &str) -> bool {
    let target = projects.join(name);
    std::fs::read_dir(projects).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.file_type().is_ok_and(|t| t.is_symlink())
                && std::fs::canonicalize(e.path()).is_ok_and(|r| r == target)
        })
    })
}

/// Whether `name` in `kind`'s folder is a leftover now. `keep`: ids
/// that still have a transcript (in any account) or run.
fn eligible(
    base: &Path,
    kind: LeftoverKind,
    name: &str,
    keep: &HashSet<String>,
    now_ms: i64,
) -> Option<Leftover> {
    if !plain_name(name) {
        return None;
    }
    let path = base.join(name);
    let m = std::fs::symlink_metadata(&path).ok()?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return None;
    }
    let mtime = mtime_ms(&m);
    if now_ms - mtime < MIN_AGE_MS {
        return None;
    }
    let fits = match kind {
        LeftoverKind::SessionEnv | LeftoverKind::FileHistory => {
            is_uuid(name) && !keep.contains(name)
        }
        LeftoverKind::Project => project_is_empty(&path) && !aliased(base, name),
    };
    fits.then(|| Leftover {
        kind,
        config_dir: PathBuf::new(),
        name: name.to_string(),
        bytes: size_of(&path),
        path,
        mtime_ms: mtime,
    })
}

/// Every leftover of the account at `config_dir`, oldest first.
pub(crate) fn find(config_dir: &Path, keep: &HashSet<String>, now_ms: i64) -> Vec<Leftover> {
    let mut out = Vec::new();
    for kind in [
        LeftoverKind::SessionEnv,
        LeftoverKind::FileHistory,
        LeftoverKind::Project,
    ] {
        let Ok(base) = account_dir(config_dir, kind.dir()) else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        for n in names {
            if let Some(mut l) = eligible(&base, kind, &n, keep, now_ms) {
                l.config_dir = config_dir.to_path_buf();
                out.push(l);
            }
        }
    }
    out.sort_by_key(|l| l.mtime_ms);
    out
}

/// Remove `items` (as [`find`] listed them), each only if it is still a
/// leftover now. Returns how many were removed.
pub(crate) fn remove(
    items: &[Leftover],
    keep: &HashSet<String>,
    now_ms: i64,
) -> Result<usize, String> {
    let mut removed = 0;
    let mut failed = Vec::new();
    for l in items {
        let Ok(base) = account_dir(&l.config_dir, l.kind.dir()) else {
            continue;
        };
        let Some(now) = eligible(&base, l.kind, &l.name, keep, now_ms) else {
            continue;
        };
        let res = match l.kind {
            LeftoverKind::Project => {
                let mem = now.path.join("memory");
                if std::fs::symlink_metadata(&mem).is_ok() {
                    let _ = std::fs::remove_dir(&mem);
                }
                std::fs::remove_dir(&now.path).map(|()| true)
            }
            _ => remove_entry(&now.path),
        };
        match res {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => failed.push(format!("{}: {e}", l.name)),
        }
    }
    if failed.is_empty() {
        Ok(removed)
    } else {
        Err(format!("Removed {removed}; failed: {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEAD: &str = "0097bc96-d15a-4b4f-81b2-31fc139fc111";
    const ALIVE: &str = "cae70d77-c84d-45b9-b9ef-3a39caf81fcb";
    const DAY: i64 = MIN_AGE_MS;

    fn now() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join(".claude");
        for p in [
            format!("session-env/{DEAD}"),
            format!("session-env/{ALIVE}"),
            "session-env/not-a-uuid".to_string(),
            format!("file-history/{DEAD}"),
            "projects/-empty".to_string(),
            "projects/-empty-memory/memory".to_string(),
            "projects/-full-memory/memory".to_string(),
            "projects/-live".to_string(),
        ] {
            std::fs::create_dir_all(cfg.join(p)).unwrap();
        }
        std::fs::write(cfg.join(format!("file-history/{DEAD}/v1")), "12345").unwrap();
        std::fs::write(cfg.join("projects/-full-memory/memory/note.md"), "x").unwrap();
        std::fs::write(cfg.join(format!("projects/-live/{ALIVE}.jsonl")), "{}").unwrap();
        (d, cfg)
    }

    #[test]
    fn lists_only_day_old_leftovers() {
        let (_d, cfg) = setup();
        let keep: HashSet<String> = [ALIVE.to_string()].into();
        // Everything was just created: nothing counts yet.
        assert!(find(&cfg, &keep, now()).is_empty());
        let later = now() + DAY + 60_000;
        let got: Vec<(LeftoverKind, String)> = find(&cfg, &keep, later)
            .into_iter()
            .map(|l| (l.kind, l.name))
            .collect();
        let mut got = got;
        got.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.dir().cmp(b.0.dir())));
        assert_eq!(
            got,
            [
                (LeftoverKind::Project, "-empty".to_string()),
                (LeftoverKind::Project, "-empty-memory".to_string()),
                (LeftoverKind::FileHistory, DEAD.to_string()),
                (LeftoverKind::SessionEnv, DEAD.to_string()),
            ]
        );
        let fh = find(&cfg, &keep, later)
            .into_iter()
            .find(|l| l.kind == LeftoverKind::FileHistory)
            .unwrap();
        assert!(fh.bytes >= 5);
    }

    #[test]
    fn removal_rechecks_each_entry() {
        let (d, cfg) = setup();
        let keep: HashSet<String> = [ALIVE.to_string()].into();
        let later = now() + DAY + 60_000;
        let items = find(&cfg, &keep, later);
        // Meanwhile a session with the dead id came back, and the empty
        // project got a transcript: both stay.
        std::fs::write(cfg.join(format!("projects/-empty/{DEAD}.jsonl")), "{}").unwrap();
        let keep2: HashSet<String> = [ALIVE.to_string(), DEAD.to_string()].into();
        assert_eq!(remove(&items, &keep2, later).unwrap(), 1);
        assert!(cfg.join(format!("session-env/{DEAD}")).exists());
        assert!(cfg.join("projects/-empty").exists());
        assert!(!cfg.join("projects/-empty-memory").exists());
        assert!(cfg.join("projects/-full-memory/memory/note.md").exists());
        // A forged entry outside the managed folders is refused.
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let forged = Leftover {
            kind: LeftoverKind::SessionEnv,
            config_dir: cfg.clone(),
            name: "../../outside".into(),
            path: outside.clone(),
            bytes: 0,
            mtime_ms: 0,
        };
        assert_eq!(remove(&[forged], &keep, later).unwrap(), 0);
        assert!(outside.exists());
    }

    #[test]
    fn symlinks_and_aliased_projects_are_never_leftovers() {
        let (d, cfg) = setup();
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(
            &outside,
            cfg.join(format!(
                "session-env/{}",
                "11111111-2222-3333-4444-555555555555"
            )),
        )
        .unwrap();
        std::os::unix::fs::symlink(cfg.join("projects/-empty"), cfg.join("projects/-alias"))
            .unwrap();
        let later = now() + DAY + 60_000;
        let names: Vec<String> = find(&cfg, &HashSet::new(), later)
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert!(!names.iter().any(|n| n.starts_with("11111111")));
        assert!(!names.contains(&"-alias".to_string()));
        assert!(
            !names.contains(&"-empty".to_string()),
            "an alias points at it"
        );
    }
}
