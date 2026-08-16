//! `mshellctl play …` — native mpv companion window control
//! (`crate::mpv::control`); `play <target>`/the video-wallpaper engine/
//! yt-dlp downloads stay proxied to `mplay` for now (a separate
//! yt-dlp-adjacent scope not yet natively ported).

use crate::mpv::control;
use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PlayCommands {
    /// Start the mpv companion window.
    Start,
    /// Toggle play/pause on the running mpv companion.
    Toggle,
    /// Play a target (URL / path / clipboard, per mplay's rules).
    Play {
        /// What to play (omit to resume).
        target: Option<String>,
    },
    /// Stop playback / close the window.
    Stop,
    /// Cycle the floating window to the next screen corner.
    Snap,
    /// Pin the window on top (toggle sticky).
    Pin,
    /// Focus the mpv window (hop monitor + tag + focus stack).
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
        PlayCommands::Start => control::start(),
        PlayCommands::Toggle => control::toggle(),
        PlayCommands::Play { target } => {
            let mut args = vec!["play".to_string()];
            if let Some(target) = target {
                args.push(target);
            }
            proxy::run("mplay", &args)
        }
        PlayCommands::Stop => control::stop(),
        PlayCommands::Snap => control::snap(),
        PlayCommands::Pin => control::pin(),
        PlayCommands::Focus => control::focus(),
        PlayCommands::Wallpaper { args } => {
            let mut argv = vec!["wallpaper".to_string()];
            argv.extend(args);
            proxy::run("mplay", &argv)
        }
        PlayCommands::Exec(args) => proxy::run("mplay", &args),
    }
}
