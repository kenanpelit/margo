//! Runtime detection of optional external tools (nmcli, iw, speedtest,
//! pactl, …). Features built on a tool that may be missing call
//! [`on_path`] and degrade to a calm "not available" hint instead of
//! spawning a doomed process.

use std::path::Path;

/// `true` when `bin` is an executable file in one of the `$PATH`
/// directories. A plain filesystem walk — no `which` fork — so it is cheap
/// enough to call on every menu reveal.
pub fn on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    on_path_in(bin, &path)
}

/// [`on_path`] against an explicit `PATH` value (testable seam).
pub fn on_path_in(bin: &str, path: &std::ffi::OsStr) -> bool {
    if bin.is_empty() || bin.contains('/') {
        return !bin.is_empty() && is_executable(Path::new(bin));
    }
    std::env::split_paths(path).any(|dir| is_executable(&dir.join(bin)))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sh_and_rejects_missing() {
        let path = std::ffi::OsString::from("/nonexistent:/usr/bin:/bin");
        assert!(on_path_in("sh", &path));
        assert!(!on_path_in("definitely-not-a-real-tool-xyz", &path));
        assert!(!on_path_in("", &path));
    }

    #[test]
    fn absolute_path_is_checked_directly() {
        let empty = std::ffi::OsString::new();
        assert!(on_path_in("/bin/sh", &empty));
        assert!(!on_path_in("/nonexistent/sh", &empty));
    }
}
