//! "Is this tool installed?" — `$PATH` lookup without forking `which`.
//!
//! Every margo tool and mshell probes for optional helpers (`flatpak`,
//! `scrcpy`, `notify-send`, a terminal …). This is the one rule they share:
//! a plain filesystem walk over `$PATH` that only accepts **executable
//! regular files** (a stray non-executable file of the same name is not a
//! hit). It never runs `which` (not in Arch `base`) or `sh -c "command -v"`,
//! so it is cheap enough to call on every menu reveal and works on a system
//! without `which`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Absolute path of `bin`: the first executable file named `bin` in one of
/// the `$PATH` directories. A `bin` containing `/` is checked as-is.
pub fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in_path_in(bin, &path)
}

/// `true` when [`find_in_path`] finds `bin`.
pub fn on_path(bin: &str) -> bool {
    find_in_path(bin).is_some()
}

/// [`find_in_path`] against an explicit `PATH` value (testable seam).
pub fn find_in_path_in(bin: &str, path: &OsStr) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    if bin.contains('/') {
        let p = Path::new(bin);
        return is_executable(p).then(|| p.to_path_buf());
    }
    std::env::split_paths(path)
        // An empty PATH entry means "the current directory" to a shell;
        // a long-lived service must not resolve tools against its cwd.
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(bin))
        .find(|candidate| is_executable(candidate))
}

/// [`on_path`] against an explicit `PATH` value (testable seam).
pub fn on_path_in(bin: &str, path: &OsStr) -> bool {
    find_in_path_in(bin, path).is_some()
}

/// A regular file (symlinks followed) with any execute bit set.
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    fn write_file(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn finds_sh_and_rejects_missing() {
        let path = OsString::from("/nonexistent:/usr/bin:/bin");
        assert!(on_path_in("sh", &path));
        assert!(!on_path_in("definitely-not-a-real-tool-xyz", &path));
        assert!(!on_path_in("", &path));
    }

    #[test]
    fn absolute_path_is_checked_directly() {
        let empty = OsString::new();
        assert!(on_path_in("/bin/sh", &empty));
        assert!(!on_path_in("/nonexistent/sh", &empty));
    }

    /// Several copies used `dir.join(bin).is_file()`: a non-executable file
    /// of the same name counted as "installed" and the spawn then failed.
    #[test]
    fn a_non_executable_file_is_not_a_tool() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "tool", 0o644);
        assert!(!on_path_in("tool", dir.path().as_os_str()));
    }

    #[test]
    fn a_directory_is_not_a_tool() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tool")).unwrap();
        assert!(!on_path_in("tool", dir.path().as_os_str()));
    }

    /// First hit wins, in `$PATH` order — like a shell, so a tool shadowed
    /// in `~/.local/bin` is the one we report.
    #[test]
    fn first_path_entry_wins_and_skips_non_executables() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let c = tempfile::tempdir().unwrap();
        write_file(a.path(), "tool", 0o644);
        let want = write_file(b.path(), "tool", 0o755);
        write_file(c.path(), "tool", 0o755);
        let path = std::env::join_paths([a.path(), b.path(), c.path()]).unwrap();
        assert_eq!(find_in_path_in("tool", &path), Some(want));
    }
}
