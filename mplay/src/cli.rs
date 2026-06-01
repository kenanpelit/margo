//! clap command surface for mplay.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mplay",
    about = "margo's native mpv companion — window control + video wallpaper",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Launch mpv (pseudo-gui) with a JSON IPC socket
    Start,
    /// Toggle play/pause
    Toggle,
    /// Play a file/URL (argument or clipboard; ytdl auto-detected)
    Play {
        /// Source path or URL (defaults to clipboard contents)
        url: Option<String>,
    },
    /// Download a YouTube video to ~/Downloads (yt-dlp)
    #[command(alias = "dl")]
    Download {
        /// YouTube URL (defaults to clipboard contents)
        url: Option<String>,
    },
    /// Cycle the floating mpv window across the four corners
    Snap,
    /// Toggle pinning mpv to all tags (sticky)
    Pin,
    /// Focus the mpv window (hopping monitor/tag as needed)
    Focus,
    /// Quit mpv
    Stop,
    /// Native video wallpaper (start/stop)
    #[command(alias = "wall", subcommand)]
    Wallpaper(WallpaperCmd),
    /// Internal: yt-dlp shim invoked by mpv's ytdl_hook (not for direct use)
    #[command(hide = true, trailing_var_arg = true, allow_hyphen_values = true)]
    Ytdlp {
        /// yt-dlp arguments forwarded by mpv
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum WallpaperCmd {
    /// Play a video wallpaper on the background layer
    Start {
        /// Video file or URL
        src: String,
        /// Target output name (default: all outputs)
        #[arg(long)]
        output: Option<String>,
        /// Mute audio
        #[arg(long)]
        mute: bool,
        /// Play once instead of looping
        #[arg(long = "no-loop")]
        no_loop: bool,
        /// Scale mode: fit | fill | stretch
        #[arg(long, default_value = "fill")]
        scale: String,
        /// Fork into the background
        #[arg(long)]
        daemon: bool,
    },
    /// Stop the video wallpaper
    Stop {
        /// Target output name (default: all)
        #[arg(long)]
        output: Option<String>,
    },
}
