//! Small helpers shared by providers that wrap optional external tools.
//!
//! Optional tools (`fd`, `plocate`, `qalc`, `pass-otp`, a Chromium
//! browser …) are detected at runtime so a missing package degrades to a
//! hint row instead of a broken action.

use std::path::{Path, PathBuf};

/// Absolute path of `bin` on `$PATH`, if an executable file by that name
/// exists. Pure filesystem lookup — no `which` subprocess, so it is cheap
/// enough for a provider constructor.
pub fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in_dirs(bin, std::env::split_paths(&path))
}

/// `find_in_path` over an explicit directory list (testable).
pub fn find_in_dirs(bin: &str, dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    if bin.is_empty() || bin.contains('/') {
        let p = Path::new(bin);
        return is_executable(p).then(|| p.to_path_buf());
    }
    dirs.into_iter()
        .map(|d| d.join(bin))
        .find(|candidate| is_executable(candidate))
}

/// True when `bin` resolves on `$PATH`.
pub fn command_exists(bin: &str) -> bool {
    find_in_path(bin).is_some()
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Single-quote `s` for `sh -c`, escaping embedded single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Replace a leading `$HOME` with `~` for display.
pub fn tildify(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sh_in_bin_dirs_and_rejects_missing() {
        let dirs = || vec![PathBuf::from("/nonexistent"), PathBuf::from("/usr/bin")];
        assert_eq!(
            find_in_dirs("sh", dirs()),
            Some(PathBuf::from("/usr/bin/sh"))
        );
        assert_eq!(find_in_dirs("definitely-not-a-binary-xyz", dirs()), None);
        assert_eq!(find_in_dirs("", dirs()), None);
    }

    #[test]
    fn quoting_and_tilde() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let home = Path::new("/home/u");
        assert_eq!(tildify(Path::new("/home/u/a/b"), home), "~/a/b");
        assert_eq!(tildify(Path::new("/home/u"), home), "~");
        assert_eq!(tildify(Path::new("/etc/x"), home), "/etc/x");
    }
}
