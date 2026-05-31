use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::subcommands::audio::AudioCommands;
use crate::subcommands::bar::BarCommands;
use crate::subcommands::brightness::BrightnessCommands;
use crate::subcommands::lock::LockCommands;
use crate::subcommands::media::MediaCommands;
use crate::subcommands::menu::MenuCommands;
use crate::subcommands::plugin::PluginCommands;
use crate::subcommands::screenshot::ScreenshotCommands;
use crate::subcommands::settings::SettingsCommands;
use crate::subcommands::wallpaper::WallpaperCommands;
use mshell_cli_style;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(styles = mshell_cli_style::get_styles())]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Kill mshell
    Quit,
    /// Launch the GTK4 inspector.  Useful for finding css node id's and classes.
    Inspect,
    /// Set the current wallpaper
    SetWallpaper {
        /// Path to the image file
        path: PathBuf,
    },
    /// Commands for opening and closing menus
    Menu {
        #[command(subcommand)]
        command: MenuCommands,
    },
    /// Commands for hiding and revealing bars
    Bar {
        #[command(subcommand)]
        command: BarCommands,
    },
    /// Control the Hidden Bar drawer widget
    HiddenBar {
        /// toggle | expand | collapse | pin | unpin
        action: String,
    },
    /// Commands for changing audio
    Audio {
        #[command(subcommand)]
        command: AudioCommands,
    },
    /// Commands for controlling media players (MPRIS)
    Media {
        #[command(subcommand)]
        command: MediaCommands,
    },
    /// Commands for changing brightness
    Brightness {
        #[command(subcommand)]
        command: BrightnessCommands,
    },
    /// Lock the session. Bare `lock` locks; `lock check` reports state.
    Lock {
        #[command(subcommand)]
        command: Option<LockCommands>,
    },
    /// Commands for the settings window
    Settings {
        #[command(subcommand)]
        command: SettingsCommands,
    },
    /// Open the in-shell setup wizard (a layer-shell menu)
    Wizard,
    /// Commands for cycling the wallpaper
    Wallpaper {
        #[command(subcommand)]
        command: WallpaperCommands,
    },
    /// Commands for installed WASM plugins (reload from disk, …)
    Plugin {
        #[command(subcommand)]
        command: PluginCommands,
    },
    /// Bridge commands for the mscreenshot CLI — currently the
    /// `select-region` helper that lets external tools reuse the
    /// in-shell area selector (preview state, snap-to-window,
    /// aspect info) instead of spawning `slurp`.
    Screenshot {
        #[command(subcommand)]
        command: ScreenshotCommands,
    },
}
