//! Opening mlogind's log files.
//!
//! Every log mlogind writes is private to its writer: the daemon log and the
//! session client log in `/var/log` (root), the greeter host's log in
//! `/run/mlogind`. They hold login events and each user's session output, so
//! no other local account may read them. `File::create` would leave them at
//! 0644 under the usual umask 022, and keep the 0644 of a file an older
//! version created.

use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// The mode every mlogind log file gets: owner read/write only.
pub const LOG_MODE: u32 = 0o600;

/// Create or truncate `path` as a private (0600) log file.
///
/// A file that already exists keeps its inode, so its mode is reset with an
/// `fchmod` on the open descriptor. `O_NOFOLLOW` refuses a symlink planted in
/// the log's place.
pub fn open_private_log(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(LOG_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.set_permissions(Permissions::from_mode(LOG_MODE))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mlogind-logfile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_log_is_owner_only() {
        let dir = scratch("new");
        let path = dir.join("x.log");
        open_private_log(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_old_world_readable_log_is_tightened_and_truncated() {
        let dir = scratch("old");
        let path = dir.join("x.log");
        std::fs::write(&path, "old secret").unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        open_private_log(&path).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o600);
        assert_eq!(meta.len(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlink_in_the_logs_place_is_refused() {
        let dir = scratch("link");
        let target = dir.join("target");
        std::fs::write(&target, "keep").unwrap();
        let path = dir.join("x.log");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(open_private_log(&path).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
