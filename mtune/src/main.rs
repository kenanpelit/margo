// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

mod application;
mod audio;
mod config;
mod cover_picture;
mod drag_overlay;
mod i18n;
// mod library; // Plan 1 phase 2
mod marquee;
mod playback_control;
mod playlist_view;
mod queue_row;
mod search;
mod song_cover;
mod song_details;
mod sort;
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
    setlocale(LocaleCategory::LcAll, "");

    bindtextdomain(GETTEXT_PACKAGE, config::localedir()).expect("Unable to bind the text domain");
    bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8")
        .expect("Unable to set the text domain encoding");
    textdomain(GETTEXT_PACKAGE).expect("Unable to switch to the text domain");

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

    gst::init().expect("Failed to initialize gstreamer");

    let ctx = glib::MainContext::default();
    let _guard = ctx.acquire().unwrap();

    Application::new().run()
}
