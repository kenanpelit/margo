//! `mshellctl play …` — native mpv companion window control + playback +
//! yt-dlp downloads (`crate::mpv`). Only the video-wallpaper engine stays
//! proxied to `mplay` (its own embedded-libmpv EGL/Wayland renderer, a
//! separate scope not yet natively ported).

use crate::mpv::{control, ytdl_shim};
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
    /// Download a YouTube URL (argument or clipboard) to ~/Downloads via yt-dlp.
    Download {
        /// The URL (omit to use the clipboard).
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
    /// Hidden: mpv's `ytdl_hook` invokes this as its `ytdl_path` (wired up
    /// by `control::ensure_ytdl_shim`) — not meant to be run by hand.
    #[command(name = "__ytdlp", hide = true, trailing_var_arg = true)]
    Ytdlp { args: Vec<String> },
    /// Any other `mplay` subcommand passes through — e.g. `play media next`.
    #[command(external_subcommand)]
    Exec(Vec<String>),
}

pub async fn execute(command: PlayCommands) -> anyhow::Result<()> {
    match command {
        PlayCommands::Start => control::start(),
        PlayCommands::Toggle => control::toggle(),
        PlayCommands::Play { target } => control::play(target.as_deref()),
        PlayCommands::Download { target } => control::download(target.as_deref()),
        PlayCommands::Stop => control::stop(),
        PlayCommands::Snap => control::snap(),
        PlayCommands::Pin => control::pin(),
        PlayCommands::Focus => control::focus(),
        PlayCommands::Wallpaper { args } => {
            let mut argv = vec!["wallpaper".to_string()];
            argv.extend(args);
            proxy::run("mplay", &argv)
        }
        PlayCommands::Ytdlp { args } => std::process::exit(ytdl_shim::run(&args)),
        PlayCommands::Exec(args) => proxy::run("mplay", &args),
    }
}
