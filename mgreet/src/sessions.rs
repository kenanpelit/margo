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
    let per_dir: Vec<Vec<String>> = SESSION_DIRS
        .iter()
        .map(|dir| match fs::read_dir(dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
                .filter_map(|p| read_name(&p))
                .collect(),
            Err(_) => Vec::new(),
        })
        .collect();
    merge(&per_dir)
}

/// De-duplicate the per-directory name lists into the final session order:
/// names sorted within each directory, earlier directories winning ties (so a
/// session present in both wayland- and xsessions keeps its first, Wayland,
/// listing). Split from [`list`] so the ordering rule is testable without a
/// filesystem.
fn merge(per_dir: &[Vec<String>]) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for names in per_dir {
        let mut names = names.clone();
        names.sort();
        for name in names {
            if !out.iter().any(|s| s.name == name) {
                out.push(Session { name });
            }
        }
    }
    out
}

/// The drop-down index to pre-select: the position of the last-used session
/// name, or `None` (leave the default, index 0) when there's no hint or it names
/// a session that is no longer present.
pub fn select_index(sessions: &[Session], initial: Option<&str>) -> Option<u32> {
    let want = initial?;
    sessions
        .iter()
        .position(|s| s.name == want)
        .map(|i| i as u32)
}

/// Read the `Name=` from the `[Desktop Entry]` group of a `.desktop` file.
fn read_name(path: &Path) -> Option<String> {
    parse_name(&fs::read_to_string(path).ok()?)
}

/// Parse the `Name=` from the `[Desktop Entry]` group of `.desktop` text. Split
/// from the filesystem read so it can be tested from a `&str`. Only the first
/// non-empty `Name=` inside `[Desktop Entry]` counts; keys in other groups (e.g.
/// `[Desktop Action …]`) are ignored.
fn parse_name(text: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(name: &str) -> Session {
        Session {
            name: name.to_string(),
        }
    }

    #[test]
    fn parses_a_well_formed_entry() {
        let text = "[Desktop Entry]\nName=GNOME\nExec=gnome-session\nType=Application\n";
        assert_eq!(parse_name(text).as_deref(), Some("GNOME"));
    }

    #[test]
    fn missing_name_yields_none() {
        // mgreet keys sessions on `Name=`, not `Exec=`; a file without it is skipped.
        let text = "[Desktop Entry]\nExec=sway\nType=Application\n";
        assert_eq!(parse_name(text), None);
    }

    #[test]
    fn empty_name_value_is_ignored() {
        assert_eq!(parse_name("[Desktop Entry]\nName=\n"), None);
    }

    #[test]
    fn name_outside_desktop_entry_group_is_ignored() {
        let text = "[Desktop Action New]\nName=New Window\n";
        assert_eq!(parse_name(text), None);
    }

    #[test]
    fn first_name_in_entry_wins() {
        let text = "[Desktop Entry]\nName=Sway\nName=Ignored\n";
        assert_eq!(parse_name(text).as_deref(), Some("Sway"));
    }

    #[test]
    fn malformed_or_empty_text_yields_none() {
        assert_eq!(parse_name("not a desktop file at all"), None);
        assert_eq!(parse_name(""), None);
    }

    #[test]
    fn leading_whitespace_is_trimmed() {
        let text = "  [Desktop Entry]\n   Name=Hyprland  \n";
        assert_eq!(parse_name(text).as_deref(), Some("Hyprland"));
    }

    #[test]
    fn merge_sorts_within_dir_and_first_dir_wins_ties() {
        let wayland = vec!["Sway".to_string(), "GNOME".to_string()];
        let xsessions = vec!["GNOME".to_string(), "i3".to_string()];
        let out: Vec<String> = merge(&[wayland, xsessions])
            .into_iter()
            .map(|s| s.name)
            .collect();
        // Wayland sorted (GNOME, Sway), then x11's i3; the duplicate GNOME drops.
        assert_eq!(out, ["GNOME", "Sway", "i3"]);
    }

    #[test]
    fn merge_of_empty_dirs_is_empty() {
        assert!(merge(&[]).is_empty());
        assert!(merge(&[Vec::new(), Vec::new()]).is_empty());
    }

    #[test]
    fn select_index_finds_the_last_used_session() {
        let sessions = [sess("GNOME"), sess("Sway"), sess("i3")];
        assert_eq!(select_index(&sessions, Some("Sway")), Some(1));
    }

    #[test]
    fn select_index_defaults_when_absent_or_unset() {
        let sessions = [sess("GNOME"), sess("Sway")];
        assert_eq!(select_index(&sessions, Some("KDE")), None);
        assert_eq!(select_index(&sessions, None), None);
        assert_eq!(select_index(&[], Some("GNOME")), None);
    }
}
