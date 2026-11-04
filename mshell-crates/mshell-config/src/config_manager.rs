use std::{
    fs,
    sync::{OnceLock, mpsc},
    thread,
};

use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use reactive_graph::prelude::ReadUntracked;
use reactive_stores::{ArcStore, Patch};
use tracing::{error, info};

use crate::config_utils::*;
use crate::errors::{ProfileCreateError, ProfileDeleteError};
use crate::paths::{DEFAULT_PROFILE_NAME, default_config_path, profile_path, profiles_dir};
use crate::schema::config::Config;

pub struct ConfigManager {
    active_profile: ArcStore<Option<String>>,
    available_profiles: ArcStore<Vec<String>>,
    config: ArcStore<Config>,
    degraded: DegradedFlag,
    persister: crate::persist::Persister,
}

static CONFIG_MANAGER: OnceLock<ConfigManager> = OnceLock::new();

pub fn config_manager() -> &'static ConfigManager {
    CONFIG_MANAGER.get_or_init(ConfigManager::new)
}

impl ConfigManager {
    fn new() -> Self {
        info!("Creating new ConfigManager");
        // Plugin secrets are read off-thread (see plugin_bridge); once they
        // are in, re-derive the plugin widgets with the real values.
        crate::plugin_bridge::on_secrets_loaded(|| config_manager().refresh_plugin_widgets());
        let mut active_profile = read_active_profile_from_cache();
        // Drop a stale cached name whose profile file no longer exists. The
        // cache lives in ~/.cache/mshell, so it survives a `~/.config/margo`
        // wipe — without this, a leftover name (e.g. "Nova" from an earlier
        // setup) pins a ghost profile and mshell regenerates an empty YAML
        // under it instead of falling back to the default. Clear it so a fresh
        // config dir really starts fresh.
        if let Some(name) = active_profile.as_deref()
            && !profile_path(name).exists()
        {
            info!(
                profile = name,
                "cached active profile has no file; clearing stale cache"
            );
            write_active_profile_to_cache(None);
            active_profile = None;
        }
        // No cached name (e.g. ~/.cache wiped) but a default profile on disk:
        // load it. Starting from bare defaults here would make the first
        // toggle write `defaults + toggle` over the user's default.yaml.
        if active_profile.is_none() && default_config_path().exists() {
            active_profile = Some(DEFAULT_PROFILE_NAME.to_string());
        }
        let (config, degraded) = match load_effective_config(active_profile.as_deref()) {
            Ok(config) => (config, false),
            Err(e) => {
                error!(error = %e, "config: failed to load the profile; running on defaults, not saving changes");
                if let Some(name) = active_profile.as_deref() {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    match quarantine_broken_profile(&profile_path(name), ts) {
                        Ok(copy) => {
                            info!(copy = %copy.display(), "config: kept a copy of the broken profile")
                        }
                        Err(e) => {
                            error!(error = %e, "config: could not copy the broken profile aside")
                        }
                    }
                }
                (Config::default(), true)
            }
        };
        let config = ArcStore::new(config);
        let degraded = DegradedFlag::new(degraded);
        let active_profile = ArcStore::new(active_profile);

        let available_profiles = ArcStore::new(list_available_profiles());

        Self {
            active_profile,
            available_profiles,
            config,
            degraded,
            persister: crate::persist::Persister::new(crate::persist::PERSIST_DEBOUNCE),
        }
    }

    /// True while the live config does not reflect the active profile file
    /// (it failed to load); changes then apply in memory but aren't saved.
    pub fn is_degraded(&self) -> bool {
        self.degraded.get()
    }

    /// Reactive view of [`is_degraded`](Self::is_degraded).
    pub fn config_degraded(&self) -> ArcStore<bool> {
        self.degraded.store()
    }

    pub fn config(&self) -> ArcStore<Config> {
        self.config.clone()
    }

    pub fn active_profile(&self) -> ArcStore<Option<String>> {
        self.active_profile.clone()
    }

    pub fn available_profiles(&self) -> ArcStore<Vec<String>> {
        self.available_profiles.clone()
    }

