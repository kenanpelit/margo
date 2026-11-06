//! Shared plumbing for the desktop-theme pickers (Theme → Cursor, Theme →
//! Icons): where icon/cursor themes live, the INI files GTK, Qt and KDE
//! keep their theme names in, `gsettings` / session-env output parsing, and
//! symlink-preserving writes. Text transforms are pure; the writes and the
//! command runner block, so callers run them off the GTK thread.

use std::path::{Path, PathBuf};

/// The icon directories searched for cursor themes, in lookup order:
/// `~/.local/share/icons`, `~/.icons`, then each `$XDG_DATA_DIRS/icons`.
pub(crate) fn icon_dirs(home: &Path, data_home: &Path, data_dirs: &str) -> Vec<PathBuf> {
    let mut dirs = vec![data_home.join("icons"), home.join(".icons")];
    let data_dirs = if data_dirs.trim().is_empty() {
        "/usr/local/share:/usr/share"
    } else {
        data_dirs
    };
    for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
        let p = Path::new(d).join("icons");
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

/// The value of `key` in `[section]` of an INI-style text (case-insensitive
/// section and key names, as GTK and the icon spec read them).
pub(crate) fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = name.trim().eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// `text` with `key=value` set in `[section]`: the existing line is
/// replaced in place, else the key is appended to the section, else the
/// section is appended. Every other line is kept as it was.
pub(crate) fn set_ini_value(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut section_seen = false;
    let mut done = false;
    // Index after the last line of the target section, for appending.
    let mut insert_at: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if in_section && !done {
                insert_at = Some(out.len());
            }
            in_section = name.trim().eq_ignore_ascii_case(section);
            section_seen |= in_section;
            out.push(line.to_string());
            continue;
        }
        if in_section
            && !done
            && let Some((k, _)) = trimmed.split_once('=')
            && !trimmed.starts_with('#')
            && k.trim().eq_ignore_ascii_case(key)
        {
            out.push(format!("{key}={value}"));
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if in_section {
            // The section runs to the end: drop trailing blank lines into
            // place after the new key.
            let mut at = out.len();
            while at > 0 && out[at - 1].trim().is_empty() {
                at -= 1;
            }
            out.insert(at, format!("{key}={value}"));
        } else if let Some(at) = insert_at.filter(|_| section_seen) {
            let mut at = at;
            while at > 0 && out[at - 1].trim().is_empty() {
                at -= 1;
            }
            out.insert(at, format!("{key}={value}"));
        } else {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{section}]"));
            out.push(format!("{key}={value}"));
        }
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// `gsettings get` output (`'Adwaita'`, `24`, `uint32 24`) without quotes
/// or type prefix.
pub(crate) fn parse_gsettings_value(out: &str) -> Option<String> {
    let v = out.trim();
    let v = v.rsplit_once(' ').map_or(v, |(_, last)| last);
    let v = v.trim_matches('\'').trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// `KEY=value` from `systemctl --user show-environment` output.
pub(crate) fn env_value(show_environment: &str, key: &str) -> Option<String> {
    show_environment.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k == key).then(|| v.trim_matches('\'').trim_matches('"').to_string())
    })
}

/// A theme name is written into config files and env assignments: keep it
/// to what icon-theme directory names use.
pub(crate) fn valid_theme_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | ' '))
}

/// The roots every path is resolved against (tests point them at a tempdir).
#[derive(Debug, Clone)]
pub(crate) struct Roots {
    pub home: PathBuf,
    pub config_home: PathBuf,
}

impl Roots {
    pub(crate) fn from_env() -> Self {
        Self {
            home: margo_fs::xdg::home().unwrap_or_default(),
            config_home: margo_fs::xdg::config_home(),
        }
    }

    pub(crate) fn gtk_ini(&self, version: &str) -> PathBuf {
        self.config_home.join(version).join("settings.ini")
    }

    pub(crate) fn x_default_dir(&self) -> PathBuf {
        self.home.join(".icons").join("default")
    }

    pub(crate) fn mdots_env(&self) -> PathBuf {
        self.home.join(".mdots").join("environment")
    }

    pub(crate) fn kcminputrc(&self) -> PathBuf {
        self.config_home.join("kcminputrc")
    }
}

/// Runs `program args…`, returning stdout or why it failed.
pub(crate) type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Result<String, String>;

/// The real runner: skips programs that aren't installed.
pub(crate) fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    if !margo_fs::path::on_path(program) {
        return Err(format!("{program} is not installed"));
    }
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// What applying did to one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Done,
    /// Left alone on purpose (the reason is shown).
    Skipped(String),
    Failed(String),
}

