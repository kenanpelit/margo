//! Reading and writing every place a desktop keeps its icon theme name:
//!
//! | Place | Key | Reaches |
//! | --- | --- | --- |
//! | `gsettings org.gnome.desktop.interface` | `icon-theme` | GTK 4 / libadwaita, portals |
//! | `gtk-3.0/settings.ini`, `gtk-4.0/settings.ini` | `gtk-icon-theme-name` | GTK 3 / GTK 4 apps |
//! | `qt5ct/qt5ct.conf`, `qt6ct/qt6ct.conf` (if present) | `[Appearance] icon_theme` | Qt apps via qt5ct/qt6ct |
//! | `kdeglobals` (if present) | `[Icons] Theme` | KDE / Plasma-integrated Qt apps |
//! | `environment.d/*.conf` lines and the session env (if set) | `QT_ICON_THEME` | Qt apps reading the env |
//!
//! Files that don't exist (qt5ct, KDE) or don't set a key (`QT_ICON_THEME`)
//! are left alone: the picker follows the setup it finds.

use crate::theme_files::{
    Roots, Runner, Step, edit_ini, env_value, ini_value, parse_gsettings_value,
    replace_env_assignment, write,
};

/// A place an icon theme name lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconPlace {
    Gsettings,
    Gtk3,
    Gtk4,
    Qt5ct,
    Qt6ct,
    Kde,
    SessionEnv,
    EnvironmentD,
}

impl IconPlace {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Gsettings => "GNOME settings (gsettings)",
            Self::Gtk3 => "GTK 3 (settings.ini)",
            Self::Gtk4 => "GTK 4 (settings.ini)",
            Self::Qt5ct => "Qt 5 (qt5ct)",
            Self::Qt6ct => "Qt 6 (qt6ct)",
            Self::Kde => "KDE (kdeglobals)",
            Self::SessionEnv => "session environment (QT_ICON_THEME)",
            Self::EnvironmentD => "environment.d (QT_ICON_THEME)",
        }
    }
}

/// What one place says (`None`: it doesn't set an icon theme).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IconReading {
    pub place: IconPlace,
    pub theme: Option<String>,
}

fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Every place's icon theme, gsettings first (it is what GTK 4 uses).
pub(crate) fn read_all(roots: &Roots, run: Runner<'_>) -> Vec<IconReading> {
    let nonempty = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut out = vec![IconReading {
        place: IconPlace::Gsettings,
        theme: run(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "icon-theme"],
        )
        .ok()
        .and_then(|o| parse_gsettings_value(&o)),
    }];
    for (place, version) in [(IconPlace::Gtk3, "gtk-3.0"), (IconPlace::Gtk4, "gtk-4.0")] {
        out.push(IconReading {
            place,
            theme: nonempty(ini_value(
                &read_file(&roots.gtk_ini(version)),
                "Settings",
                "gtk-icon-theme-name",
            )),
        });
    }
    for (place, tool) in [(IconPlace::Qt5ct, "qt5ct"), (IconPlace::Qt6ct, "qt6ct")] {
        out.push(IconReading {
            place,
            theme: nonempty(ini_value(
                &read_file(&roots.qtct_conf(tool)),
                "Appearance",
                "icon_theme",
            )),
        });
    }
    out.push(IconReading {
        place: IconPlace::Kde,
        theme: nonempty(ini_value(&read_file(&roots.kdeglobals()), "Icons", "Theme")),
    });
    let env = run("systemctl", &["--user", "show-environment"]).unwrap_or_default();
    out.push(IconReading {
        place: IconPlace::SessionEnv,
        theme: nonempty(env_value(&env, "QT_ICON_THEME")),
    });
    out
}

/// The theme in use: the first place that names one.
pub(crate) fn in_use(readings: &[IconReading]) -> Option<String> {
    readings.iter().find_map(|r| r.theme.clone())
}

/// The places naming a theme other than `theme`.
pub(crate) fn out_of_step(readings: &[IconReading], theme: &str) -> Vec<IconPlace> {
    readings
        .iter()
        .filter(|r| r.theme.as_deref().is_some_and(|t| t != theme))
        .map(|r| r.place)
        .collect()
}

/// Write `theme` to every file-based place.
pub(crate) fn apply_files(roots: &Roots, theme: &str) -> Vec<(IconPlace, Step)> {
    let mut out = Vec::new();
    let gtk = [("gtk-icon-theme-name", theme.to_string())];
    for (place, version) in [(IconPlace::Gtk3, "gtk-3.0"), (IconPlace::Gtk4, "gtk-4.0")] {
        out.push((
            place,
            edit_ini(&roots.gtk_ini(version), "Settings", &gtk, true),
        ));
    }
    let qt = [("icon_theme", theme.to_string())];
    for (place, tool) in [(IconPlace::Qt5ct, "qt5ct"), (IconPlace::Qt6ct, "qt6ct")] {
        out.push((
            place,
            edit_ini(&roots.qtct_conf(tool), "Appearance", &qt, false),
        ));
    }
    let kde = [("Theme", theme.to_string())];
    out.push((
        IconPlace::Kde,
        edit_ini(&roots.kdeglobals(), "Icons", &kde, false),
    ));

    if let Ok(entries) = std::fs::read_dir(roots.environment_d()) {
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "conf"))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(new) = replace_env_assignment(&text, "QT_ICON_THEME", theme)
                && new != text
            {
                let step = match write(&file, &new) {
                    Ok(()) => Step::Done,
                    Err(e) => Step::Failed(e),
                };
                out.push((IconPlace::EnvironmentD, step));
            }
        }
    }
    out
}

