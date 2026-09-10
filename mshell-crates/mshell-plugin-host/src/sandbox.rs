//! Path sandbox for the plugin `read-file` / `write-file` capabilities.
//!
//! THE security boundary of the WASM tier's filesystem surface: a guest may
//! only ever touch its own per-plugin data dir. [`resolve_scoped`] is the
//! lexical gate (no `..`, no absolute); [`resolve_inside_root`] adds the
//! symlink check so a link planted inside the data dir can't redirect a
//! read/write out of it.
//! Deliberately wasmtime-free and compiled unconditionally (the `wasm`
//! feature only gates the runtime), so the unit tests below run on every
//! `cargo test --workspace` — CI exercises the boundary even in builds that
//! never link wasmtime.

use std::path::{Component, Path, PathBuf};

/// Resolve `rel_path` against `root` and reject any traversal: rejects empty,
/// absolute, or `..`-bearing paths (every component must be a plain name —
/// `.` / `..` / prefixes / root markers all fail). The returned path is
/// lexically inside `root`.
///
/// This is only the *lexical* gate — [`resolve_inside_root`] adds the
/// symlink check that stops a link planted inside `root` (a hostile
/// plugin bundle could ship one; the write API itself can't create one)
/// from redirecting a read/write out of the sandbox.
#[cfg_attr(not(feature = "wasm"), allow(dead_code))]
pub(crate) fn resolve_scoped(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    if rel_path.is_empty() {
        return Err("path is empty".to_string());
    }
    let candidate = Path::new(rel_path);
    if candidate.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(format!("disallowed path component in `{rel_path}`")),
        }
    }
    Ok(root.join(candidate))
}

/// The real canonical location of `root`'s deepest ancestor of `resolved`
/// that exists on disk. Following symlinks. `None` when `root` itself
/// doesn't resolve (a fresh plugin whose data dir hasn't been created —
/// the caller's own read / `create_dir_all` handles that; there's no
/// link to chase yet).
fn existing_canonical_anchor(root: &Path, resolved: &Path) -> Option<PathBuf> {
    let mut anchor: &Path = resolved;
    loop {
        if let Ok(c) = anchor.canonicalize() {
            return Some(c);
        }
        if anchor == root {
            return None;
        }
        anchor = anchor.parent()?;
    }
}

/// [`resolve_scoped`] plus the symlink check: the resolved path's
/// existing prefix, canonicalised, must still sit inside the canonical
/// `root`. Guards against a symlink component pointing out of the
/// sandbox — an interior symlink that stays inside `root` is fine.
#[cfg_attr(not(feature = "wasm"), allow(dead_code))]
pub(crate) fn resolve_inside_root(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let resolved = resolve_scoped(root, rel_path)?;
    if let Ok(canon_root) = root.canonicalize()
        && let Some(anchor) = existing_canonical_anchor(root, &resolved)
        && !anchor.starts_with(&canon_root)
    {
        return Err(format!(
            "`{rel_path}` escapes the plugin data dir through a symlink"
        ));
    }
    Ok(resolved)
}

/// Scoped read: resolve under `root` (traversal + symlink checked), then
/// read the file.
#[cfg_attr(not(feature = "wasm"), allow(dead_code))]
pub(crate) fn read_scoped(root: &Path, rel_path: &str) -> Result<Vec<u8>, String> {
    let path = resolve_inside_root(root, rel_path)?;
    std::fs::read(&path).map_err(|e| e.to_string())
}