/// `text` (a shell env file of `export KEY="value"` lines) with each
/// `(key, value)` set: an existing `export KEY=` / `KEY=` line is replaced,
/// a missing one appended.
pub(crate) fn set_env_exports(text: &str, pairs: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (key, value) in pairs {
        let new = format!("export {key}=\"{value}\"");
        let hit = lines.iter_mut().find(|l| {
            let t = l.trim_start();
            let t = t.strip_prefix("export ").unwrap_or(t).trim_start();
            t.strip_prefix(key).is_some_and(|r| r.starts_with('='))
        });
        match hit {
            Some(line) => *line = new,
            None => lines.push(new),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Write `text` to `path` in place (a dotfiles symlink stays a symlink).
pub(crate) fn write(path: &Path, text: &str) -> Result<(), String> {
    margo_fs::atomic_write::atomic_write(path, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Set `pairs` in `[section]` of the INI file at `path` (created if missing
/// when `create`, else skipped).
pub(crate) fn edit_ini(path: &Path, section: &str, pairs: &[(&str, String)], create: bool) -> Step {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) if create => String::new(),
        Err(_) => return Step::Skipped(format!("{} does not exist", path.display())),
    };
    let new = pairs
        .iter()
        .fold(text, |t, (k, v)| set_ini_value(&t, section, k, v));
    match write(path, &new) {
        Ok(()) => Step::Done,
        Err(e) => Step::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_dirs_follow_the_lookup_order() {
        let dirs = icon_dirs(
            Path::new("/home/u"),
            Path::new("/home/u/.local/share"),
            "/usr/local/share:/usr/share:/usr/share",
        );
        assert_eq!(
            dirs,
            [
                "/home/u/.local/share/icons",
                "/home/u/.icons",
                "/usr/local/share/icons",
                "/usr/share/icons"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            icon_dirs(Path::new("/h"), Path::new("/h/.local/share"), "").len(),
            4
        );
    }

    #[test]
    fn ini_value_reads_case_insensitively_in_its_section() {
        let t = "[Settings]\ngtk-theme-name=Adwaita\n gtk-cursor-theme-name = capitaine-cursors \n[Other]\ngtk-cursor-theme-name=nope\n";
        assert_eq!(
            ini_value(t, "settings", "GTK-CURSOR-THEME-NAME").as_deref(),
            Some("capitaine-cursors")
        );
        assert_eq!(ini_value(t, "Settings", "missing"), None);
        assert_eq!(
            ini_value("[icon theme]\nInherits=breeze\n", "Icon Theme", "Inherits").as_deref(),
            Some("breeze")
        );
    }

    /// The owner's real `~/.config/gtk-3.0/settings.ini` (2026-11-05).
    const GTK3: &str = "[Settings]\ngtk-theme-name=catppuccin-mocha-mauve-standard+default\ngtk-icon-theme-name=kora\ngtk-font-name=Maple Mono NF 12\ngtk-cursor-theme-name=capitaine-cursors\ngtk-cursor-theme-size=24\ngtk-toolbar-style=GTK_TOOLBAR_BOTH\ngtk-application-prefer-dark-theme=1\ngtk-decoration-layout=appmenu\n";

    #[test]
    fn set_ini_value_replaces_in_place_and_keeps_everything_else() {
        let out = set_ini_value(GTK3, "Settings", "gtk-cursor-theme-name", "breeze_cursors");
        assert_eq!(
            out,
            GTK3.replace(
                "gtk-cursor-theme-name=capitaine-cursors",
                "gtk-cursor-theme-name=breeze_cursors"
            )
        );
        let out = set_ini_value(&out, "Settings", "gtk-cursor-theme-size", "32");
        assert!(out.contains("gtk-cursor-theme-size=32\n"));
        assert_eq!(out.lines().count(), GTK3.lines().count());
    }

    #[test]
    fn set_ini_value_appends_a_missing_key_or_section() {
        let out = set_ini_value("[Settings]\na=1\n\n[Other]\nb=2\n", "Settings", "k", "v");
        assert_eq!(out, "[Settings]\na=1\nk=v\n\n[Other]\nb=2\n");
        let out = set_ini_value("[Settings]\na=1\n\n", "Settings", "k", "v");
        assert_eq!(out, "[Settings]\na=1\nk=v\n\n");
        let out = set_ini_value("[Other]\nb=2\n", "Settings", "k", "v");
        assert_eq!(out, "[Other]\nb=2\n\n[Settings]\nk=v\n");
        assert_eq!(set_ini_value("", "Settings", "k", "v"), "[Settings]\nk=v\n");
        // A commented-out key is not the key.
        let out = set_ini_value("[Settings]\n#k=old\n", "Settings", "k", "v");
        assert_eq!(out, "[Settings]\n#k=old\nk=v\n");
    }

    #[test]
    fn gsettings_and_env_values_parse() {
        // Captured from this machine.
        assert_eq!(
            parse_gsettings_value("'capitaine-cursors'\n").as_deref(),
            Some("capitaine-cursors")
        );
        assert_eq!(parse_gsettings_value("24\n").as_deref(), Some("24"));
        assert_eq!(parse_gsettings_value("uint32 24").as_deref(), Some("24"));
        assert_eq!(parse_gsettings_value("''"), None);
        let env = "HOME=/home/u\nXCURSOR_SIZE=24\nXCURSOR_THEME=capitaine-cursors\n";
        assert_eq!(
            env_value(env, "XCURSOR_THEME").as_deref(),
            Some("capitaine-cursors")
        );
        assert_eq!(env_value(env, "XCURSOR_SIZE").as_deref(), Some("24"));
        assert_eq!(env_value(env, "XCURSOR"), None);
    }

    #[test]
    fn theme_ids_are_plain_names() {
        for ok in [
            "capitaine-cursors",
            "Bibata-Modern-Ice",
            "breeze_cursors",
            "Vimix cursors",
            "phinger+",
        ] {
            assert!(valid_theme_id(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "x\ny", "a=b", "$(rm)", "a;b"] {
            assert!(!valid_theme_id(bad), "{bad:?}");
        }
    }
}
