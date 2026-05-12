use crate::config::{Position, get_config};
use crate::outputs::Outputs;
use app::App;
use clap::Parser;
use flexi_logger::{
    Age, Cleanup, Criterion, FileSpec, LogSpecBuilder, LogSpecification, Logger, Naming,
};
use iced::{Anchor, Font, KeyboardInteractivity, Layer, LayerShellSettings};
use log::{debug, error, warn};
use std::backtrace::Backtrace;
use std::panic;
use std::path::PathBuf;

mod app;
mod components;
mod config;
mod i18n;
mod ipc;
mod matugen;
mod modules;
mod osd;
mod outputs;
mod services;
mod theme;
mod utils;
mod wallpaper;

const NERD_FONT: &[u8] = include_bytes!("../target/generated/SymbolsNerdFont-Regular-Subset.ttf");
const NERD_FONT_MONO: &[u8] =
    include_bytes!("../target/generated/SymbolsNerdFontMono-Regular-Subset.ttf");
const CUSTOM_FONT: &[u8] = include_bytes!("../assets/MshellCustomIcon-Regular.otf");
const TMP_FILE_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"),
    about = env!("CARGO_PKG_DESCRIPTION")
)]
struct Args {
    #[arg(short, long, value_parser = clap::value_parser!(PathBuf))]
    config_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Send a message to a running mshell instance
    Msg {
        #[command(subcommand)]
        command: ipc::IpcCommand,
    },
    /// Generate a Material You palette from the active output's
    /// wallpaper (or an explicit path) and live-apply it via
    /// `~/.config/margo/matugen/config.toml` + `mctl reload`.
    Matugen {
        /// Wallpaper image. If omitted, the active output's
        /// `[wallpaper.tags]` entry for the focused tag is used.
        wallpaper: Option<PathBuf>,
    },
}

fn get_log_spec(log_level: &str) -> LogSpecification {
    // Use MSHELL_LOG as the env-override hook instead of RUST_LOG.
    // Sharing RUST_LOG with margo (the compositor binary that
    // launches us via exec-once) was a silent footgun: a user
    // debugging margo with `RUST_LOG=margo=debug` would unknowingly
    // mute every mshell::* log because flexi_logger's env_or_parse
    // would adopt that spec verbatim and drop everything outside
    // the `margo` crate namespace.
    let spec_source = std::env::var("MSHELL_LOG")
        .unwrap_or_else(|_| log_level.to_string());

    // Drop chatty WARN/INFO lines from the wgpu/iced backend
    // (Vulkan extension hints, GLES re-init, mesa quirks) — they're
    // noise from the renderer, not actionable for the user.
    // If the spec already names wgpu_*/naga/iced_*, honour it.
    let needs_suppress = !spec_source
        .split(',')
        .any(|s| s.starts_with("wgpu") || s.starts_with("naga") || s.starts_with("iced"));
    let spec_str = if needs_suppress {
        format!(
            "{spec_source},wgpu_hal=error,wgpu_core=error,naga=error,iced_wgpu=error"
        )
    } else {
        spec_source
    };

    match LogSpecification::parse(&spec_str) {
        Ok(spec) => spec,
        Err(err) => {
            warn!("Failed to parse log level {spec_str:?}: {err}, using default");
            LogSpecification::default()
        }
    }
}

fn main() -> iced::Result {
    let args = Args::parse();

    if let Some(Command::Msg { command }) = &args.command {
        if let Err(e) = ipc::run_client(command) {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    if let Some(Command::Matugen { wallpaper }) = args.command.as_ref() {
        if let Err(e) = matugen::run_cli(wallpaper.clone()) {
            eprintln!("mshell matugen: {e:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    debug!("args: {args:?}");

    let logger = Logger::with(
        LogSpecBuilder::new()
            .default(log::LevelFilter::Info)
            .module("wgpu_hal", log::LevelFilter::Error)
            .module("wgpu_core", log::LevelFilter::Error)
            .module("naga", log::LevelFilter::Error)
            .module("iced_wgpu", log::LevelFilter::Error)
            .build(),
    )
    .log_to_file(FileSpec::default().directory("/tmp/mshell"))
    .duplicate_to_stdout(flexi_logger::Duplicate::All)
    .rotate(
        Criterion::AgeOrSize(Age::Day, TMP_FILE_SIZE),
        Naming::Timestamps,
        Cleanup::KeepLogFiles(7),
    );
    let logger = if cfg!(debug_assertions) {
        logger.duplicate_to_stdout(flexi_logger::Duplicate::All)
    } else {
        logger
    };
    let logger = logger.start().unwrap_or_else(|e| {
        eprintln!("Failed to initialize file logger: {e}, falling back to stderr-only");
        Logger::with(
            LogSpecBuilder::new()
                .default(log::LevelFilter::Info)
                .build(),
        )
        .start()
        .expect("critical: cannot initialize any logger")
    });
    panic::set_hook(Box::new(|info| {
        let b = Backtrace::capture();
        error!("Panic: {info} \n {b}");
    }));

    let (config, config_path) = get_config(args.config_path).unwrap_or_else(|err| {
        error!("Failed to read config: {err}");

        std::process::exit(1);
    });

    logger.set_new_spec(get_log_spec(&config.log_level));

    let font = if let Some(font_name) = &config.appearance.font_name {
        Font::with_name(Box::leak(font_name.clone().into_boxed_str()))
    } else {
        Font::DEFAULT
    };

    let height = Outputs::get_height(config.appearance.style, config.appearance.scale_factor);

    let iced_layer = match config.layer {
        config::Layer::Top => Layer::Top,
        config::Layer::Bottom => Layer::Bottom,
        config::Layer::Overlay => Layer::Overlay,
    };

    iced::application(
        App::new((logger, config.clone(), config_path)),
        App::update,
        App::view,
    )
    .layer_shell(LayerShellSettings {
        anchor: match config.position {
            Position::Top => Anchor::TOP,
            Position::Bottom => Anchor::BOTTOM,
        } | Anchor::LEFT
            | Anchor::RIGHT,
        layer: iced_layer,
        exclusive_zone: height as i32,
        size: Some((0, height as u32)),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: "mshell-main-layer".into(),
        ..Default::default()
    })
    .subscription(App::subscription)
    .theme(App::theme)
    .scale_factor(App::scale_factor)
    .font(NERD_FONT)
    .font(NERD_FONT_MONO)
    .font(CUSTOM_FONT)
    .default_font(font)
    .run()
}
