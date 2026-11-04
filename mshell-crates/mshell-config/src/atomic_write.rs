//! Symlink-preserving atomic file write — see [`margo_fs::atomic_write`],
//! which holds the sync version (shared with tools that don't use tokio).
//! This module re-exports it and adds the tokio flavour.

use std::path::Path;

pub use margo_fs::atomic_write::atomic_write;
use margo_fs::atomic_write::{resolve_symlink_target, tmp_sibling};

/// Async wrapper for use from tokio-based code. Same semantics as
/// [`atomic_write`] — symlinks survive, regular files get the
/// conventional behaviour.
pub async fn atomic_write_async(
    path: &Path,
    contents: impl AsRef<[u8]> + Send + 'static,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let target = resolve_symlink_target(path);
    let tmp = tmp_sibling(&target);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Same durability rationale as the sync path: fsync data before rename,
    // then fsync the parent dir after.
    {
        let mut f = tokio::fs::File::create(&tmp).await?;
        f.write_all(contents.as_ref()).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, &target).await?;
    if let Some(parent) = target.parent()
        && let Ok(dir) = tokio::fs::File::open(parent).await
    {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn async_write_lands_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/doc.json");
        atomic_write_async(&path, b"hi".to_vec()).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hi");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