    /// Sets active profile name (without ".yaml"), persists it, reloads immediately
    pub fn set_active_profile(&self, name: Option<String>) {
        // No-op guard: re-selecting the already-active profile must not write
        // the cache + run a full `reload_config`. The General settings profile
        // DropDown fires `connect_selected_notify` on its own programmatic
        // `set_selected` during init (and again from the
        // Active/AvailableProfiles effects) — at login that re-selected the
        // same "active" profile several times, and each call reparsed the
        // profile and re-ran every config effect across both bars and all
        // menus (the "Config reloaded" burst in the startup window). Skip when
        // the name is unchanged.
        if self.active_profile.read_untracked().as_deref() == name.as_deref() {
            return;
        }

        self.active_profile.patch(name.clone());

        write_active_profile_to_cache(name.as_deref());

        self.reload_config();
    }

    pub fn create_profile(&self, name: &str) -> Result<(), ProfileCreateError> {
        if !crate::paths::is_safe_profile_name(name) {
            return Err(ProfileCreateError::InvalidName);
        }
        let path = profile_path(name);
        if path.exists() {
            return Err(ProfileCreateError::AlreadyExists);
        }

        if let Err(e) = persist_config_layer(&Config::default(), &path) {
            return Err(ProfileCreateError::Io(e));
        }

        self.available_profiles.patch(list_available_profiles());
        self.set_active_profile(Some(name.to_string()));
        Ok(())
    }

    /// Snapshot the CURRENT effective config into profile `name` and make
    /// it active. Unlike [`create_profile`](Self::create_profile) (which
    /// seeds a fresh `Config::default()`), this captures the live
    /// configuration — the "keep my current setup as a named profile"
    /// path. Overwrites `name` if it already exists (re-snapshot).
    pub fn snapshot_active_as(&self, name: &str) -> Result<(), ProfileCreateError> {
        if !crate::paths::is_safe_profile_name(name) {
            return Err(ProfileCreateError::InvalidName);
        }
        let path = profile_path(name);
        let current = self.config.read_untracked().clone();
        if let Err(e) = persist_config_layer(&current, &path) {
            return Err(ProfileCreateError::Io(e));
        }
        self.available_profiles.patch(list_available_profiles());
        self.set_active_profile(Some(name.to_string()));
        Ok(())
    }

    pub fn delete_profile(&self, name: &str) -> Result<(), ProfileDeleteError> {
        // Defense in depth: every current caller already passes a name
        // sourced from `list_available_profiles()` (real files on disk),
        // but this is the one operation here that removes a file — worth
        // never trusting `name` blindly regardless of caller discipline.
        if !crate::paths::is_safe_profile_name(name) {
            return Err(ProfileDeleteError::NotFound);
        }
        let path = profile_path(name);
        if !path.exists() {
            return Err(ProfileDeleteError::NotFound);
        }

        // If deleting the active profile, switch away first
        let active = self.active_profile.read_untracked();
        if active.as_deref() == Some(name) {
            drop(active); // release borrow before mutating

            let available = list_available_profiles();
            let fallback = available.iter().find(|p| p.as_str() != name).cloned();
            self.set_active_profile(fallback);
        }

        fs::remove_file(&path).map_err(ProfileDeleteError::Io)?;
        self.available_profiles.patch(list_available_profiles());
        Ok(())
    }

