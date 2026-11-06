//! Reading and writing every place a cursor choice lives (see the table in
//! [`super::engine`]). File edits are pure text transforms plus one write
//! each; commands go through a runner so tests never touch the session.
//! Everything here blocks: the page calls it off the GTK thread.

use super::engine::{Place, Reading, default_index_theme};
use crate::theme_files::{
    Roots, Runner, Step, edit_ini, env_value, ini_value, parse_gsettings_value, set_env_exports,
    write,
};

/// Every place's current theme and size, margo first (it decides what
/// "in use" means). `margo` is the `config.conf` reading
/// (`cursor_theme`, `cursor_size`).
pub(crate) fn read_all(
    roots: &Roots,
    margo: (Option<String>, Option<String>),
    run: Runner<'_>,
) -> Vec<Reading> {
    let size = |s: Option<String>| {
        s.and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|&n| n > 0)
    };
    let nonempty = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut out = vec![Reading {
        place: Place::Margo,
        theme: nonempty(margo.0),
        size: size(margo.1),
    }];
    let gs = |key: &str| {
        run("gsettings", &["get", "org.gnome.desktop.interface", key])
            .ok()
            .and_then(|o| parse_gsettings_value(&o))
    };
    out.push(Reading {
        place: Place::Gsettings,
        theme: gs("cursor-theme"),
        size: size(gs("cursor-size")),
    });
    for (place, version) in [(Place::Gtk3, "gtk-3.0"), (Place::Gtk4, "gtk-4.0")] {
        let text = std::fs::read_to_string(roots.gtk_ini(version)).unwrap_or_default();
        out.push(Reading {
            place,
            theme: nonempty(ini_value(&text, "Settings", "gtk-cursor-theme-name")),
            size: size(ini_value(&text, "Settings", "gtk-cursor-theme-size")),
        });
    }
    let index =
        std::fs::read_to_string(roots.x_default_dir().join("index.theme")).unwrap_or_default();
    out.push(Reading {
        place: Place::XDefault,
        theme: nonempty(ini_value(&index, "Icon Theme", "Inherits")),
        size: None,
    });
    let env = run("systemctl", &["--user", "show-environment"]).unwrap_or_default();
    out.push(Reading {
        place: Place::SessionEnv,
        theme: nonempty(env_value(&env, "XCURSOR_THEME")),
        size: size(env_value(&env, "XCURSOR_SIZE")),
    });
    out
}

/// Write `theme` / `size` to every file-based place: GTK 3 and 4
/// `settings.ini`, the X11 default theme, KDE's `kcminputrc` (if present)
/// and mdots' shell environment (if present).
pub(crate) fn apply_files(roots: &Roots, theme: &str, size: u32) -> Vec<(Place, Step)> {
    let mut out = Vec::new();
    let gtk = [
        ("gtk-cursor-theme-name", theme.to_string()),
        ("gtk-cursor-theme-size", size.to_string()),
    ];
    for (place, version) in [(Place::Gtk3, "gtk-3.0"), (Place::Gtk4, "gtk-4.0")] {
        out.push((
            place,
            edit_ini(&roots.gtk_ini(version), "Settings", &gtk, true),
        ));
    }

    let default_dir = roots.x_default_dir();
    let step = if default_dir.join("cursors").is_dir() {
        Step::Skipped("~/.icons/default is a cursor theme of its own; left as is".into())
    } else {
        match write(
            &default_dir.join("index.theme"),
            &default_index_theme(theme),
        ) {
            Ok(()) => Step::Done,
            Err(e) => Step::Failed(e),
        }
    };
    out.push((Place::XDefault, step));

    let kde = [
        ("cursorTheme", theme.to_string()),
        ("cursorSize", size.to_string()),
    ];
    if let Step::Failed(e) = edit_ini(&roots.kcminputrc(), "Mouse", &kde, false) {
        out.push((Place::Kde, Step::Failed(e)));
    }

    let env_file = roots.mdots_env();
    if let Ok(text) = std::fs::read_to_string(&env_file) {
        let size = size.to_string();
        let new = set_env_exports(&text, &[("XCURSOR_THEME", theme), ("XCURSOR_SIZE", &size)]);
        if new != text
            && let Err(e) = write(&env_file, &new)
        {
            out.push((Place::ShellEnv, Step::Failed(e)));
        }
    }
    out
}

