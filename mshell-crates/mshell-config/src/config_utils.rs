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
use serde_yaml::{Mapping, Value};
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

/// Run the migration pre-pass on a profile file: if it exists and migrating it
/// changes anything, write the upgraded YAML back atomically (symlink-safe).
/// A missing file or a clean parse/migrate failure is a no-op — figment then
/// falls back to defaults / last-good as it already does.
fn migrate_profile_file(path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    match crate::migration::migrate_yaml(&raw) {
        Ok(m) if m.changed => {
            if let Err(e) = crate::atomic_write::atomic_write(path, m.yaml.as_bytes()) {
                tracing::error!(error = %e, "config: profile migration write-back failed");
            } else {
                info!(
                    "Migrated profile {} from config_version {} to {}",
                    path.display(),
                    m.from,
                    crate::migration::CONFIG_VERSION
                );
            }
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "config: profile migration parse failed (leaving as-is)")
        }
    }
}

pub(crate) fn load_effective_config(
    active_profile: Option<&str>,
) -> Result<Config, figment::Error> {
    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    if let Some(name) = active_profile {
        let path = profile_path(name);
        // Migration pre-pass: bring an older on-disk profile up to the current
        // format before figment reads it, writing the upgraded YAML back once
        // (idempotent — a current profile is left untouched). See migration.rs.
        migrate_profile_file(&path);
        figment = figment.merge(Yaml::file(path));
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
        // When idle, block indefinitely for the next event; only poll on
        // the debounce deadline while a change is pending. The old
        // unconditional `recv_timeout(50ms)` was a permanent 20 Hz wakeup
        // thread for a file that changes a few times a day.
        let recv = if pending {
            let wait = Duration::from_millis(DEBOUNCE_MS).saturating_sub(last_event_at.elapsed());
            rx.recv_timeout(wait)
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match recv {
            Ok(Ok(event)) => {
                if is_relevant_config_event(&event) {
                    pending = true;
                    last_event_at = Instant::now();
                }
            }
            Ok(Err(e)) => tracing::error!(error = %e, "config: watch error"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if pending && last_event_at.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
            pending = false;
            let active = active_profile.read_untracked();
            let active = active.as_deref();
            match load_effective_config(active) {
                Ok(new_cfg) => {
                    // The profile-list store is cheap and may have changed even
                    // when the effective config did not (a profile file added /
                    // removed), so always refresh it.
                    available_profiles.patch(list_available_profiles());
                    // No-op guard: the notify watcher also fires on mshell's
                    // OWN profile writes (`persist_config_layer`'s atomic
                    // rename) — at login that bounced two self-writes back as
                    // full reloads. When the reloaded config is identical to
                    // what's already live, skip `config.patch` so we don't
                    // re-run every config effect across both bars and all menus
                    // for no change. A genuine external edit differs and still
                    // patches.
                    let unchanged = new_cfg == *config.read_untracked();
                    if unchanged {
                        continue;
                    }
                    config.patch(new_cfg);
                    info!("New config loaded in watch loop");
                }
                Err(e) => {
                    tracing::error!(error = %e, "config: reload failed (keeping last-good)")
                }
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

pub(crate) fn persist_config_layer(
    value: &Config,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Persist only what the user actually changed from `Config::default()`.
    // `load_effective_config` layers the profile *over* `Config::default()`
    // via figment, so any field omitted here transparently picks up the
    // current code default on load. Writing the full resolved config (the
    // old behaviour) baked every `#[serde(default = …)]` field's value into
    // the profile permanently: serde then saw the key present and never
    // re-applied a later code-default change, silently freezing the field
    // at whatever the default was the first time the profile was written.
    // Diffing kills that shadowing for every field at once — a field is
    // only persisted once it actually diverges from the default.
    let full = serde_yaml::to_value(value)?;
    let base = serde_yaml::to_value(Config::default())?;
    let diff = diff_against_default(&full, &base).unwrap_or_else(|| Value::Mapping(Mapping::new()));

    // Pin the current format version at the top of the serialized profile;
    // the Config struct deliberately omits the meta key (migration.rs), so we
    // stamp it on the way out.
    let yaml = crate::migration::stamp_version(&serde_yaml::to_string(&diff)?);

    // Atomic write via the symlink-preserving helper so dcli /
    // stow / chezmoi users keep their `~/.config/margo/mshell/
    // profiles/default.yaml -> ~/.cachy/modules/.../default.yaml`
    // links intact. The plain `write(tmp) + rename(tmp, path)`
    // pattern clobbered the symlink with a regular file every
    // time Settings was saved.
    crate::atomic_write::atomic_write(path, yaml.as_bytes())?;

    Ok(())
}

/// Recursively strip from `value` everything that equals the matching entry
/// in `base` (a serialized `Config::default()`), keeping only the leaves the
/// user changed. Mappings recurse key-by-key; sequences and scalars are
/// atomic — kept whole when they differ, because figment *replaces* arrays
/// rather than element-merging them. Returns `None` when nothing differs.
fn diff_against_default(value: &Value, base: &Value) -> Option<Value> {
    match (value, base) {
        (Value::Mapping(v), Value::Mapping(b)) => {
            let mut out = Mapping::new();
            for (k, vv) in v {
                match b.get(k) {
                    Some(bv) => {
                        if let Some(d) = diff_against_default(vv, bv) {
                            out.insert(k.clone(), d);
                        }
                    }
                    // Key absent from the default tree — keep it verbatim
                    // (shouldn't happen for a same-shaped Config, but safe).
                    None => {
                        out.insert(k.clone(), vv.clone());
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Mapping(out))
            }
        }
        _ => {
            if value == base {
                None
            } else {
                Some(value.clone())
            }
        }
    }
}

// Starter profiles baked into the binary (also installed to
// /usr/share/margo/mshell/profiles/ for reference). The setup wizard seeds
// the chosen one into the user's profiles dir on first run — baking them in
// means it works whether or not the package data is present (e.g. a dev
// `cargo install` of just the binary).
const BUNDLED_DEFAULT: &str = include_str!("../../../mshell/examples/profiles/default.yaml");
const BUNDLED_MARGO: &str = include_str!("../../../mshell/examples/profiles/margo.yaml");

/// Baked YAML for a bundled starter profile, if the name is one we ship.
pub fn bundled_profile_yaml(name: &str) -> Option<&'static str> {
    match name {
        "default" => Some(BUNDLED_DEFAULT),
        "margo" => Some(BUNDLED_MARGO),
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

    /// The profile examples shipped with the package (default / margo) must
    /// merge cleanly over the compiled-in defaults and extract to a full
    /// `Config` — a broken example would ship a broken first-run experience.
    #[test]
    fn shipped_profiles_parse() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mshell/examples/profiles");
        for name in ["default.yaml", "margo.yaml"] {
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

    /// The diff-persist path must reproduce a customised config exactly when
    /// figment layers the sparse profile back over the compiled-in defaults
    /// (the same merge `load_effective_config` does). If any changed field
    /// were dropped or any default field mis-restored, equality fails.
    #[test]
    fn diff_persist_round_trips_a_customised_config() {
        let mut cfg = Config::default();
        cfg.osd.width += 123;
        cfg.osd.distance += 7;
        cfg.osd.border_width += 2;

        let full = serde_yaml::to_value(&cfg).unwrap();
        let base = serde_yaml::to_value(Config::default()).unwrap();
        let diff =
            diff_against_default(&full, &base).expect("a customised config diffs to something");
        let yaml = serde_yaml::to_string(&diff).unwrap();

        let reloaded = Figment::from(Serialized::defaults(Config::default()))
            .merge(Yaml::string(&yaml))
            .extract::<Config>()
            .expect("sparse profile re-extracts");

        assert_eq!(
            reloaded, cfg,
            "diff + figment-merge must reproduce the config exactly"
        );
    }

    /// The whole point of diffing: a field left at its default is omitted, so
    /// a later change to that code default is never shadowed by a stale baked
    /// copy. An all-default config diffs to nothing; a one-field change diffs
    /// to only that branch.
    #[test]
    fn diff_drops_fields_left_at_default() {
        let base = serde_yaml::to_value(Config::default()).unwrap();
        assert!(
            diff_against_default(&base, &base).is_none(),
            "an all-default config persists nothing"
        );

        let mut cfg = Config::default();
        cfg.osd.width += 1;
        let full = serde_yaml::to_value(&cfg).unwrap();
        let diff = diff_against_default(&full, &base).unwrap();
        let Value::Mapping(map) = &diff else {
            panic!("config diffs to a mapping");
        };
        assert!(map.contains_key("osd"), "the changed branch is present");
        assert!(
            !map.contains_key("launcher"),
            "an untouched branch is omitted"
        );
    }

    /// Shell-side first-login bootstrap: an empty profiles dir gets the chosen
    /// bundled profile seeded (and it parses as a `Config`); a profile the user
    /// already has is never clobbered; an unknown name seeds nothing. Mutates
    /// `$HOME` (paths.rs derives everything from it), so it runs under a lock
    /// and restores the env — no other test in this crate reads `$HOME`.
    #[test]
    fn seed_bundled_profile_creates_default_and_preserves_existing() {
        use std::sync::Mutex;
        static HOME_LOCK: Mutex<()> = Mutex::new(());
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("margo-seed-test-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let prev = std::env::var_os("HOME");
        // SAFETY: single-threaded within the HOME_LOCK; restored below.
        unsafe { std::env::set_var("HOME", &home) };

        // Empty dir → seeding creates the profile, and it parses as Config.
        let path = crate::paths::profile_path("default");
        assert!(!path.exists(), "starts absent");
        assert!(
            seed_bundled_profile("default"),
            "default is a bundled profile"
        );
        assert!(path.exists(), "default.yaml seeded");
        serde_yaml::from_str::<Config>(&std::fs::read_to_string(&path).unwrap())
            .expect("seeded profile parses as Config");

        // Re-seeding must NOT clobber a profile the user has customised.
        std::fs::write(&path, "general:\n  panel_scale: 1.25\n").unwrap();
        assert!(
            seed_bundled_profile("default"),
            "returns true (already present)"
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("panel_scale"),
            "existing profile preserved, not overwritten"
        );

        // An unknown profile name seeds nothing.
        assert!(!seed_bundled_profile("no-such-profile"));
        assert!(!crate::paths::profile_path("no-such-profile").exists());

        // Restore the environment for any later test.
        match prev {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