    /// Apply a mutation to the current effective config, persist the change to
    /// the appropriate layer (active profile if one is set, otherwise global
    /// config), then reload so the full effective config reflects it.
    #[track_caller]
    pub fn update_config<F>(&self, f: F)
    where
        F: FnOnce(&mut Config),
    {
        let caller = std::panic::Location::caller();
        // Snapshot current effective config and apply the mutation.
        let mut updated = self.config.read_untracked().clone();
        f(&mut updated);
        // Plugin widgets are a derived layer (re-added on load); never write
        // them into the user's profile.
        crate::plugin_bridge::strip_plugin_widgets(&mut updated);

        // No-op guard: if the mutation left the effective config unchanged,
        // skip the persist + reload (and its "Config reloaded" log). Eagerly
        // built per-monitor widgets re-persist unchanged values at startup,
        // which otherwise triggers a YAML write + a full reparse each time —
        // the ~45 "Config reloaded" burst at login.
        let mut baseline = self.config.read_untracked().clone();
        crate::plugin_bridge::strip_plugin_widgets(&mut baseline);
        if updated == baseline {
            return;
        }

        // Who actually wrote (after the no-op guard). A write that recurs on
        // every login points at a non-idempotent migration/normalisation worth
        // fixing — `mshellctl log level debug` surfaces the call site.
        tracing::debug!(caller = %caller, "update_config: persisting a real change");

        // Determine which file owns this layer.
        let active = self.active_profile.read_untracked();
        let layer_path = match active.as_deref() {
            Some(name) => profile_path(name),
            None => default_config_path(),
        };

        // The live config didn't come from this file (it failed to load):
        // persisting would replace it with `defaults + this change`. Apply the
        // change for this session only; the next successful load lifts this.
        if self.degraded.get() {
            tracing::warn!(
                caller = %caller,
                path = %layer_path.display(),
                "update_config: config degraded, change applied in memory but not saved"
            );
            self.degraded.notify_refused(&layer_path);
            drop(active);
            crate::plugin_bridge::resync_plugin_widgets(&mut updated);
            self.config.patch(updated);
            return;
        }

        tracing::debug!(
            active = ?active.as_deref(),
            path = %layer_path.display(),
            "update_config: persisting layer"
        );

        // Apply now, persist later: the store is patched immediately (the
        // UI follows at once) and the profile write is queued on the
        // persist thread with a short trailing debounce, so a slider drag is
        // one write of the final value instead of a serialise + two fsyncs
        // on the GTK thread per step. `flush()` (shutdown) writes anything
        // still queued. Only the plugin-widget layer stripped above needs
        // re-adding before the patch (a fresh load does the same).
        //
        // The `None` (default-layer) branch keeps its bootstrap behaviour:
        // write the default profile now, then make it active.
        if active.is_some() {
            drop(active);
            self.persister.schedule(layer_path, updated.clone());
            crate::plugin_bridge::resync_plugin_widgets(&mut updated);
            self.config.patch(updated);
        } else {
            drop(active);
            if let Err(e) = persist_config_layer(&updated, &layer_path) {
                tracing::error!(error = %e, path = %layer_path.display(), "update_config: persist_config_layer failed");
                return;
            }
            self.set_active_profile(Some(DEFAULT_PROFILE_NAME.to_string()));
        }
    }

    /// Write any queued profile change now (shutdown; before reading the
    /// profile file back).
    pub fn flush(&self) {
        self.persister.flush();
    }

    /// Re-derive the plugin-widget layer on the live config (after plugin
    /// secrets arrive from the keyring). No file I/O beyond the plugin
    /// manifests; patches only on a real change.
    pub fn refresh_plugin_widgets(&self) {
        let mut cfg = self.config.read_untracked().clone();
        crate::plugin_bridge::resync_plugin_widgets(&mut cfg);
        if cfg != *self.config.read_untracked() {
            self.config.patch(cfg);
        }
    }

    pub fn reload_config(&self) {
        // A queued change must be on disk before the file is read back.
        self.persister.flush();
        let active = self.active_profile.read_untracked();
        let active = active.as_deref();

        match load_effective_config(active) {
            Ok(new_cfg) => {
                self.config.patch(new_cfg);
                self.degraded.set(false);
                info!("Config reloaded");
            }
            Err(e) => {
                error!(error = %e, "config: reload failed (keeping last-good)");
                // Last-good may belong to another profile (a failed switch)
                // or predate the user's hand-edit — don't persist over it.
                self.degraded.set(true);
            }
        }
    }

