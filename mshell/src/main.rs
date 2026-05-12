//! mshell — bar shell for the margo Wayland compositor.
//!
//! GTK4 + gtk4-layer-shell rewrite (in progress, see CHANGELOG).
//! The entrypoint sets up the GTK Application, installs the global
//! CSS provider, and spawns one bar window per Wayland output.
//! Per-monitor layout + module wiring lives in `bar.rs`.

mod bar;
mod modules;
mod restart;
mod services;
mod state;
mod wallpaper;
mod widgets;

use clap::{Parser, Subcommand};
use gio::prelude::*;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{Application, CssProvider};

const APP_ID: &str = "io.margo.mshell";
const STYLE: &str = include_str!("../assets/style.css");

#[derive(Parser, Debug)]
#[command(
    version = env!("CARGO_PKG_VERSION"),
    about = "Bar shell for the margo Wayland compositor (gtk4-rewrite branch)"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Stop any running mshell instance and launch a fresh detached
    /// one. Useful after rebuilding the binary.
    Restart,
}

fn main() -> glib::ExitCode {
    init_logging();
    let args = Args::parse();

    // `mshell restart` doesn't open a GTK Application — it just
    // talks to /proc + spawns. Handle it before Application::new
    // so we don't pay the GTK init cost (and don't fight a second
    // application instance on the way out).
    if matches!(args.command, Some(Command::Restart)) {
        if let Err(e) = restart::run() {
            eprintln!("mshell restart: {e:#}");
            return glib::ExitCode::FAILURE;
        }
        return glib::ExitCode::SUCCESS;
    }

    // Strip command-line arguments from glib's argv so GTK doesn't
    // complain about the unrecognised `restart` subcommand if we
    // ever add more. `.run()` with no args means "use the default
    // application logic".
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| install_css());
    app.connect_activate(spawn_bars);
    app.run_with_args::<&str>(&[])
}

fn init_logging() {
    let filter = std::env::var("MSHELL_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn install_css() {
    let provider = CssProvider::new();
    provider.load_from_string(STYLE);
    let Some(display) = gdk::Display::default() else {
        tracing::warn!("no default Gdk display — CSS provider not installed");
        return;
    };
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn spawn_bars(app: &Application) {
    let Some(display) = gdk::Display::default() else {
        tracing::error!("no default Gdk display — cannot spawn bars");
        return;
    };
    let monitors = display.monitors();
    let n = monitors.n_items();
    tracing::info!(count = n, "spawning bars across outputs");
    for i in 0..n {
        let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else {
            tracing::warn!(idx = i, "monitor list item could not be downcast");
            continue;
        };
        bar::build(app, &monitor);
    }

    // Wallpaper driver — polls margo's state.json and dispatches
    // per-output paths through `swww img`. Works alongside the
    // bar; if swww-daemon isn't running we just log + skip.
    services::wallpaper::start();
}