/// Push `theme` / `size` to gsettings and to the systemd + D-Bus activation
/// environment (apps started from now on).
pub(crate) fn apply_commands(theme: &str, size: u32, run: Runner<'_>) -> Vec<(Place, Step)> {
    let size_s = size.to_string();
    let step = |r: Result<String, String>| match r {
        Ok(_) => Step::Done,
        Err(e) if e.ends_with("is not installed") => Step::Skipped(e),
        Err(e) => Step::Failed(e),
    };
    let gs = run(
        "gsettings",
        &["set", "org.gnome.desktop.interface", "cursor-theme", theme],
    )
    .and_then(|_| {
        run(
            "gsettings",
            &["set", "org.gnome.desktop.interface", "cursor-size", &size_s],
        )
    });
    let theme_kv = format!("XCURSOR_THEME={theme}");
    let size_kv = format!("XCURSOR_SIZE={size}");
    let env = run(
        "dbus-update-activation-environment",
        &["--systemd", &theme_kv, &size_kv],
    )
    .or_else(|_| {
        run(
            "systemctl",
            &["--user", "set-environment", &theme_kv, &size_kv],
        )
    });
    vec![(Place::Gsettings, step(gs)), (Place::SessionEnv, step(env))]
}

/// One line per place that didn't simply work, for the status label;
/// empty when everything was applied.
pub(crate) fn summarize(steps: &[(Place, Step)]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|(place, step)| match step {
            Step::Done => None,
            Step::Skipped(why) => Some(format!("{}: skipped ({why})", place.label())),
            Step::Failed(why) => Some(format!("{}: failed ({why})", place.label())),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;

    fn roots(tmp: &Path) -> Roots {
        Roots {
            home: tmp.join("home"),
            config_home: tmp.join("home/.config"),
        }
    }

    /// The owner's real `~/.config/gtk-4.0/settings.ini` cursor lines.
    const GTK4: &str = "[Settings]\ngtk-icon-theme-name=kora\ngtk-cursor-theme-name=capitaine-cursors\ngtk-cursor-theme-size=24\n";

    #[test]
    fn files_are_edited_in_place_or_created() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = roots(tmp.path());
        std::fs::create_dir_all(r.config_home.join("gtk-4.0")).expect("mkdir");
        std::fs::write(r.gtk_ini("gtk-4.0"), GTK4).expect("write");
        let steps = apply_files(&r, "Bibata-Modern-Ice", 32);
        assert!(steps.iter().all(|(_, s)| *s == Step::Done), "{steps:?}");
        let gtk4 = std::fs::read_to_string(r.gtk_ini("gtk-4.0")).expect("read");
        assert_eq!(
            gtk4,
            "[Settings]\ngtk-icon-theme-name=kora\ngtk-cursor-theme-name=Bibata-Modern-Ice\ngtk-cursor-theme-size=32\n"
        );
        let gtk3 = std::fs::read_to_string(r.gtk_ini("gtk-3.0")).expect("read");
        assert_eq!(
            gtk3,
            "[Settings]\ngtk-cursor-theme-name=Bibata-Modern-Ice\ngtk-cursor-theme-size=32\n"
        );
        let index = std::fs::read_to_string(r.x_default_dir().join("index.theme")).expect("index");
        assert_eq!(
            ini_value(&index, "Icon Theme", "Inherits").as_deref(),
            Some("Bibata-Modern-Ice")
        );
        assert!(!r.kcminputrc().exists(), "KDE file not created");
        assert!(!r.mdots_env().exists(), "mdots file not created");
    }

    #[test]
    fn a_real_default_theme_is_left_alone_and_optional_files_follow() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = roots(tmp.path());
        std::fs::create_dir_all(r.x_default_dir().join("cursors")).expect("mkdir");
        std::fs::create_dir_all(r.home.join(".mdots")).expect("mkdir");
        std::fs::write(
            r.mdots_env(),
            "# mdots\nexport GTK_THEME=\"x\"\nexport XCURSOR_THEME=\"capitaine-cursors\"\n",
        )
        .expect("write");
        std::fs::create_dir_all(&r.config_home).expect("mkdir");
        std::fs::write(r.kcminputrc(), "[Mouse]\ncursorTheme=breeze_cursors\n").expect("write");
        let steps = apply_files(&r, "Adwaita", 24);
        assert!(matches!(
            steps.iter().find(|(p, _)| *p == Place::XDefault),
            Some((_, Step::Skipped(_)))
        ));
        assert!(!r.x_default_dir().join("index.theme").exists());
        assert_eq!(
            std::fs::read_to_string(r.mdots_env()).expect("read"),
            "# mdots\nexport GTK_THEME=\"x\"\nexport XCURSOR_THEME=\"Adwaita\"\nexport XCURSOR_SIZE=\"24\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(r.kcminputrc()).expect("read"),
            "[Mouse]\ncursorTheme=Adwaita\ncursorSize=24\n"
        );
    }

    #[test]
    fn env_exports_replace_plain_and_exported_lines() {
        assert_eq!(
            set_env_exports(
                "XCURSOR_SIZE=24\nXCURSOR_SIZE_X=1\n",
                &[("XCURSOR_SIZE", "32")]
            ),
            "export XCURSOR_SIZE=\"32\"\nXCURSOR_SIZE_X=1\n"
        );
        assert_eq!(set_env_exports("", &[("A", "b")]), "export A=\"b\"\n");
    }

    #[test]
    fn commands_set_gsettings_and_the_activation_env() {
        let calls = RefCell::new(Vec::new());
        let run = |p: &str, a: &[&str]| -> Result<String, String> {
            calls.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        let steps = apply_commands("Bibata", 32, &run);
        assert!(steps.iter().all(|(_, s)| *s == Step::Done));
        assert_eq!(
            *calls.borrow(),
            [
                "gsettings set org.gnome.desktop.interface cursor-theme Bibata",
                "gsettings set org.gnome.desktop.interface cursor-size 32",
                "dbus-update-activation-environment --systemd XCURSOR_THEME=Bibata XCURSOR_SIZE=32",
            ]
        );
    }

    #[test]
    fn missing_tools_are_skips_and_the_env_falls_back_to_systemctl() {
        let calls = RefCell::new(Vec::new());
        let run = |p: &str, a: &[&str]| -> Result<String, String> {
            calls.borrow_mut().push(p.to_string());
            match p {
                "systemctl" => {
                    assert_eq!(a[..2], ["--user", "set-environment"]);
                    Ok(String::new())
                }
                _ => Err(format!("{p} is not installed")),
            }
        };
        let steps = apply_commands("Bibata", 32, &run);
        assert!(matches!(steps[0], (Place::Gsettings, Step::Skipped(_))));
        assert_eq!(steps[1], (Place::SessionEnv, Step::Done));
        assert_eq!(summarize(&steps).len(), 1);
    }

    #[test]
    fn read_all_collects_every_place() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = roots(tmp.path());
        std::fs::create_dir_all(r.config_home.join("gtk-4.0")).expect("mkdir");
        std::fs::write(r.gtk_ini("gtk-4.0"), GTK4).expect("write");
        // Captured from this machine (`gsettings get …`, `systemctl --user
        // show-environment`, trimmed).
        let run = |p: &str, a: &[&str]| -> Result<String, String> {
            match (p, a.last().copied()) {
                ("gsettings", Some("cursor-theme")) => Ok("'capitaine-cursors'\n".into()),
                ("gsettings", Some("cursor-size")) => Ok("24\n".into()),
                ("systemctl", _) => {
                    Ok("HOME=/home/u\nXCURSOR_SIZE=24\nXCURSOR_THEME=capitaine-cursors\n".into())
                }
                _ => Err("no".into()),
            }
        };
        let readings = read_all(&r, (None, Some("24".into())), &run);
        let by = |p: Place| {
            readings
                .iter()
                .find(|x| x.place == p)
                .cloned()
                .expect("place")
        };
        assert_eq!(by(Place::Margo).theme, None);
        assert_eq!(by(Place::Margo).size, Some(24));
        assert_eq!(
            by(Place::Gsettings).theme.as_deref(),
            Some("capitaine-cursors")
        );
        assert_eq!(by(Place::Gtk4).size, Some(24));
        assert_eq!(by(Place::Gtk3).theme, None, "no file");
        assert_eq!(by(Place::XDefault).theme, None);
        assert_eq!(
            by(Place::SessionEnv).theme.as_deref(),
            Some("capitaine-cursors")
        );
    }
}