    pub fn watch_config(&self) {
        let active_profile = self.active_profile.clone();
        let available_profiles = self.available_profiles.clone();
        let config = self.config.clone();
        let degraded = self.degraded.clone();
        let persister = self.persister.clone();

        thread::spawn(move || {
            let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
            let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(
                tx,
                NotifyConfig::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    // e.g. the host's inotify instance limit is already
                    // exhausted. Losing live-reload is far better than
                    // taking the whole shell down over it.
                    error!(error = %e, "config: failed to create watcher, live-reload disabled");
                    return;
                }
            };

            let prof_dir = profiles_dir();

            if let Err(e) = fs::create_dir_all(&prof_dir) {
                error!("failed to create profiles dir: {e}");
                return;
            }

            let _ = watcher.watch(&prof_dir, RecursiveMode::NonRecursive);

            watch_config_loop(
                rx,
                active_profile,
                available_profiles,
                config,
                degraded,
                persister,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_home::TempHome;

    fn write_profile(name: &str, body: &str) -> std::path::PathBuf {
        let path = profile_path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        write_active_profile_to_cache(Some(name));
        path
    }

    fn broken_copies(name: &str) -> Vec<std::path::PathBuf> {
        let prefix = format!("{name}.yaml.broken-");
        fs::read_dir(profiles_dir())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect()
    }

    /// A profile that fails to parse at startup must not be replaced by
    /// `defaults + one toggle` the first time a setting changes: the manager
    /// starts degraded, keeps a `.broken-<ts>` copy, applies changes in memory
    /// only, and resumes persisting once the file loads again.
    #[test]
    fn startup_parse_error_never_overwrites_the_profile() {
        let _home = TempHome::new("degraded-test");
        // `osd.width` must be an integer — a type error fails the extract.
        let broken = "osd:\n  width: not-a-number\n";
        let path = write_profile("work", broken);

        let mgr = ConfigManager::new();
        assert!(mgr.is_degraded(), "a parse error marks the config degraded");
        let copies = broken_copies("work");
        assert_eq!(copies.len(), 1, "one quarantine copy");
        assert_eq!(fs::read_to_string(&copies[0]).unwrap(), broken);

        let width = mgr.config.read_untracked().osd.width;
        mgr.update_config(|c| c.osd.width = width + 5);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            broken,
            "degraded: the broken profile is left for the user to fix"
        );
        assert_eq!(
            mgr.config.read_untracked().osd.width,
            width + 5,
            "the change still applies for this session"
        );

        // The user fixes the file; a successful reload lifts the guard.
        fs::write(&path, "osd:\n  width: 333\n").unwrap();
        mgr.reload_config();
        assert!(!mgr.is_degraded(), "a clean reload clears degraded");
        mgr.update_config(|c| c.osd.distance += 1);
        mgr.flush();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("width: 333"),
            "kept the user's value: {saved}"
        );
        assert!(saved.contains("distance"), "persisted the change: {saved}");
    }

    /// Switching to a profile that fails to load keeps the previous profile's
    /// config live; persisting that into the new profile would clobber it.
    #[test]
    fn failed_profile_switch_does_not_write_old_config_into_new_profile() {
        let _home = TempHome::new("degraded-switch-test");
        write_profile("good", "osd:\n  width: 321\n");
        let mgr = ConfigManager::new();
        assert!(!mgr.is_degraded());

        let bad_body = "osd:\n  width: [1, 2]\n";
        let bad = profile_path("bad");
        fs::write(&bad, bad_body).unwrap();
        mgr.set_active_profile(Some("bad".to_string()));
        assert!(mgr.is_degraded(), "the failed load marks degraded");

        mgr.update_config(|c| c.osd.distance += 1);
        assert_eq!(fs::read_to_string(&bad).unwrap(), bad_body);
    }

    /// With no cached active profile but an existing `default.yaml`, the
    /// manager loads that file rather than showing bare defaults — otherwise
    /// the first toggle writes `defaults + toggle` over it.
    #[test]
    fn missing_cache_adopts_existing_default_profile() {
        let _home = TempHome::new("nocache-test");
        let path = default_config_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "osd:\n  width: 444\n").unwrap();

        let mgr = ConfigManager::new();
        assert_eq!(mgr.config.read_untracked().osd.width, 444);
        mgr.update_config(|c| c.osd.distance += 1);
        mgr.flush();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("width: 444"), "user value kept: {saved}");
    }

    /// A slider drag (many `update_config` calls in a row) applies every
    /// step to the live config at once but writes the profile once, with
    /// the final value, off the calling thread.
    #[test]
    fn rapid_updates_apply_at_once_and_persist_once() {
        let _home = TempHome::new("debounce-test");
        let path = write_profile("work", "osd:\n  width: 300\n");
        let mgr = ConfigManager::new();
        let before = fs::read_to_string(&path).unwrap();
        for w in 1..=60 {
            mgr.update_config(|c| c.osd.width = 300 + w);
            assert_eq!(
                mgr.config.read_untracked().osd.width,
                300 + w,
                "live at once"
            );
        }
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "nothing written synchronously"
        );
        mgr.flush();
        assert_eq!(mgr.persister.writes(), 1, "one write for the burst");
        assert!(fs::read_to_string(&path).unwrap().contains("width: 360"));
    }
}
