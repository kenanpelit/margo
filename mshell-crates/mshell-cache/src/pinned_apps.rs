use reactive_graph::prelude::{ReadUntracked, Update};
use reactive_stores::{ArcStore, Patch, Store};
use relm4::gtk::glib;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
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
    let path = pinned_apps_path();
    // Read the whole (tiny) file at once: simpler than a line
    // iterator and sidesteps the `lines().filter_map(Result::ok)`
    // loop-forever hazard clippy flags.
    match fs::read_to_string(&path) {
        Ok(contents) => contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(PinnedApp::from_line)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn save_pinned_apps(apps: &[PinnedApp]) -> std::io::Result<()> {
    let path = pinned_apps_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&path)?;
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
