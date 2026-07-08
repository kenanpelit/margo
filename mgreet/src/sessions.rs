//! Enumerate available login sessions from freedesktop `.desktop` entries.
//!
//! The `Name=` field is used both as the label and as the hand-off key, so it
//! must match what the mlogind orchestrator's `get_envs` produces (it derives
//! the same names from the same session dirs) for the credential hand-off —
//! which selects the session by name — to round-trip.

use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Session {
    /// The `.desktop` `Name=` — display label and orchestrator hand-off key.
    pub name: String,
}

const SESSION_DIRS: &[&str] = &["/usr/share/wayland-sessions", "/usr/share/xsessions"];

/// All sessions found under the standard dirs, de-duplicated by name and in a
/// stable order (Wayland first). Never fails: a missing dir is skipped.
pub fn list() -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for dir in SESSION_DIRS {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
            .filter_map(|p| read_name(&p))
            .collect();
        names.sort();
        for name in names {
            if !out.iter().any(|s| s.name == name) {
                out.push(Session { name });
            }
        }
    }
    out
}

/// Read the `Name=` from the `[Desktop Entry]` group of a `.desktop` file.
fn read_name(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(group) = line.strip_prefix('[') {
            in_entry = group.starts_with("Desktop Entry");
            continue;
        }
        if in_entry && let Some(value) = line.strip_prefix("Name=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
