//! `mshellctl play …` — native mpv companion, proxied to `mplay`
//! (window control + video-wallpaper engine + yt-dlp shim).

use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PlayCommands {
    /// Start the mpv companion window.
    Start,
    /// Toggle the mpv window (show/hide).
    Toggle,
    /// Play a target (URL / path / clipboard, per mplay's rules).
    Play {
        /// What to play (omit to resume).
        target: Option<String>,
    },
    /// Stop playback / close the window.
    Stop,
    /// Snapshot the current frame.
    Snap,
    /// Pin the window on top.
    Pin,
    /// Focus the mpv window.
    Focus,
    /// Video-wallpaper engine — `wallpaper start [PATH]` / `wallpaper stop`.
    Wallpaper {
        /// `start [PATH]` or `stop`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Any other `mplay` subcommand passes through — e.g.
    /// `play download <url>`, `play media next`.
    #[command(external_subcommand)]
    Exec(Vec<String>),
}

pub async fn execute(command: PlayCommands) -> anyhow::Result<()> {
    match command {
        PlayCommands::Start => proxy::run("mplay", ["start"]),
        PlayCommands::Toggle => proxy::run("mplay", ["toggle"]),
        PlayCommands::Play { target } => {
            let mut args = vec!["play".to_string()];
            if let Some(target) = target {
                args.push(target);
            }
            proxy::run("mplay", &args)
        }
        PlayCommands::Stop => proxy::run("mplay", ["stop"]),
        PlayCommands::Snap => proxy::run("mplay", ["snap"]),
        PlayCommands::Pin => proxy::run("mplay", ["pin"]),
        PlayCommands::Focus => proxy::run("mplay", ["focus"]),
        PlayCommands::Wallpaper { args } => {
            let mut argv = vec!["wallpaper".to_string()];
            argv.extend(args);
            proxy::run("mplay", &argv)
        }
        PlayCommands::Exec(args) => proxy::run("mplay", &args),
    }
}
