//! History of `config.conf`'s content: a timestamped copy is saved every
//! time a config successfully takes effect (compositor boot, or a
//! successful `mctl reload`), so `mctl config rollback` is a one-command
//! undo and a boot-time parse failure can fall back to the last
//! known-good file instead of `Config::default()`. Only the resolved
//! `config.conf` itself is snapshotted — never the `source`d fragments
//! (`conf.d/*`, `binds.d/*`), which are machine-written by other tools
//! with their own lifecycles. See
//! `docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One saved copy of `config.conf`'s content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    /// The filename stem, e.g. `"20260901T142233Z"` — sortable
    /// lexicographically in save order, unique per save (second
    /// resolution; two saves within the same second overwrite each
    /// other, which is an acceptable edge case for a manual/reload-rate
    /// trigger).
    pub id: String,
    /// System time when this generation was saved.
    pub timestamp: SystemTime,
    /// Path to the generation file on disk.
    pub path: PathBuf,
}

/// Compute the generations directory from optional XDG_STATE_HOME and HOME env values.
/// Pure function for testing without env mutation.
fn generations_dir_from(xdg_state_home: Option<&str>, home: Option<&str>) -> PathBuf {
    let base = xdg_state_home.map(PathBuf::from).unwrap_or_else(|| {
        let home_path = home.unwrap_or("/tmp");
        PathBuf::from(home_path).join(".local/state")
    });
    base.join("margo").join("config-generations")
}

/// `$XDG_STATE_HOME/margo/config-generations`, falling back to
/// `~/.local/state/margo/config-generations` — mirrors
/// `margo_logging::logs_dir`'s `margo/logs` sibling.
pub fn generations_dir() -> PathBuf {
    let xdg_state_home = std::env::var("XDG_STATE_HOME").ok();
    let home = std::env::var("HOME").ok();
    generations_dir_from(xdg_state_home.as_deref(), home.as_deref())
}

fn generation_id_now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// Save `content` as a new generation under `dir`, unless it's
/// byte-identical to the most recently saved one. Prunes down to `keep`
/// afterward. Best-effort — I/O failures are logged (`tracing::warn!`)
/// and swallowed (`None`) rather than returned as an error, since this
/// runs on the boot/reload-critical path and a full disk must never
/// block config from applying.
pub fn save_to(dir: &Path, content: &str, keep: usize) -> Option<Generation> {
    if let Ok(existing) = list_in(dir)
        && let Some(newest) = existing.first()
        && let Ok(prev) = std::fs::read_to_string(&newest.path)
        && prev == content
    {
        return None;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!(
            "config generations: could not create {}: {e}",
            dir.display()
        );
        return None;
    }
    let id = generation_id_now();
    let path = dir.join(format!("{id}.conf"));
    if let Err(e) = std::fs::write(&path, content) {
        tracing::warn!(
            "config generations: could not save generation to {}: {e}",
            path.display()
        );
        return None;
    }
    prune_to(dir, keep);
    Some(Generation {
        id,
        timestamp: SystemTime::now(),
        path,
    })
}

/// [`save_to`] against [`generations_dir`].
pub fn save(content: &str, keep: usize) -> Option<Generation> {
    save_to(&generations_dir(), content, keep)
}

/// List generations under `dir`, newest first.
pub fn list_in(dir: &Path) -> std::io::Result<Vec<Generation>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut gens: Vec<Generation> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("conf") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let timestamp = entry.metadata().ok()?.modified().ok()?;
            Some(Generation {
                id,
                timestamp,
                path,
            })
        })
        .collect();
    // The id format is sortable, so lexicographic order == save order.
    gens.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(gens)
}

/// [`list_in`] against [`generations_dir`].
pub fn list() -> std::io::Result<Vec<Generation>> {
    list_in(&generations_dir())
}

