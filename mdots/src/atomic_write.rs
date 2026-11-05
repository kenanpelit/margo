//! mdots' own state files go through the shared `margo_fs::atomic_write`
//! (fsynced unique tmp sibling + rename, parent dirs created, the tmp file
//! removed on failure). Plain `fs::write` truncates in place, so a crash or
//! a full disk would leave a corrupt state file and `mdots` would forget
//! what it tracked. This only adds anyhow context for the callers.

use anyhow::{Context, Result};
#[cfg(test)]
use std::fs;
use std::path::Path;

/// Atomically write `contents` to `path`, creating parent directories
/// as needed.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    margo_fs::atomic_write::atomic_write(path, contents.as_bytes())
        .with_context(|| format!("writing {path:?} atomically"))
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
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.yaml")]);
    }
}
