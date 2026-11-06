// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

mod application;
mod audio;
mod bridge;
mod config;
mod cover_picture;
mod dbus;
mod discover;
mod drag_overlay;
mod i18n;
mod library;
mod marquee;
mod net;
mod notify;
mod playback_control;
mod playlist;
mod playlist_view;
mod podcasts;
mod queue_row;
mod radio;
mod search;
mod song_cover;
mod song_details;
mod sort;
mod tray;
mod utils;
mod volume_control;
mod waveform_view;
mod window;

use std::env;

use config::{APPLICATION_ID, GETTEXT_PACKAGE, PROFILE};
use gettextrs::{LocaleCategory, bind_textdomain_codeset, bindtextdomain, setlocale, textdomain};
use gtk::{gio, glib, prelude::*};
use log::{LevelFilter, debug};

use self::application::Application;

fn main() -> glib::ExitCode {
    let mut builder = pretty_env_logger::formatted_builder();
    if APPLICATION_ID.ends_with("Devel") {
        builder.filter(Some("mtune"), LevelFilter::Debug);
    } else {
        builder.filter(Some("mtune"), LevelFilter::Info);
    }
    builder.init();

    // Set up gettext translations
    debug!("Setting up locale data");
    // SAFETY: setlocale reads the environment and mutates process-global
    // locale state; it is sound only while no other thread exists. This is
    // the start of `main`: the logger above spawns no threads and GTK /
    // gstreamer are not initialised yet.
    unsafe {
        setlocale(LocaleCategory::LcAll, "");
    }

    let gettext = bindtextdomain(GETTEXT_PACKAGE, config::localedir())
        .and_then(|_| bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8"))
        .and_then(|_| textdomain(GETTEXT_PACKAGE));
    if let Err(e) = gettext {
        log::warn!("Translations unavailable: {e}");
    }

    debug!("Setting up pulseaudio environment");
    let app_id = APPLICATION_ID.trim_end_matches(".Devel");
    // SAFETY: single-threaded, before any threads are spawned (GTK / gstreamer
    // are still uninitialised here). Rust 2024 marks `set_var` unsafe for the
    // multi-threaded case, which does not apply at this point in `main`.
    unsafe {
        env::set_var("PULSE_PROP_application.icon_name", app_id);
        env::set_var("PULSE_PROP_application.name", "Tune");
        env::set_var("PULSE_PROP_media.role", "music");
    }

    debug!("Loading resources");
    let resources = gio::Resource::from_data(&glib::Bytes::from_static(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/mtune.gresource"
    ))))
    .expect("compiled-in mtune.gresource is valid");
    gio::resources_register(&resources);

    debug!("Setting up application (profile: {})", &PROFILE);
    glib::set_application_name("Tune");
    glib::set_program_name(Some("mtune"));

    if let Err(e) = gst::init() {
        eprintln!("mtune: GStreamer could not be initialised: {e}");
        return glib::ExitCode::FAILURE;
    }

    let ctx = glib::MainContext::default();
    // The default context, on the main thread, before anything else owns it.
    let _guard = ctx.acquire().unwrap();

    let app = Application::new();
    // Read `--hidden` straight from argv (before `run()` hands the rest
    // to GApplication, which knows the flag via `add_main_option`).
    if env::args().any(|a| a == "--hidden") {
        app.set_start_hidden(true);
    }
    app.run()
}
