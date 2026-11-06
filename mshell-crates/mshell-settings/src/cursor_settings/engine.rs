//! Cursor themes: where they live, what each one is called, and the places
//! a desktop reads "the cursor" from. Pure where it can be (the parsers and
//! planners take text and return text), so the page only does IO.
//!
//! The places a cursor choice must reach, and what each reaches:
//!
//! | Place | Key | Reaches |
//! | --- | --- | --- |
//! | margo `config.conf` | `cursor_theme` / `cursor_size` | the compositor's pointer, XWayland, apps margo starts |
//! | `gsettings org.gnome.desktop.interface` | `cursor-theme` / `cursor-size` | GTK 4 / libadwaita, portals |
//! | `~/.config/gtk-3.0/settings.ini`, `gtk-4.0/settings.ini` | `gtk-cursor-theme-name` / `-size` | GTK 3 / GTK 4 apps |
//! | `~/.icons/default/index.theme` | `Inherits` | libXcursor's fallback: X11 / XWayland and Qt apps without the env |
//! | systemd + D-Bus activation environment | `XCURSOR_THEME` / `XCURSOR_SIZE` | Qt, Electron, apps started by D-Bus / systemd |
//!
//! These are the same places `mdots`' theming (`apply_cursor_theme`,
//! `apply_gtk_settings`, `apply_environment_vars`) writes.

use crate::theme_files::ini_value;
use std::path::PathBuf;

/// One installed cursor theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorTheme {
    /// Directory name — what every config key stores.
    pub id: String,
    /// `Name=` from its `index.theme`, else the id.
    pub name: String,
    /// The theme directory.
    pub dir: PathBuf,
}

/// The cursor themes under `dirs`: every directory with a `cursors/`
/// subdirectory, the first one found winning when a name repeats (the
/// lookup order). The `default` alias is not a theme. Sorted by name.
pub(crate) fn discover(dirs: &[PathBuf]) -> Vec<CursorTheme> {
    let mut themes: Vec<CursorTheme> = Vec::new();
    for root in dirs {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            if id == "default" || !dir.join("cursors").is_dir() {
                continue;
            }
            if themes.iter().any(|t| t.id == id) {
                continue;
            }
            let name = std::fs::read_to_string(dir.join("index.theme"))
                .ok()
                .and_then(|t| ini_value(&t, "Icon Theme", "Name"))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            themes.push(CursorTheme { id, name, dir });
        }
    }
    themes.sort_by_key(|t| t.name.to_lowercase());
    themes
}

/// The `~/.icons/default/index.theme` that makes `theme` the fallback.
pub(crate) fn default_index_theme(theme: &str) -> String {
    format!(
        "[Icon Theme]\nName=Default\nComment=Default cursor theme (set by margo)\nInherits={theme}\n"
    )
}

/// Where the cursor is read from, for the "out of step" warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Margo,
    Gsettings,
    Gtk3,
    Gtk4,
    XDefault,
    SessionEnv,
    /// KDE's `kcminputrc` (written only when it exists).
    Kde,
    /// mdots' `~/.mdots/environment` (written only when it exists).
    ShellEnv,
}

impl Place {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Margo => "margo (config.conf)",
            Self::Gsettings => "GNOME settings (gsettings)",
            Self::Gtk3 => "GTK 3 (settings.ini)",
            Self::Gtk4 => "GTK 4 (settings.ini)",
            Self::XDefault => "X11 / XWayland default (~/.icons/default)",
            Self::SessionEnv => "session environment (XCURSOR_*)",
            Self::Kde => "KDE (kcminputrc)",
            Self::ShellEnv => "shell environment (~/.mdots/environment)",
        }
    }
}

/// What one place says: theme and size, each `None` when the place doesn't
/// set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reading {
    pub place: Place,
    pub theme: Option<String>,
    pub size: Option<u32>,
}

/// The places that set a theme or size other than `theme` / `size`. A place
/// that doesn't set a value is not out of step (the others decide).
pub(crate) fn out_of_step(readings: &[Reading], theme: &str, size: u32) -> Vec<Place> {
    readings
        .iter()
        .filter(|r| {
            r.theme.as_deref().is_some_and(|t| t != theme) || r.size.is_some_and(|s| s != size)
        })
        .map(|r| r.place)
        .collect()
}

/// The theme and size to show as "in use": margo's when it sets them, else
/// the first place that does, else `default` / 24.
pub(crate) fn effective(readings: &[Reading]) -> (String, u32) {
    let theme = readings
        .iter()
        .find_map(|r| r.theme.clone())
        .unwrap_or_else(|| "default".to_string());
    let size = readings.iter().find_map(|r| r.size).unwrap_or(24);
    (theme, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn discover_finds_cursor_themes_once_in_lookup_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user = tmp.path().join("user");
        let sys = tmp.path().join("sys");
        let mk = |root: &Path, id: &str, cursors: bool, index: Option<&str>| {
            let d = root.join(id);
            std::fs::create_dir_all(&d).expect("mkdir");
            if cursors {
                std::fs::create_dir_all(d.join("cursors")).expect("mkdir");
            }
            if let Some(text) = index {
                std::fs::write(d.join("index.theme"), text).expect("write");
            }
        };
        mk(&user, "capitaine-cursors", true, None);
        mk(
            &sys,
            "capitaine-cursors",
            true,
            Some("[Icon Theme]\nName=Capitaine (system)\n"),
        );
        mk(
            &sys,
            "breeze_cursors",
            true,
            Some("[Icon Theme]\nName=Breeze\n"),
        );
        mk(&sys, "kora", false, Some("[Icon Theme]\nName=Kora\n"));
        mk(
            &sys,
            "default",
            true,
            Some("[Icon Theme]\nInherits=breeze_cursors\n"),
        );
        let themes = discover(&[user.clone(), sys, tmp.path().join("missing")]);
        let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["breeze_cursors", "capitaine-cursors"]);
        assert_eq!(themes[0].name, "Breeze");
        assert_eq!(
            themes[1].name, "capitaine-cursors",
            "user copy wins, no index"
        );
        assert_eq!(themes[1].dir, user.join("capitaine-cursors"));
    }

    #[test]
    fn default_index_theme_inherits_the_choice() {
        let t = default_index_theme("breeze_cursors");
        assert_eq!(
            ini_value(&t, "Icon Theme", "Inherits").as_deref(),
            Some("breeze_cursors")
        );
    }

    fn reading(place: Place, theme: Option<&str>, size: Option<u32>) -> Reading {
        Reading {
            place,
            theme: theme.map(str::to_string),
            size,
        }
    }

    #[test]
    fn out_of_step_lists_places_that_disagree() {
        let r = [
            reading(Place::Margo, Some("breeze"), Some(24)),
            reading(Place::Gsettings, Some("capitaine-cursors"), Some(24)),
            reading(Place::Gtk3, Some("breeze"), Some(32)),
            reading(Place::Gtk4, None, None),
            reading(Place::SessionEnv, Some("breeze"), None),
        ];
        assert_eq!(
            out_of_step(&r, "breeze", 24),
            [Place::Gsettings, Place::Gtk3]
        );
        assert!(out_of_step(&r[3..], "breeze", 24).is_empty());
    }

    #[test]
    fn effective_prefers_the_first_place_that_sets_a_value() {
        let r = [
            reading(Place::Margo, None, Some(24)),
            reading(Place::Gsettings, Some("capitaine-cursors"), Some(32)),
        ];
        assert_eq!(effective(&r), ("capitaine-cursors".to_string(), 24));
        assert_eq!(effective(&[]), ("default".to_string(), 24));
    }
}
