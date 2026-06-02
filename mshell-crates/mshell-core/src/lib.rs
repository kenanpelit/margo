mod ipc;
mod monitors;
mod relm_app;
mod sleep_lock;

use crate::relm_app::{Shell, ShellInit};
use any_spawner::Executor;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, IconsStoreFields, LauncherStoreFields,
    NotificationsStoreFields, ThemeStoreFields,
};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::notification_service;
use mshell_services::weather_service;
use reactive_graph::effect::Effect;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::prelude::*;
use std::cell::Cell;
use std::error::Error;
use tracing::info;
use wayle_weather::{LocationQuery, TemperatureUnit};

// The shared tokio runtime now lives in mshell-services so that
// crates without a dependency on mshell-core can still spawn onto it.
use mshell_services::tokio_rt;

/// Render the resolved plugin keybinds into the margo binds fragment + ask
/// margo to reload if it actually changed. Logs a one-shot hint the first
/// time the user has bindings but hasn't `source=`d our file.
fn sync_plugin_keybinds() {
    use mshell_plugins::{PluginStore, keybinds};
    let store = PluginStore::new();
    if let Err(e) = keybinds::sync_with_margo(&store) {
        tracing::warn!("plugin keybinds: sync failed: {e}");
        return;
    }
    // Log a one-shot hint if the user has bindings but hasn't sourced us.
    let resolved = keybinds::resolve_all(&store);
    let active = resolved.iter().filter(|r| r.is_active()).count();
    if active == 0 {
        return;
    }
    let config_conf = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("margo")
        .join("config.conf");
    if !keybinds::user_sources_us(&config_conf) {
        tracing::info!(
            "plugin keybinds: {active} active binding(s) waiting at {}. \
             Add `source=binds.d/mshell-plugins.conf` to ~/.config/margo/config.conf \
             and run `mctl reload` to activate.",
            keybinds::binds_path().display()
        );
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let start = std::time::Instant::now();
    info!("Welcome to MShell!");

    // Give relm4's private runtime more than a single worker thread.
    // The default (1) is dangerous: every `watch!` macro on a
    // `wayle_*` property, every `sender.command(...)` block, and
    // every `FactoryVecDeque::forward(...)` task lives on that one
    // worker. With ~46 such tasks already spawned at startup, an
    // active `flume::recv_async` or `WatchStream::poll_next` that
    // ends up at the front of the queue can monopolise the thread
    // long enough that the factory output_receiver (the channel
    // behind `Settings → ThemeSelected` and `Bar settings reorder
    // buttons`) never gets serviced — both broke silently on the
    // default. Must be set before relm4 first calls `relm4::spawn`,
    // which is why this is at the very top of `run()`.
    relm4::RELM_THREADS.set(4).ok();

    // Filter out the harmless `GtkStack` "Child name '<name>' not found"
    // warnings. Several panel views (weather, wallpaper) bind
    // `set_visible_child_name` via `#[watch]` on a model field whose
    // initial value names a child that hasn't been appended yet — the
    // property setter runs before the macro emits the `add_named`
    // calls. GTK logs a warning and then no-ops the set; the next
    // `#[watch]` re-fire (once the children exist) succeeds. Drop
    // these specific lines so they don't drown out real warnings.
    relm4::gtk::glib::log_set_writer_func(|level, fields| {
        use relm4::gtk::glib;
        if matches!(level, glib::LogLevel::Warning) {
            let msg = fields.iter().find_map(|f| {
                if f.key() == "MESSAGE" {
                    f.value_str()
                } else {
                    None
                }
            });
            if let Some(msg) = msg
                && msg.starts_with("Child name '")
                && msg.contains("not found in GtkStack")
            {
                return glib::LogWriterOutput::Handled;
            }
        }
        glib::log_writer_default(level, fields)
    });

    Executor::init_glib().expect("Executor could not be initialized.");

    let config_manager = mshell_config::config_manager::config_manager();
    config_manager.watch_config();

    // Seed the clipboard watcher's settings from config before any
    // bar/menu widget touches `clipboard_service()` (which is lazy
    // and would otherwise spin up with defaults). Persistence +
    // history-size + sensitive-skip all flow from here.
    mshell_clipboard::init_settings(clipboard_settings_from_config(
        &config_manager.config().get_untracked().clipboard,
    ));

    // Initialize the effects in the wallpaper store
    let _ = mshell_cache::wallpaper::wallpaper_store();

    let location_query = LocationQuery::from(
        config_manager
            .config()
            .general()
            .weather_location_query()
            .get_untracked(),
    );

    let temperature_units = TemperatureUnit::from(
        config_manager
            .config()
            .general()
            .temperature_unit()
            .get_untracked(),
    );

    tokio_rt().block_on(async {
        mshell_services::init_services(location_query, temperature_units).await
    })?;

    tokio_rt().spawn(async move {
        let _ = IdleInhibitor::global().init().await;
    });

    Effect::new(move |_| {
        let theme = config_manager
            .config()
            .theme()
            .icons()
            .shell_icon_theme()
            .get();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_icon_theme_name(Some(theme.as_str()));
    });

    // Sync the per-app notification blocklist from config → service.
    // Runs once at startup (applies the persisted list) and re-runs
    // whenever Settings edits it, so a mute takes effect immediately
    // and survives restart.
    Effect::new(|_| {
        let blocklist = mshell_config::config_manager::config_manager()
            .config()
            .notifications()
            .blocklist()
            .get();
        notification_service().set_blocklist(blocklist);
    });

    // Autostart: run each `>start` script the user ticked in Settings,
    // `delay_secs` after startup. Spawned by short name via the session
    // $PATH (the same way ScriptsProvider discovered it). One-shot at
    // boot — not reactive; toggling in Settings applies next launch.
    //
    // `LoginOnce` entries are gated on a per-session marker file under
    // $XDG_RUNTIME_DIR (torn down by systemd at logout). If it already
    // exists this is an in-session restart, so we skip them; otherwise
    // it's the first start of the login and we run them + drop the marker.
    let first_start_of_session = !autostart_marker_seen_then_touch();
    for entry in mshell_config::config_manager::config_manager()
        .config()
        .launcher()
        .autostart_scripts()
        .get_untracked()
        .into_iter()
        .filter(|e| e.enabled && !e.name.is_empty())
        .filter(|e| {
            !matches!(
                e.trigger,
                mshell_config::schema::config::AutostartTrigger::LoginOnce
            ) || first_start_of_session
        })
    {
        tokio_rt().spawn(async move {
            if entry.delay_secs > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(entry.delay_secs as u64)).await;
            }
            if let Err(err) = std::process::Command::new(&entry.name).spawn() {
                tracing::warn!(script = %entry.name, ?err, "autostart: spawn failed");
            }
        });
    }

    // Plugin keybinds: generate the binds file each launch + after any change
    // to the resolved set. Idempotent — `write_binds_file` returns false when
    // the contents already match. We call `mctl reload` only if the file
    // actually changed *and* the user has opted in by sourcing it.
    sync_plugin_keybinds();

    // One-shot security migration: any `type = "secret"` plugin setting that
    // still lives in plaintext in plugins.toml (from before this feature
    // shipped) gets moved into the system keyring. Idempotent — costs a
    // toml read each boot once everything is already migrated.
    let moved = mshell_plugins::PluginStore::new().migrate_plaintext_secrets();
    if moved > 0 {
        tracing::info!("migrated {moved} plaintext plugin secret(s) into the keyring");
    }

    // Auto-update: if the user picked "On login" in Settings → Plugins, fetch
    // every source registry ~1 minute after login and reinstall any installed,
    // enabled plugin that has a newer version. Off the main thread; the result
    // is applied on the main loop (reload_config + a desktop notification).
    if mshell_plugins::PluginStore::new()
        .load_state()
        .auto_update_on_login()
    {
        tokio_rt().spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let outcome = tokio::task::spawn_blocking(|| {
                mshell_plugins::PluginStore::new().run_update_pass()
            })
            .await
            .unwrap_or_default();
            relm4::gtk::glib::MainContext::default().invoke(move || {
                let n = outcome.updated.len();
                if n > 0 {
                    mshell_config::config_manager::config_manager().reload_config();
                    let body = format!("Updated {n} plugin(s): {}", outcome.updated.join(", "));
                    let _ = std::process::Command::new("notify-send")
                        .args([
                            "-a",
                            "Plugins",
                            "-i",
                            "software-update-available-symbolic",
                            "Plugin updates installed",
                            &body,
                        ])
                        .spawn();
                } else if !outcome.errors.is_empty() {
                    tracing::warn!(
                        errors = ?outcome.errors,
                        "auto-update: pass had errors"
                    );
                }
            });
        });
    }

    // skip first run
    let initialized = Cell::new(false);
    Effect::new(move |_| {
        let location_query = config_manager
            .config()
            .general()
            .weather_location_query()
            .get();
        if !initialized.get() {
            initialized.set(true);
            return;
        }
        let weather = weather_service();
        weather.set_location(LocationQuery::from(location_query));
    });

    // skip first run
    let initialized = Cell::new(false);
    Effect::new(move |_| {
        let temp_unit = config_manager.config().general().temperature_unit().get();
        if !initialized.get() {
            initialized.set(true);
            return;
        }
        let weather = weather_service();
        weather.set_units(TemperatureUnit::from(temp_unit));
    });

    let app = RelmApp::new("mshell.main");
    info!("Startup completed in {:?}", start.elapsed());
    app.run::<Shell>(ShellInit {});

    info!("Goodbye!");

    Ok(())
}

