//! Crash-safe replacement of the active-layout symlink.
//!
//! File contents go through `margo_fs::atomic_write` (fsynced tmp + rename,
//! symlink-preserving — a stow-managed `config.conf` keeps its link). The
//! active-layout symlink itself is swapped the same way here: build the new
//! link at a sibling temp path, then `rename` it over the old one.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Atomically point the symlink `link` at `target`: create the new link at a
/// sibling temp path and rename it over `link`, so a reader (a racing
/// `mctl reload`) sees either the old link or the new one, never none.
pub fn replace_symlink(target: &Path, link: &Path) -> Result<()> {
    let tmp = margo_fs::atomic_write::tmp_sibling(link);
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
