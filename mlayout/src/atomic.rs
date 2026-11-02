//! Crash-safe file replacement for everything mlayout writes.
//!
//! A plain `fs::write` truncates the destination first, so a crash, ^C or
//! full disk mid-write leaves a half-written layout file or — worse — a
//! truncated `config.conf`. Every write goes through here instead: write a
//! unique sibling temp file, fsync it, then `rename` it over the target
//! (atomic on one filesystem). The active-layout symlink is swapped the same
//! way.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Unique sibling temp path for `path` (same directory, so the final rename
/// never crosses a filesystem).
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.mlayout-tmp", std::process::id()))
}

/// Atomically replace `path` with `contents`.
///
/// If `path` is a symlink (a stow-managed `config.conf`, say), the file it
/// points at is replaced and the link is left alone — the same thing the
/// plain `fs::write` this replaces did, without its truncation window.
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)
            .with_context(|| format!("resolve symlink {}", path.display()))?,
        _ => path.to_path_buf(),
    };
    let tmp = temp_sibling(&target);
    let result = (|| {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(contents.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        // Data must be on disk before the rename publishes it.
        f.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
        std::fs::rename(&tmp, &target)
            .with_context(|| format!("rename {} → {}", tmp.display(), target.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Atomically point the symlink `link` at `target`: create the new link at a
/// sibling temp path and rename it over `link`, so a reader (a racing
/// `mctl reload`) sees either the old link or the new one, never none.
pub fn replace_symlink(target: &Path, link: &Path) -> Result<()> {
    let tmp = temp_sibling(link);
    // Clean any leftover from a previous crash.
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp)
        .with_context(|| format!("create symlink at {}", tmp.display()))?;
    if let Err(err) = std::fs::rename(&tmp, link) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("rename {} → {}", tmp.display(), link.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh empty directory under the system temp dir.
    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "mlayout-atomic-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn write_creates_and_replaces_without_leftovers() {
        let dir = scratch("write");
        let p = dir.join("layout_a.conf");
        write_atomic(&p, "one\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\n");
        write_atomic(&p, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two\n");
        assert_eq!(entries(&dir), vec!["layout_a.conf".to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_through_a_symlink_keeps_the_link() {
        let dir = scratch("symlink");
        let real = dir.join("real.conf");
        std::fs::write(&real, "old\n").unwrap();
        let link = dir.join("config.conf");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomic(&link, "new\n").unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the dotfile symlink must survive"
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
        assert_eq!(
            entries(&dir),
            vec!["config.conf".to_string(), "real.conf".to_string()]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_write_leaves_the_original() {
        let dir = scratch("fail");
        // Target is a directory: the rename must fail, and the temp file
        // must be cleaned up.
        let target = dir.join("busy");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), "x").unwrap();
        assert!(write_atomic(&target, "data").is_err());
        assert_eq!(entries(&dir), vec!["busy".to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replace_symlink_swaps_the_target() {
        let dir = scratch("link");
        let link = dir.join("mlayout.conf");
        replace_symlink(Path::new("layouts/layout_a.conf"), &link).unwrap();
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            Path::new("layouts/layout_a.conf")
        );
        replace_symlink(Path::new("layouts/layout_b.conf"), &link).unwrap();
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            Path::new("layouts/layout_b.conf")
        );
        assert_eq!(entries(&dir), vec!["mlayout.conf".to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