/// Return whether the per-login autostart marker already existed, and
/// create it if it didn't. `false` means "first mshell start of this
/// login session"; `true` means an in-session restart.
///
/// The marker lives under `$XDG_RUNTIME_DIR/margo/` — that directory is
/// per-login and torn down by systemd at logout, so the marker is gone
/// by the next login and `LoginOnce` scripts run again. If we can't
/// resolve a runtime dir or touch the file, we conservatively report
/// "first start" so a misconfigured environment still autostarts.
fn autostart_marker_seen_then_touch() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from) else {
        tracing::warn!("autostart: XDG_RUNTIME_DIR unset; treating as first login start");
        return false;
    };
    let marker = dir.join("margo").join("autostart.done");
    if marker.exists() {
        return true;
    }
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::File::create(&marker) {
        tracing::warn!(?err, "autostart: could not write session marker");
    }
    false
}

/// Map the YAML clipboard config onto the clipboard crate's
/// runtime settings (the crate is config-agnostic by design).
fn clipboard_settings_from_config(
    c: &mshell_config::schema::clipboard::Clipboard,
) -> mshell_clipboard::ClipboardSettings {
    use mshell_config::schema::clipboard::{ClipboardClearPolicy, ClipboardPersist};
    mshell_clipboard::ClipboardSettings {
        max_entries: c.max_entries.max(1),
        persist: match c.persist {
            ClipboardPersist::None => mshell_clipboard::PersistMode::None,
            ClipboardPersist::FavoritesOnly => mshell_clipboard::PersistMode::FavoritesOnly,
            ClipboardPersist::All => mshell_clipboard::PersistMode::All,
        },
        clear_policy: match c.clear_policy {
            ClipboardClearPolicy::Never => mshell_clipboard::ClearPolicy::Never,
            ClipboardClearPolicy::AfterHours => mshell_clipboard::ClearPolicy::AfterHours,
            ClipboardClearPolicy::OnLogout => mshell_clipboard::ClearPolicy::OnLogout,
        },
        clear_after_hours: c.clear_after_hours,
        skip_sensitive: c.skip_sensitive,
        image_history: c.image_history,
    }
}
