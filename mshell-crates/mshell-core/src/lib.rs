mod ipc;
mod monitors;
mod relm_app;

use crate::relm_app::{Shell, ShellInit};
use any_spawner::Executor;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, IconsStoreFields, ThemeStoreFields,
};
use mshell_idle::inhibitor::IdleInhibitor;
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
            if let Some(msg) = msg {
                if msg.starts_with("Child name '") && msg.contains("not found in GtkStack") {
                    return glib::LogWriterOutput::Handled;
                }
            }
        }
        glib::log_writer_default(level, fields)
    });

    Executor::init_glib().expect("Executor could not be initialized.");

    let config_manager = mshell_config::config_manager::config_manager();
    config_manager.watch_config();

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