/// Set gsettings, and `QT_ICON_THEME` in the systemd + D-Bus activation
/// environment when the session already sets it.
pub(crate) fn apply_commands(
    theme: &str,
    session_sets_qt: bool,
    run: Runner<'_>,
) -> Vec<(IconPlace, Step)> {
    let step = |r: Result<String, String>| match r {
        Ok(_) => Step::Done,
        Err(e) if e.ends_with("is not installed") => Step::Skipped(e),
        Err(e) => Step::Failed(e),
    };
    let mut out = vec![(
        IconPlace::Gsettings,
        step(run(
            "gsettings",
            &["set", "org.gnome.desktop.interface", "icon-theme", theme],
        )),
    )];
    if session_sets_qt {
        let kv = format!("QT_ICON_THEME={theme}");
        let r = run("dbus-update-activation-environment", &["--systemd", &kv])
            .or_else(|_| run("systemctl", &["--user", "set-environment", &kv]));
        out.push((IconPlace::SessionEnv, step(r)));
    }
    out
}

/// One line per place that was skipped for a reason worth showing or
/// failed. Files that simply don't exist (no qt5ct, no KDE) are not
/// reported.
pub(crate) fn summarize(steps: &[(IconPlace, Step)]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|(place, step)| match step {
            Step::Done => None,
            Step::Skipped(why) if why.ends_with("does not exist") => None,
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

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn files_follow_the_setup_that_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = roots(tmp.path());
        // Shaped like the owner's qt5ct.conf and environment.d/20-qt.conf.
        put(
            &r.qtct_conf("qt5ct"),
            "[Appearance]\ncolor_scheme_path=x\nicon_theme=kora\nstyle=kvantum\n",
        );
        put(
            &r.environment_d().join("20-qt.conf"),
            "QT_QPA_PLATFORMTHEME=kvantum\nQT_ICON_THEME=kora\n",
        );
        put(&r.environment_d().join("10-gtk.conf"), "GTK_THEME=x\n");
        let steps = apply_files(&r, "Papirus-Dark");
        assert!(summarize(&steps).is_empty(), "{steps:?}");
        let read = |p: &Path| std::fs::read_to_string(p).expect("read");
        assert_eq!(
            read(&r.qtct_conf("qt5ct")),
            "[Appearance]\ncolor_scheme_path=x\nicon_theme=Papirus-Dark\nstyle=kvantum\n"
        );
        assert!(
            !r.qtct_conf("qt6ct").exists(),
            "absent qt6ct is not created"
        );
        assert!(!r.kdeglobals().exists(), "absent kdeglobals is not created");
        assert_eq!(
            read(&r.gtk_ini("gtk-3.0")),
            "[Settings]\ngtk-icon-theme-name=Papirus-Dark\n"
        );
        assert_eq!(
            read(&r.environment_d().join("20-qt.conf")),
            "QT_QPA_PLATFORMTHEME=kvantum\nQT_ICON_THEME=Papirus-Dark\n"
        );
        assert_eq!(
            read(&r.environment_d().join("10-gtk.conf")),
            "GTK_THEME=x\n"
        );
    }

    #[test]
    fn readings_and_the_disagreement_list() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = roots(tmp.path());
        put(
            &r.gtk_ini("gtk-4.0"),
            "[Settings]\ngtk-icon-theme-name=kora\n",
        );
        put(&r.qtct_conf("qt6ct"), "[Appearance]\nicon_theme=breeze\n");
        // Captured from this machine (`gsettings get … icon-theme`).
        let run = |p: &str, _: &[&str]| -> Result<String, String> {
            match p {
                "gsettings" => Ok("'kora'\n".into()),
                "systemctl" => Ok("HOME=/h\nQT_ICON_THEME=kora\n".into()),
                _ => Err("no".into()),
            }
        };
        let readings = read_all(&r, &run);
        assert_eq!(in_use(&readings).as_deref(), Some("kora"));
        assert_eq!(out_of_step(&readings, "kora"), [IconPlace::Qt6ct]);
        assert!(out_of_step(&readings[..1], "kora").is_empty());
    }

    #[test]
    fn commands_touch_the_session_env_only_when_it_is_set() {
        let calls = RefCell::new(Vec::new());
        let run = |p: &str, a: &[&str]| -> Result<String, String> {
            calls.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        apply_commands("Papirus", false, &run);
        assert_eq!(
            *calls.borrow(),
            ["gsettings set org.gnome.desktop.interface icon-theme Papirus"]
        );
        calls.borrow_mut().clear();
        apply_commands("Papirus", true, &run);
        assert_eq!(
            calls.borrow()[1],
            "dbus-update-activation-environment --systemd QT_ICON_THEME=Papirus"
        );
    }
}
