use reactive_graph::prelude::{ReadUntracked, Update};
use reactive_stores::{ArcStore, Patch, Store};
use relm4::gtk::glib;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct PinnedAppsState {
    pub apps: Vec<PinnedApp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Patch)]
pub struct PinnedApp {
    /// `.desktop` id (or raw launch key) used to start the app when no
    /// window is running.
    pub desktop_id: String,
    /// Wayland app-id the compositor reports for this app's windows
    /// (`mctl clients` APP-ID); matched verbatim against running clients
    /// to decide focus-vs-launch.
    pub app_id: String,
}

static PINNED_APPS: LazyLock<ArcStore<PinnedAppsState>> = LazyLock::new(|| {
    ArcStore::new(PinnedAppsState {
        apps: load_pinned_apps(),
    })
});

pub fn pinned_apps_store() -> ArcStore<PinnedAppsState> {
    PINNED_APPS.clone()
}

pub fn pin_app(app: PinnedApp) {
    let store = pinned_apps_store();
    if store
        .read_untracked()
        .apps
        .iter()
        .any(|a| a.desktop_id == app.desktop_id)
    {
        return;
    }
    store.update(|s| s.apps.push(app));
    persist();
}

pub fn unpin_app(desktop_id: &str) {
    let store = pinned_apps_store();
    let len_before = store.read_untracked().apps.len();
    store.update(|s| s.apps.retain(|a| a.desktop_id != desktop_id));
    let changed = store.read_untracked().apps.len() != len_before;
    if changed {
        persist();
    }
}

fn persist() {
    let apps = pinned_apps_store().read_untracked().apps.clone();
    if let Err(e) = save_pinned_apps(&apps) {
        eprintln!("Failed to save pinned apps: {e}");
    }
}

fn pinned_apps_path() -> PathBuf {
    glib::user_cache_dir()
        .join("mshell")
        .join("pinned_apps.txt")
}

fn load_pinned_apps() -> Vec<PinnedApp> {
    load_pinned_apps_from(&pinned_apps_path())
}

fn load_pinned_apps_from(path: &Path) -> Vec<PinnedApp> {
    // Read the whole (tiny) file at once: simpler than a line
    // iterator and sidesteps the `lines().filter_map(Result::ok)`
    // loop-forever hazard clippy flags.
    match fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(PinnedApp::from_line)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn save_pinned_apps(apps: &[PinnedApp]) -> std::io::Result<()> {
    save_pinned_apps_to(&pinned_apps_path(), apps)
}

fn save_pinned_apps_to(path: &Path, apps: &[PinnedApp]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    for app in apps {
        writeln!(file, "{}", app.to_line())?;
    }
    Ok(())
}

impl PinnedApp {
    fn to_line(&self) -> String {
        format!("{}\t{}", self.desktop_id, self.app_id)
    }

    fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.splitn(2, '\t');
        let desktop_id = parts.next()?.trim().to_string();
        let app_id = parts.next()?.trim().to_string();
        if desktop_id.is_empty() || app_id.is_empty() {
            return None;
        }
        Some(Self { desktop_id, app_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(desktop_id: &str, app_id: &str) -> PinnedApp {
        PinnedApp {
            desktop_id: desktop_id.to_string(),
            app_id: app_id.to_string(),
        }
    }

    #[test]
    fn line_format_round_trips() {
        let a = app("org.mozilla.firefox.desktop", "firefox");
        assert_eq!(a.to_line(), "org.mozilla.firefox.desktop\tfirefox");
        assert_eq!(PinnedApp::from_line(&a.to_line()), Some(a));
    }

    #[test]
    fn from_line_rejects_incomplete_lines() {
        assert_eq!(PinnedApp::from_line("firefox"), None);
        assert_eq!(PinnedApp::from_line("firefox\t"), None);
        assert_eq!(PinnedApp::from_line("\tfirefox"), None);
        assert_eq!(PinnedApp::from_line("  \t  "), None);
    }

    #[test]
    fn from_line_trims_fields_and_keeps_extra_tabs_in_app_id() {
        assert_eq!(
            PinnedApp::from_line(" kitty.desktop \t kitty \r"),
            Some(app("kitty.desktop", "kitty"))
        );
        assert_eq!(
            PinnedApp::from_line("a.desktop\tb\tc"),
            Some(app("a.desktop", "b\tc"))
        );
    }

    #[test]
    fn disk_round_trip_skips_garbage_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mshell").join("pinned_apps.txt");
        let apps = vec![app("a.desktop", "a"), app("b.desktop", "org.b")];
        save_pinned_apps_to(&path, &apps).unwrap();
        assert_eq!(load_pinned_apps_from(&path), apps);

        fs::write(
            &path,
            "a.desktop\ta\n\nlegacy-line-without-tab\nb.desktop\tb\n",
        )
        .unwrap();
        assert_eq!(
            load_pinned_apps_from(&path),
            vec![app("a.desktop", "a"), app("b.desktop", "b")]
        );
        assert!(load_pinned_apps_from(&tmp.path().join("missing")).is_empty());
    }
}
