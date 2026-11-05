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
use std::time::{Duration, SystemTime};

/// One saved copy of `config.conf`'s content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    /// The filename stem, e.g. `"20260901T142233.123Z"` (UTC, millisecond
    /// resolution), unique per save: a save that would reuse a taken name
    /// or sort before the newest one gets a `-N` suffix instead
    /// (`"20260901T142233.123Z-1"`). Older files use the
    /// second form `"20260901T142233Z"` and still list in order.
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
    chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string()
}

/// Save `content` as a new generation under `dir`, unless it's
/// byte-identical to the most recently saved one. Prunes down to `keep`
/// afterward. Best-effort — I/O failures are logged (`tracing::warn!`)
/// and swallowed (`None`) rather than returned as an error, since this
/// runs on the boot/reload-critical path and a full disk must never
/// block config from applying.
pub fn save_to(dir: &Path, content: &str, keep: usize) -> Option<Generation> {
    save_inner(dir, content, keep, None)
}

/// Like [`save_to`], but when the newest generation is younger than
/// `window` it is *replaced* rather than kept alongside the new one. Used
/// for config auto-reload, where every editor save is a reload — without
/// coalescing an afternoon of tweaking would push every earlier
/// known-good config out of the `keep` window. The generation that
/// predates the editing burst is never touched.
pub fn save_coalescing_to(
    dir: &Path,
    content: &str,
    keep: usize,
    window: Duration,
) -> Option<Generation> {
    save_inner(dir, content, keep, Some(window))
}

