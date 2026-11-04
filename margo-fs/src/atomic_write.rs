//! Symlink-preserving atomic file write.
//!
//! Many dotfile-management systems (`stow`, `dcli`, `chezmoi`,
//! `yadm`, etc.) put the canonical file in a tracked directory and
//! symlink the published path (e.g. `~/.config/margo/config.conf`)
//! at it. The usual atomic-write recipe — write a sibling `.tmp`
//! then `rename` over the target — replaces the symlink with the
//! regular tmp file, which silently breaks the dotfile link.
//! Subsequent `dcli stow` / `chezmoi apply` runs see a divergence
//! and the user discovers their tracked config no longer reflects
//! what's actually being read.
//!
//! This helper resolves the symlink first (one level — does not
//! recurse through chains) and writes the tmp + rename against
//! the **resolved** target. The symlink at `path` continues to
//! point at the same file, which now carries the new content.
//! Behaviour for non-symlinked paths is identical to the
//! conventional pattern.

use std::path::{Path, PathBuf};

/// Atomically write `contents` to `path`.
///
/// * If `path` is a symlink, the symlink survives; the target file
///   gets the new content via tmp-sibling + rename against the
///   target.
/// * If `path` is a regular file (or doesn't exist), the rename
///   lands on `path` directly — same effect as the conventional
///   write-then-rename idiom.
///
/// Parent directories of the resolved target are created if
/// missing. Errors surface from the underlying I/O calls; failures
/// during rename leave the tmp file behind for caller-side
/// inspection (we don't best-effort delete on failure because that
/// can mask the original error).
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let target = resolve_symlink_target(path);
    let tmp = tmp_sibling(&target);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // fsync the data before the rename: rename is only atomic w.r.t.
    // *metadata*. Without this, a crash/power-loss right after the rename
    // can leave the target existing but empty/truncated — the user's whole
    // config resets to defaults on next boot.
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target)?;
    fsync_parent_dir(&target);
    Ok(())
}

/// Best-effort fsync of a path's parent directory so the rename entry
/// itself is durable. Errors are ignored — some filesystems don't support
/// directory fsync, and the data is already durable at this point.
pub fn fsync_parent_dir(target: &Path) {
    if let Some(parent) = target.parent()
        && let Ok(dir) = std::fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
}

/// Resolve `path` through one level of symlink dereference. Returns
/// the canonical target path of the link, or `path` unchanged if
/// it's not a symlink (or doesn't exist).
///
/// We deliberately do NOT recurse through symlink chains — most
/// dotfile managers produce one-level links and recursing would
/// cross-mount, follow user-confusing chains, and complicate the
/// error path. If the user has a chain they're probably doing it
/// on purpose; the one-level resolve still preserves their *first*
/// link, which is the one tracked by `dcli`/`stow`.
pub fn resolve_symlink_target(path: &Path) -> PathBuf {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return path.to_path_buf();
    };
    if !meta.file_type().is_symlink() {
        return path.to_path_buf();
    }
    let Ok(link) = std::fs::read_link(path) else {
        return path.to_path_buf();
    };
    if link.is_absolute() {
        link
    } else {
        // Relative symlinks are resolved against the link's parent
        // directory, not the current working dir.
        path.parent().map(|p| p.join(&link)).unwrap_or(link)
    }
}

/// Build the temp-file path next to `target` so the subsequent
/// `rename(tmp, target)` is a same-filesystem atomic operation
/// (required for atomicity — cross-filesystem rename falls back
/// to copy + unlink, which loses atomicity AND breaks if the dest
/// device is full).
///
/// The name is unique per call (`.mshell-tmp-<pid>-<n>`): two overlapping
/// writes to one path must not share a tmp file, or the second `create`
/// truncates the first writer's file and the rename can publish a newer
/// prefix glued to an older tail.
pub fn tmp_sibling(target: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut file_name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(format!(".mshell-tmp-{}-{n}", std::process::id()));
    target.with_file_name(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn writes_through_symlink_preserves_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.conf");
        let link = dir.path().join("link.conf");
        std::fs::write(&real, b"old").unwrap();
        symlink(&real, &link).unwrap();

        atomic_write(&link, b"new").unwrap();

        // The link is still a symlink, the real file got the new content.
        let link_meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(link_meta.file_type().is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
    }

    #[test]
    fn writes_regular_file_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.conf");
        std::fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    /// Two overlapping writes to one path must never share a tmp file: with
    /// one fixed `<file>.mshell-tmp`, the second `File::create` truncates the
    /// first writer's inode and the rename can publish a newer prefix glued to
    /// an older tail.
    #[test]
    fn tmp_names_are_unique_per_call() {
        let target = Path::new("/x/notes.json");
        let a = tmp_sibling(target);
        let b = tmp_sibling(target);
        assert_ne!(a, b, "each write gets its own tmp file");
        assert_eq!(a.parent(), target.parent(), "same dir, so rename is atomic");
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("notes.json.mshell-tmp-"), "{name}");
    }

    /// Many threads writing different full documents to one path: the
    /// result is always exactly one of them, and no tmp file is left over.
    #[test]
    fn concurrent_writes_publish_one_whole_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        let docs: Vec<Vec<u8>> = (0..8u8)
            .map(|i| vec![b'a' + i; 4096 * (usize::from(i) + 1)])
            .collect();
        std::thread::scope(|s| {
            for doc in &docs {
                let path = path.clone();
                s.spawn(move || {
                    for _ in 0..20 {
                        atomic_write(&path, doc).unwrap();
                    }
                });
            }
        });
        let got = std::fs::read(&path).unwrap();
        assert!(docs.contains(&got), "torn write: {} bytes", got.len());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != "doc.json")
            .collect();
        assert!(leftovers.is_empty(), "tmp files left: {leftovers:?}");
    }

    #[test]
    fn creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c/x.conf");
        atomic_write(&nested, b"hi").unwrap();
        assert_eq!(std::fs::read(&nested).unwrap(), b"hi");
    }
}
