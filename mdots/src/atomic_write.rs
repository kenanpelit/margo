//! Small atomic-write helper for mdots' own state files.
//!
//! Plain `fs::write` truncates the destination in place — a crash or
//! disk-full mid-write leaves a corrupt/empty file where a good one used
//! to be, and since these are the tool's own bookkeeping (dotfiles state,
//! secrets state, theming state), a corrupt file means `mdots` forgets
//! what it previously tracked. Write to a sibling temp file, fsync it,
//! then rename over the target (atomic on the same filesystem).
//!
//! Doesn't do the symlink-preserving dance `mshell-config`'s
//! `atomic_write` does — these are internal state files `mdots` itself
//! creates and never a stow-managed dotfile target, so there's nothing
//! to preserve.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Atomically write `contents` to `path`, creating parent directories
/// as needed.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory for {path:?}"))?;
    }
    let tmp = path.with_extension("mdots-tmp");
    {
        use std::io::Write as _;
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating temp file for {path:?}"))?;
        f.write_all(contents.as_bytes())
            .with_context(|| format!("writing temp file for {path:?}"))?;
        // fsync the data before the rename: rename is only atomic w.r.t.
        // *metadata* — without this a crash right after it can leave the
        // target existing but truncated.
        f.sync_all()
            .with_context(|| format!("syncing temp file for {path:?}"))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming into place: {path:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_creates_parent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/state.yaml");
        write_atomic(&path, "hello: world\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello: world\n");
    }

    #[test]
    fn overwrites_existing_file_and_leaves_no_tmp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.yaml");
        fs::write(&path, "old\n").unwrap();
        write_atomic(&path, "new\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert!(!path.with_extension("mdots-tmp").exists());
    }
}