/// Delete the oldest generations under `dir` until at most `keep`
/// remain. Best-effort: a failed listing or delete is logged and
/// otherwise ignored — never blocks the save that just succeeded.
fn prune_to(dir: &Path, keep: usize) {
    let Ok(gens) = list_in(dir) else {
        tracing::warn!(
            "config generations: could not list {} to prune",
            dir.display()
        );
        return;
    };
    for r#gen in gens.into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_file(&r#gen.path) {
            tracing::warn!(
                "config generations: could not prune {}: {e}",
                r#gen.path.display()
            );
        }
    }
}

/// The most recent generation's id + content, if any exist and the
/// newest one is readable.
pub fn latest_in(dir: &Path) -> Option<(Generation, String)> {
    let gens = list_in(dir).ok()?;
    let newest = gens.into_iter().next()?;
    let content = std::fs::read_to_string(&newest.path).ok()?;
    Some((newest, content))
}

/// [`latest_in`] against [`generations_dir`].
pub fn latest() -> Option<(Generation, String)> {
    latest_in(&generations_dir())
}

/// Read one generation's content by id (as listed by [`list`]).
pub fn read(id: &str) -> std::io::Result<String> {
    std::fs::read_to_string(generations_dir().join(format!("{id}.conf")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_creates_a_generation_file() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save_to(dir.path(), "borderpx = 7\n", 20);
        let r#gen = saved.expect("first save must produce a generation");
        assert_eq!(
            std::fs::read_to_string(&r#gen.path).unwrap(),
            "borderpx = 7\n"
        );
        assert!(r#gen.path.starts_with(dir.path()));
    }

    #[test]
    fn identical_content_save_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_to(dir.path(), "borderpx = 7\n", 20);
        assert!(first.is_some());
        let second = save_to(dir.path(), "borderpx = 7\n", 20);
        assert!(
            second.is_none(),
            "byte-identical content must not create a new generation"
        );
        assert_eq!(list_in(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn changed_content_creates_a_new_generation() {
        let dir = tempfile::tempdir().unwrap();
        save_to(dir.path(), "borderpx = 7\n", 20);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // cross a whole second
        let second = save_to(dir.path(), "borderpx = 9\n", 20);
        assert!(second.is_some());
        assert_eq!(list_in(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn prune_keeps_only_the_newest_n() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            save_to(dir.path(), &format!("borderpx = {i}\n"), 3);
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let gens = list_in(dir.path()).unwrap();
        assert_eq!(gens.len(), 3, "prune must keep exactly `keep` generations");
        // Newest first: the last-saved content survives.
        assert_eq!(
            std::fs::read_to_string(&gens[0].path).unwrap(),
            "borderpx = 4\n"
        );
    }

    #[test]
    fn list_in_on_missing_dir_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-yet");
        assert_eq!(list_in(&missing).unwrap(), Vec::new());
    }

    #[test]
    fn latest_in_returns_newest_content() {
        let dir = tempfile::tempdir().unwrap();
        save_to(dir.path(), "borderpx = 1\n", 20);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        save_to(dir.path(), "borderpx = 2\n", 20);
        let (r#gen, content) = latest_in(dir.path()).expect("a generation exists");
        assert_eq!(content, "borderpx = 2\n");
        assert_eq!(
            r#gen.path.file_name().unwrap().to_str().unwrap(),
            format!("{}.conf", r#gen.id)
        );
    }

    #[test]
    fn latest_in_on_empty_dir_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(latest_in(dir.path()).is_none());
    }

    #[test]
    fn generations_dir_honours_xdg_state_home() {
        // Test via pure helper function, no env mutation.
        assert_eq!(
            generations_dir_from(Some("/tmp/margo-generations-test-xdg"), None),
            PathBuf::from("/tmp/margo-generations-test-xdg/margo/config-generations")
        );
        assert_eq!(
            generations_dir_from(None, Some("/home/testuser")),
            PathBuf::from("/home/testuser/.local/state/margo/config-generations")
        );
        assert_eq!(
            generations_dir_from(None, None),
            PathBuf::from("/tmp/.local/state/margo/config-generations")
        );
    }
}