/// Scoped write: resolve under `root`, create parent dirs, then write
/// atomically-ish (tmp + rename) so a crash mid-write doesn't corrupt the
/// existing file.
#[cfg_attr(not(feature = "wasm"), allow(dead_code))]
pub(crate) fn write_scoped(root: &Path, rel_path: &str, bytes: &[u8]) -> Result<(), String> {
    // Materialise `root` first (a fixed, host-controlled path — every
    // segment is a plain name) so the symlink check below has a real
    // directory to canonicalise against.
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;

    let path = resolve_inside_root(root, rel_path)?;
    let canon_root = root.canonicalize().map_err(|e| e.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Re-check after creating: the parent chain is now real, so its
        // canonical form is authoritative — a pre-existing symlink dir
        // in the chain shows up here even if it didn't resolve before.
        if !parent
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&canon_root)
        {
            return Err(format!(
                "`{rel_path}` escapes the plugin data dir through a symlink"
            ));
        }
    }
    let tmp = path.with_extension("mplugin-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new("/data/plugins/weather").to_path_buf()
    }

    // ── resolve_scoped: the traversal matrix ───────────────────────────

    #[test]
    fn plain_and_nested_relative_paths_resolve_inside_root() {
        let r = root();
        assert_eq!(
            resolve_scoped(&r, "notes.txt").unwrap(),
            r.join("notes.txt")
        );
        assert_eq!(
            resolve_scoped(&r, "cache/v1/state.json").unwrap(),
            r.join("cache/v1/state.json")
        );
    }

    #[test]
    fn empty_path_is_rejected() {
        assert!(resolve_scoped(&root(), "").is_err());
    }

    #[test]
    fn absolute_paths_are_rejected() {
        for p in ["/etc/passwd", "/", "/data/plugins/weather/own-file"] {
            assert!(resolve_scoped(&root(), p).is_err(), "{p} must be rejected");
        }
    }

    #[test]
    fn parent_traversal_is_rejected_wherever_it_appears() {
        for p in [
            "..",
            "../sibling-plugin/secrets",
            "a/../../escape",
            "a/b/../../../etc/passwd",
            "../../../../home/user/.ssh/id_ed25519",
        ] {
            assert!(resolve_scoped(&root(), p).is_err(), "{p} must be rejected");
        }
    }

    #[test]
    fn leading_curdir_is_rejected_interior_one_normalises_harmlessly() {
        // A leading `./` survives `Path::components()` as a CurDir component
        // → rejected (plain names only). An *interior* `/./` is normalised
        // away by std before we ever see it, so `a/./b` lands on `a/b` —
        // still inside the root, no escape. Lock both behaviours.
        assert!(resolve_scoped(&root(), "./notes.txt").is_err());
        assert_eq!(
            resolve_scoped(&root(), "a/./b").unwrap(),
            root().join("a/b")
        );
    }

    #[test]
    fn dotdot_as_a_filename_fragment_is_allowed() {
        // `..foo` / `foo..` are ordinary names, not traversal — only a real
        // `..` component escapes. Guard against over-blocking.
        let r = root();
        assert!(resolve_scoped(&r, "..hidden").is_ok());
        assert!(resolve_scoped(&r, "archive..old/data").is_ok());
    }

    #[test]
    fn percent_encoding_is_not_decoded() {
        // `%2e%2e%2f` must stay a literal filename — if some future change
        // url-decodes guest paths, this turns into traversal and this test
        // is the tripwire.
        let r = root();
        assert_eq!(
            resolve_scoped(&r, "%2e%2e%2fescape").unwrap(),
            r.join("%2e%2e%2fescape")
        );
    }

    // ── read/write through the boundary ────────────────────────────────

    /// Unique per-test scratch dir under the system tmp; removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("mplugin-sandbox-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn write_then_read_roundtrips_and_creates_parents() {
        let s = Scratch::new("roundtrip");
        write_scoped(&s.0, "nested/deep/state.json", b"{\"v\":1}").unwrap();
        assert_eq!(
            read_scoped(&s.0, "nested/deep/state.json").unwrap(),
            b"{\"v\":1}"
        );
        // Overwrite goes through the same tmp+rename path.
        write_scoped(&s.0, "nested/deep/state.json", b"{\"v\":2}").unwrap();
        assert_eq!(
            read_scoped(&s.0, "nested/deep/state.json").unwrap(),
            b"{\"v\":2}"
        );
        // The tmp sidecar must not linger after a successful write.
        assert!(!s.0.join("nested/deep/state.mplugin-tmp").exists());
    }

    #[test]
    fn write_outside_the_root_is_impossible_and_touches_nothing() {
        let s = Scratch::new("escape");
        let before: Vec<_> = std::fs::read_dir(&s.0).unwrap().collect();
        assert!(write_scoped(&s.0, "../escape.txt", b"x").is_err());
        assert!(write_scoped(&s.0, "/tmp/escape.txt", b"x").is_err());
        // A rejected write must not have created files or parent dirs.
        let after: Vec<_> = std::fs::read_dir(&s.0).unwrap().collect();
        assert_eq!(before.len(), after.len());
    }

    #[test]
    fn read_of_missing_file_is_an_error_not_a_panic() {
        let s = Scratch::new("missing");
        assert!(read_scoped(&s.0, "nope.txt").is_err());
    }

    // ── symlink escape ────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_root_is_refused_for_read_and_write() {
        use std::os::unix::fs::symlink;
        let inside = Scratch::new("symlink-in");
        let outside = Scratch::new("symlink-out");
        std::fs::write(outside.0.join("secret"), b"top secret").unwrap();

        // A hostile plugin bundle plants `escape -> <somewhere else>`
        // inside its own data dir.
        symlink(&outside.0, inside.0.join("escape")).unwrap();

        assert!(
            read_scoped(&inside.0, "escape/secret").is_err(),
            "read through an escaping symlink must be refused"
        );
        assert!(
            write_scoped(&inside.0, "escape/pwned.txt", b"x").is_err(),
            "write through an escaping symlink must be refused"
        );
        assert!(
            !outside.0.join("pwned.txt").exists(),
            "the refused write must not have touched the link target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_interior_symlink_that_stays_inside_root_is_allowed() {
        use std::os::unix::fs::symlink;
        let s = Scratch::new("symlink-interior");
        std::fs::create_dir_all(s.0.join("real")).unwrap();
        symlink(s.0.join("real"), s.0.join("link")).unwrap();

        write_scoped(&s.0, "link/x.txt", b"ok").unwrap();
        assert_eq!(std::fs::read(s.0.join("real/x.txt")).unwrap(), b"ok");
        assert_eq!(read_scoped(&s.0, "link/x.txt").unwrap(), b"ok");
    }
}
