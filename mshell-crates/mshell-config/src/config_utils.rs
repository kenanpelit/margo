use figment::{
    Figment,
    providers::{Format, Serialized, Yaml},
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use tracing::info;

use notify::{Event, EventKind};
use reactive_stores::{ArcStore, Patch};

use crate::paths::{active_profile_cache_path, profile_path, profiles_dir};
use crate::schema::config::Config;
use reactive_graph::prelude::ReadUntracked;
use serde::Serialize;
use std::fs;
use std::path::Path;

pub(crate) fn read_active_profile_from_cache() -> Option<String> {
    let p = active_profile_cache_path();
    let s = fs::read_to_string(p).ok()?;
    let name = s.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

pub(crate) fn write_active_profile_to_cache(name: Option<&str>) {
    let p = active_profile_cache_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match name {
        Some(n) => {
            let _ = fs::write(p, n);
        }
        None => {
            let _ = fs::remove_file(p);
        }
    }
}

/// True once the setup wizard has applied at least once (the sentinel
/// file exists). Used to gate first-launch auto-open.
pub fn wizard_completed() -> bool {
    crate::paths::wizard_sentinel_path().exists()
}

/// Mark the setup wizard as completed so first-launch auto-open stops.
pub fn mark_wizard_completed() {
    let p = crate::paths::wizard_sentinel_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&p, b"1\n");
}

pub fn list_available_profiles() -> Vec<String> {
    let dir = profiles_dir();
    let mut out = Vec::new();

    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    out
}

pub(crate) fn load_effective_config(
    active_profile: Option<&str>,
) -> Result<Config, figment::Error> {
    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    if let Some(name) = active_profile {
        figment = figment.merge(Yaml::file(profile_path(name)));
    }

    let mut config = figment.extract::<Config>()?;
    // Plugin widgets are derived from the plugin manager's own files, not the
    // profile — fold the enabled ones in on every load.
    crate::plugin_bridge::resync_plugin_widgets(&mut config);
    Ok(config)
}

pub(crate) fn watch_config_loop(
    rx: mpsc::Receiver<notify::Result<Event>>,
    active_profile: ArcStore<Option<String>>,
    available_profiles: ArcStore<Vec<String>>,
    config: ArcStore<Config>,
) {
    let mut pending = false;
    let mut last_event_at = Instant::now();
    const DEBOUNCE_MS: u64 = 200;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) => {
                if is_relevant_config_event(&event) {
                    pending = true;
                    last_event_at = Instant::now();
                }
            }
            Ok(Err(e)) => eprintln!("config: watch error: {e}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if pending && last_event_at.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
            pending = false;
            let active = active_profile.read_untracked();
            let active = active.as_deref();
            match load_effective_config(active) {
                Ok(new_cfg) => {
                    config.patch(new_cfg);
                    available_profiles.patch(list_available_profiles());
                    info!("New config loaded in watch loop");
                }
                Err(e) => eprintln!("config: reload failed (keeping last-good): {e}"),
            }
        }
    }
}

pub(crate) fn is_relevant_config_event(event: &Event) -> bool {
    match event.kind {
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
        _ => return false,
    }
    event.paths.iter().any(|path| {
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && (name.ends_with('~')
                || name.ends_with(".swp")
                || name.ends_with(".swx")
                || name.ends_with(".tmp")
                || name.starts_with(".#")
                || name.starts_with('#'))
        {
            return false;
        }
        path.extension().and_then(|s| s.to_str()) == Some("yaml")
    })
}

pub(crate) fn persist_config_layer<T: Serialize>(
    value: &T,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let yaml = serde_yaml::to_string(value)?;

    // Atomic write via the symlink-preserving helper so dcli /
    // stow / chezmoi users keep their `~/.config/margo/mshell/
    // profiles/default.yaml -> ~/.cachy/modules/.../default.yaml`
    // links intact. The plain `write(tmp) + rename(tmp, path)`
    // pattern clobbered the symlink with a regular file every
    // time Settings was saved.
    crate::atomic_write::atomic_write(path, yaml.as_bytes())?;

    Ok(())
}

// Starter profiles baked into the binary (also installed to
// /usr/share/margo/mshell/profiles/ for reference). The setup wizard seeds
// the chosen one into the user's profiles dir on first run — baking them in
// means it works whether or not the package data is present (e.g. a dev
// `cargo install` of just the binary).
const BUNDLED_DEFAULT: &str = include_str!("../../../mshell/examples/profiles/default.yaml");
const BUNDLED_NOVA: &str = include_str!("../../../mshell/examples/profiles/Nova.yaml");

/// Baked YAML for a bundled starter profile, if the name is one we ship.
pub fn bundled_profile_yaml(name: &str) -> Option<&'static str> {
    match name {
        "default" => Some(BUNDLED_DEFAULT),
        "Nova" => Some(BUNDLED_NOVA),
        _ => None,
    }
}

/// Write a bundled starter profile into the user's profiles dir, but only if
/// no profile of that name exists yet — so re-running the wizard never
/// clobbers a profile the user has customised. Returns whether a file for
/// `name` exists afterwards (freshly seeded or already there).
pub fn seed_bundled_profile(name: &str) -> bool {
    let Some(yaml) = bundled_profile_yaml(name) else {
        return false;
    };
    let path = profile_path(name);
    if path.exists() {
        return true; // keep the user's existing profile untouched
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, yaml).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The profile examples shipped with the package (Default / Nova) must
    /// merge cleanly over the compiled-in defaults and extract to a full
    /// `Config` — a broken example would ship a broken first-run experience.
    #[test]
    fn shipped_profiles_parse() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mshell/examples/profiles");
        for name in ["default.yaml", "Nova.yaml"] {
            let path = base.join(name);
            let cfg = Figment::from(Serialized::defaults(Config::default()))
                .merge(Yaml::file(&path))
                .extract::<Config>();
            assert!(
                cfg.is_ok(),
                "shipped profile {name} failed to parse: {:?}",
                cfg.err()
            );
        }
    }
}
