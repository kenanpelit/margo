//! Guarded load/save for small side files (tool settings, notes, history).
//!
//! The rule the profile loader already follows, for every other file mshell
//! edits: a file that does not parse is never replaced by defaults. Loading
//! it keeps a `<file>.bak` copy and reports [`SideLoad::Broken`]; saving
//! re-checks the file on disk and refuses ([`SaveError::Unparseable`]) while
//! it still does not parse, so the user's hand edits survive until fixed.

use std::fmt::Display;
use std::path::{Path, PathBuf};

/// Result of [`load`].
#[derive(Debug, PartialEq)]
pub enum SideLoad<T> {
    /// No file yet: use defaults; the first save creates it.
    Missing,
    Loaded(T),
    /// The file exists but does not parse (or could not be read). A copy is
    /// at `backup` when one could be made. Use defaults for display, but do
    /// not save over it.
    Broken {
        error: String,
        backup: Option<PathBuf>,
    },
}

impl<T: Default> SideLoad<T> {
    /// The loaded value, or defaults for a missing/broken file.
    pub fn or_default(self) -> T {
        match self {
            SideLoad::Loaded(v) => v,
            SideLoad::Missing | SideLoad::Broken { .. } => T::default(),
        }
    }
}

/// Why [`save`] did not write.
#[derive(Debug)]
pub enum SaveError {
    /// The file on disk does not parse; it was left untouched.
    Unparseable { path: PathBuf, error: String },
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Unparseable { path, error } => write!(
                f,
                "{} does not parse ({error}); not saving over it — fix it or move it away",
                path.display()
            ),
            SaveError::Io { path, error } => write!(f, "writing {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for SaveError {}

/// Copy a file that failed to parse to `<file>.bak` (overwriting an older
/// copy) and return the copy's path. The original stays in place.
pub fn backup_broken(path: &Path) -> std::io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    let copy = path.with_file_name(name);
    std::fs::copy(path, &copy)?;
    Ok(copy)
}

/// Read and parse `path`. A parse error keeps a `.bak` copy.
pub fn load<T, E: Display>(path: &Path, parse: impl FnOnce(&str) -> Result<T, E>) -> SideLoad<T> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SideLoad::Missing,
        Err(e) => {
            return SideLoad::Broken {
                error: e.to_string(),
                backup: None,
            };
        }
    };
    match parse(&raw) {
        Ok(v) => SideLoad::Loaded(v),
        Err(e) => SideLoad::Broken {
            error: e.to_string(),
            backup: backup_broken(path).ok(),
        },
    }
}

/// Atomically write `body` to `path`, unless the file already there does not
/// parse with `check` (then it is left alone and a `.bak` copy is kept).
pub fn save<E: Display>(
    path: &Path,
    body: &[u8],
    check: impl FnOnce(&str) -> Result<(), E>,
) -> Result<(), SaveError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            if let Err(e) = check(&raw) {
                let _ = backup_broken(path);
                return Err(SaveError::Unparseable {
                    path: path.to_path_buf(),
                    error: e.to_string(),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Unreadable (e.g. not UTF-8): don't replace what we can't inspect.
        Err(e) => {
            return Err(SaveError::Unparseable {
                path: path.to_path_buf(),
                error: e.to_string(),
            });
        }
    }
    crate::atomic_write::atomic_write(path, body).map_err(|error| SaveError::Io {
        path: path.to_path_buf(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_num(s: &str) -> Result<u32, std::num::ParseIntError> {
        s.trim().parse()
    }

    fn check_num(s: &str) -> Result<(), std::num::ParseIntError> {
        parse_num(s).map(|_| ())
    }

    #[test]
    fn missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("x"), parse_num), SideLoad::Missing);
    }

    #[test]
    fn good_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        std::fs::write(&p, "42").unwrap();
        assert_eq!(load(&p, parse_num), SideLoad::Loaded(42));
    }

    /// A broken file: load reports it and keeps a .bak; save refuses and
    /// leaves the file byte for byte; after the user fixes it, save works.
    #[test]
    fn broken_file_is_backed_up_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ai.json");
        std::fs::write(&p, "4x2").unwrap();
        let SideLoad::Broken { backup, .. } = load(&p, parse_num) else {
            panic!("expected Broken");
        };
        let bak = backup.expect("a copy was made");
        assert_eq!(bak, dir.path().join("ai.json.bak"));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "4x2");

        let err = save(&p, b"7", check_num).unwrap_err();
        assert!(matches!(err, SaveError::Unparseable { .. }), "{err}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "4x2");

        std::fs::write(&p, "1").unwrap();
        save(&p, b"7", check_num).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "7");
    }

    #[test]
    fn save_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/x");
        save(&p, b"5", check_num).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "5");
    }
}
