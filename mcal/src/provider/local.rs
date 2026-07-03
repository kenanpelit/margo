//! Local provider — `.ics` files and directories under a root, mirroring
//! dankcalendar's `internal/providers/local`.
//!
//! Layout: a `.ics` file directly under the root is one calendar
//! (`file:<name>`); a sub-directory is one calendar (`dir:<name>`) aggregating
//! the `.ics` files inside it (non-recursive). The root is created if missing.

use super::{Provider, Window};
use crate::error::McalError;
use crate::ics::parse_ics;
use crate::model::Calendar;
use crate::model::Event;
use crate::recur;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A calendar source backed by a local directory of `.ics` files.
pub struct LocalProvider {
    account_id: String,
    root: PathBuf,
}

impl LocalProvider {
    /// Open (creating if absent) the local calendar root.
    pub fn new(account_id: impl Into<String>, root: impl AsRef<Path>) -> Result<Self, McalError> {
        let root = root.as_ref().to_path_buf();
        match fs::metadata(&root) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(McalError::Io {
                    path: root.display().to_string(),
                    source: io::Error::other("local calendar root is not a directory"),
                });
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(&root).map_err(|source| McalError::Io {
                    path: root.display().to_string(),
                    source,
                })?;
            }
            Err(source) => {
                return Err(McalError::Io {
                    path: root.display().to_string(),
                    source,
                });
            }
        }
        Ok(Self {
            account_id: account_id.into(),
            root,
        })
    }

    fn calendar(&self, remote_id: String, name: String) -> Calendar {
        Calendar {
            account_id: self.account_id.clone(),
            remote_id,
            name,
            color: None,
        }
    }

    /// `(path, calendar_id)` for every `.ics` file to read.
    fn sources(&self) -> Result<Vec<(PathBuf, String)>, McalError> {
        let mut sources = Vec::new();
        for entry in read_dir(&self.root)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                let cal_id = format!("dir:{name}");
                for inner in read_dir(&path)? {
                    let inner_path = inner.path();
                    if is_ics(&inner_path) {
                        sources.push((inner_path, cal_id.clone()));
                    }
                }
            } else if is_ics(&path) {
                sources.push((path, format!("file:{name}")));
            }
        }
        Ok(sources)
    }
}

impl Provider for LocalProvider {
    fn calendars(&self) -> Result<Vec<Calendar>, McalError> {
        let mut calendars = Vec::new();
        for entry in read_dir(&self.root)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                calendars.push(self.calendar(format!("dir:{name}"), name));
            } else if is_ics(&path) {
                let stem = trim_ics(&name);
                calendars.push(self.calendar(format!("file:{name}"), stem));
            }
        }
        Ok(calendars)
    }

    fn events(&self, window: Window) -> Result<Vec<Event>, McalError> {
        let mut out = Vec::new();
        for (path, calendar_id) in self.sources()? {
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "mcal: unreadable .ics, skipping");
                    continue;
                }
            };
            match parse_ics(&text, &calendar_id) {
                Ok(events) => {
                    for event in &events {
                        out.extend(recur::expand(event, window.0, window.1));
                    }
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "mcal: malformed .ics, skipping")
                }
            }
        }
        Ok(out)
    }
}

/// Read a directory into owned entries, mapping IO errors to [`McalError`].
fn read_dir(path: &Path) -> Result<Vec<fs::DirEntry>, McalError> {
    let iter = fs::read_dir(path).map_err(|source| McalError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut entries = Vec::new();
    for entry in iter {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(err) => tracing::warn!(path = %path.display(), %err, "mcal: skipping dir entry"),
        }
    }
    Ok(entries)
}

fn is_ics(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ics"))
}

fn trim_ics(name: &str) -> String {
    name.strip_suffix(".ics")
        .or_else(|| name.strip_suffix(".ICS"))
        .unwrap_or(name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const ONE_EVENT: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a@x\r\nSUMMARY:A\r\nDTSTART:20260703T090000Z\r\nDTEND:20260703T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn wide_window() -> Window {
        (
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
        )
    }

    #[test]
    fn missing_root_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("calendars");
        assert!(!root.exists());
        let provider = LocalProvider::new("local", &root).expect("new");
        assert!(root.is_dir());
        assert!(provider.calendars().unwrap().is_empty());
    }

    #[test]
    fn reads_files_and_subdir_calendars() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("work.ics"), ONE_EVENT).unwrap();
        let sub = root.join("personal");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("birthdays.ics"), ONE_EVENT).unwrap();
        // A non-ics file is ignored.
        fs::write(root.join("notes.txt"), "ignore me").unwrap();

        let provider = LocalProvider::new("local", root).expect("new");

        let cals = provider.calendars().unwrap();
        assert_eq!(cals.len(), 2, "one file cal + one dir cal");
        assert!(
            cals.iter()
                .any(|c| c.remote_id == "file:work.ics" && c.name == "work")
        );
        assert!(cals.iter().any(|c| c.remote_id == "dir:personal"));

        let events = provider.events(wide_window()).unwrap();
        assert_eq!(events.len(), 2, "one event from each calendar");
        assert!(events.iter().any(|e| e.calendar_id == "file:work.ics"));
        assert!(events.iter().any(|e| e.calendar_id == "dir:personal"));
    }

    #[test]
    fn rejects_non_directory_root() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not_a_dir");
        fs::write(&file, "x").unwrap();
        assert!(LocalProvider::new("local", &file).is_err());
    }
}