fn save_inner(
    dir: &Path,
    content: &str,
    keep: usize,
    coalesce: Option<Duration>,
) -> Option<Generation> {
    let newest = list_in(dir).ok().and_then(|g| g.into_iter().next());
    if let Some(newest) = &newest
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
    let id = unique_generation_id(
        dir,
        generation_id_now(),
        newest.as_ref().map(|g| g.id.as_str()),
    );
    let path = dir.join(format!("{id}.conf"));
    // Atomic: this file is boot recovery's last-known-good source, so a
    // crash mid-write must never leave a truncated generation behind.
    if let Err(e) = margo_fs::atomic_write::atomic_write(&path, content.as_bytes()) {
        tracing::warn!(
            "config generations: could not save generation to {}: {e}",
            path.display()
        );
        return None;
    }
    if let (Some(window), Some(newest)) = (coalesce, &newest)
        && newest.id != id
        && newest.timestamp.elapsed().is_ok_and(|age| age < window)
        && let Err(e) = std::fs::remove_file(&newest.path)
    {
        tracing::warn!(
            "config generations: could not coalesce {}: {e}",
            newest.path.display()
        );
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

/// [`save_coalescing_to`] against [`generations_dir`].
pub fn save_coalescing(content: &str, keep: usize, window: Duration) -> Option<Generation> {
    save_coalescing_to(&generations_dir(), content, keep, window)
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
    // Newest first, by the id's time then its collision suffix.
    gens.sort_by_key(|g| std::cmp::Reverse(id_sort_key(&g.id)));
    Ok(gens)
}

/// Split an id into its time part and its collision suffix
/// (`"20260901T142233.123Z-2"` → `("20260901T142233.123Z", 2)`); an id
/// without a numeric `-N` tail has suffix 0.
fn split_suffix(id: &str) -> (&str, u64) {
    if let Some((base, n)) = id.rsplit_once('-')
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
        && let Ok(n) = n.parse()
    {
        return (base, n);
    }
    (id, 0)
}

/// Save-order key of a generation id. Accepts the current millisecond
/// form (`20260901T142233.123Z`), the older second form
/// (`20260901T142233Z`, read as `.000`) and a `-N` collision suffix,
/// compared as a number so `-10` sorts after `-2`.
fn id_sort_key(id: &str) -> (String, u64) {
    let (base, n) = split_suffix(id);
    let stamp = base.strip_suffix('Z').unwrap_or(base);
    let (secs, millis) = stamp.split_once('.').unwrap_or((stamp, "000"));
    (format!("{secs}.{millis:0<3}"), n)
}

/// A fresh, unused id under `dir` that sorts after `newest`: the current
/// UTC time in milliseconds, with a `-N` suffix when that name is taken
/// or when the clock reads earlier than the newest saved id (same
/// millisecond, or the clock stepped back).
fn unique_generation_id(dir: &Path, now_id: String, newest: Option<&str>) -> String {
    let (base, mut n) = match newest {
        Some(newest) if id_sort_key(&now_id) <= id_sort_key(newest) => {
            let (base, n) = split_suffix(newest);
            (base.to_string(), n + 1)
        }
        _ => (now_id, 0),
    };
    loop {
        let id = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        if !dir.join(format!("{id}.conf")).exists() {
            return id;
        }
        n += 1;
    }
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
        let second = save_to(dir.path(), "borderpx = 9\n", 20);
        assert!(second.is_some());
        assert_eq!(list_in(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn prune_keeps_only_the_newest_n() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            save_to(dir.path(), &format!("borderpx = {i}\n"), 3);
        }
        let gens = list_in(dir.path()).unwrap();
        assert_eq!(gens.len(), 3, "prune must keep exactly `keep` generations");
        // Newest first: the last-saved content survives.
        assert_eq!(
            std::fs::read_to_string(&gens[0].path).unwrap(),
            "borderpx = 4\n"
        );
    }

    /// Two different saves inside one second (a `mctl config set` script,
    /// Settings + an editor) must both survive: the second used to take
    /// the same second-resolution id and overwrite the first on disk.
    #[test]
    fn two_different_saves_in_the_same_second_keep_both() {
        let dir = tempfile::tempdir().unwrap();
        let a = save_to(dir.path(), "borderpx = 1\n", 20).expect("first save");
        let b = save_to(dir.path(), "borderpx = 2\n", 20).expect("second save");
        assert_ne!(a.id, b.id);
        let gens = list_in(dir.path()).unwrap();
        assert_eq!(gens.len(), 2, "{gens:?}");
        assert_eq!(gens[0].id, b.id, "the second save is the newest");
        assert_eq!(read_to(&gens[0]), "borderpx = 2\n");
        assert_eq!(read_to(&gens[1]), "borderpx = 1\n");
    }

    /// Many saves faster than the id's resolution still get unique ids
    /// that list in save order.
    #[test]
    fn a_burst_of_saves_lists_in_save_order() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..12 {
            save_to(dir.path(), &format!("borderpx = {i}\n"), 50).expect("save");
        }
        let contents: Vec<String> = list_in(dir.path()).unwrap().iter().map(read_to).collect();
        let want: Vec<String> = (0..12).rev().map(|i| format!("borderpx = {i}\n")).collect();
        assert_eq!(contents, want);
    }

    /// Old second-resolution names, millisecond names and collision
    /// suffixes order by time, then by suffix number (not as text:
    /// `-10` is newer than `-2`).
    #[test]
    fn ids_order_by_time_then_suffix() {
        let mut ids = vec![
            "20260901T142233Z",
            "20260901T142233.500Z-10",
            "20260901T142233.500Z-2",
            "20260901T142233.500Z",
            "20260901T142232.999Z",
            "20260901T142234Z",
        ];
        ids.sort_by_key(|id| std::cmp::Reverse(id_sort_key(id)));
        assert_eq!(
            ids,
            vec![
                "20260901T142234Z",
                "20260901T142233.500Z-10",
                "20260901T142233.500Z-2",
                "20260901T142233.500Z",
                "20260901T142233Z",
                "20260901T142232.999Z",
            ]
        );
    }

    fn read_to(g: &Generation) -> String {
        std::fs::read_to_string(&g.path).unwrap()
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

    #[test]
    fn coalescing_replaces_a_recent_newest_but_keeps_older_history() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("20200101T000000Z.conf");
        std::fs::write(&old, "borderpx = 1\n").unwrap();
        let recent = dir.path().join("20200101T000001Z.conf");
        std::fs::write(&recent, "borderpx = 2\n").unwrap();
        // `recent` has a fresh mtime, so it is inside the window.
        let saved = save_coalescing_to(dir.path(), "borderpx = 3\n", 20, Duration::from_secs(600))
            .expect("changed content must save");
        let ids: Vec<String> = list_in(dir.path())
            .unwrap()
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, vec![saved.id, "20200101T000000Z".to_string()]);
    }

    #[test]
    fn coalescing_keeps_a_newest_older_than_the_window() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("20200101T000000Z.conf"), "borderpx = 1\n").unwrap();
        save_coalescing_to(dir.path(), "borderpx = 2\n", 20, Duration::ZERO).unwrap();
        assert_eq!(list_in(dir.path()).unwrap().len(), 2);
    }
}
