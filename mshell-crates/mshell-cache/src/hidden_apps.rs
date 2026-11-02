use reactive_graph::prelude::{ReadUntracked, Update};
use reactive_stores::{ArcStore, Store};
use relm4::gtk::glib;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct HiddenAppsState {
    pub apps: Vec<String>, // desktop_ids
}

static HIDDEN_APPS: LazyLock<ArcStore<HiddenAppsState>> = LazyLock::new(|| {
    ArcStore::new(HiddenAppsState {
        apps: load_hidden_apps(),
    })
});

pub fn hidden_apps_store() -> ArcStore<HiddenAppsState> {
    HIDDEN_APPS.clone()
}

pub fn is_hidden(desktop_id: &str) -> bool {
    hidden_apps_store()
        .read_untracked()
        .apps
        .iter()
        .any(|a| a == desktop_id)
}

pub fn hide_app(desktop_id: String) {
    let store = hidden_apps_store();
    if store.read_untracked().apps.iter().any(|a| a == &desktop_id) {
        return;
    }
    store.update(|s| s.apps.push(desktop_id));
    persist();
}

pub fn unhide_app(desktop_id: String) {
    let store = hidden_apps_store();
    let len_before = store.read_untracked().apps.len();
    store.update(|s| s.apps.retain(|a| a != desktop_id.as_str()));
    let changed = store.read_untracked().apps.len() != len_before;
    if changed {
        persist();
    }
}

fn persist() {
    let apps = hidden_apps_store().read_untracked().apps.clone();
    if let Err(e) = save_hidden_apps(&apps) {
        eprintln!("Failed to save hidden apps: {e}");
    }
}

fn hidden_apps_path() -> PathBuf {
    glib::user_cache_dir()
        .join("mshell")
        .join("hidden_apps.txt")
}

fn load_hidden_apps() -> Vec<String> {
    load_hidden_apps_from(&hidden_apps_path())
}

fn load_hidden_apps_from(path: &Path) -> Vec<String> {
    // Read the whole (tiny) file at once: simpler than a line
    // iterator and sidesteps the `lines().filter_map(Result::ok)`
    // loop-forever hazard clippy flags.
    match fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn save_hidden_apps(apps: &[String]) -> std::io::Result<()> {
    save_hidden_apps_to(&hidden_apps_path(), apps)
}

fn save_hidden_apps_to(path: &Path, apps: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    for app in apps {
        writeln!(file, "{}", app)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk_and_creates_the_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mshell").join("hidden_apps.txt");
        let apps = vec!["org.gnome.Nautilus".to_string(), "steam".to_string()];
        save_hidden_apps_to(&path, &apps).unwrap();
        assert_eq!(load_hidden_apps_from(&path), apps);
        save_hidden_apps_to(&path, &[]).unwrap();
        assert!(load_hidden_apps_from(&path).is_empty());
    }

    #[test]
    fn load_trims_lines_and_skips_blanks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hidden_apps.txt");
        fs::write(&path, "  firefox  \r\n\n\t\nkitty\n").unwrap();
        assert_eq!(load_hidden_apps_from(&path), ["firefox", "kitty"]);
    }

    #[test]
    fn missing_file_means_nothing_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_hidden_apps_from(&tmp.path().join("nope.txt")).is_empty());
    }
}
