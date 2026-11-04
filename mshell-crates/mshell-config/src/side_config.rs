//! Shared rules for small side files mshell reads and writes next to its
//! profiles (notes, chat history, tool settings).
//!
//! The rule from the profile loader applies to all of them: a file that does
//! not parse is never replaced by defaults. Keep a copy, warn, and refuse to
//! save until the user fixes it.

use std::path::{Path, PathBuf};

/// Copy a file that failed to parse to `<file>.bak` (overwriting an older
/// copy) and return the copy's path. The original stays in place.
pub fn backup_broken(path: &Path) -> std::io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    let copy = path.with_file_name(name);
    std::fs::copy(path, &copy)?;
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_sits_next_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        std::fs::write(&path, "{oops").unwrap();
        let bak = backup_broken(&path).unwrap();
        assert_eq!(bak, dir.path().join("ai.json.bak"));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{oops");
        assert!(path.exists());
    }
}
