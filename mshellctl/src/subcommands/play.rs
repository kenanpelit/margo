//! `mshellctl play …` — native mpv companion: window control, playback,
//! yt-dlp downloads, and the video-wallpaper engine (`crate::mpv`).

use crate::mpv::geometry::ScaleMode;
use crate::mpv::youtube::Hints;
use crate::mpv::{control, paper, track, ytdl_shim};
use clap::{Args, Subcommand};

/// Metadata for a YouTube video's history row, passed by the launcher's
/// `yt` provider (mpv only learns the title once yt-dlp answers).
#[derive(Args, Debug, Default)]
pub struct VideoMeta {
    /// Video title.
    #[arg(long)]
    title: Option<String>,
    /// Channel name.
    #[arg(long)]
    channel: Option<String>,
    /// Channel id (UC…).
    #[arg(long = "channel-id")]
    channel_id: Option<String>,
    /// Length in seconds.
    #[arg(long)]
    duration: Option<f64>,
}

impl VideoMeta {
    fn hints(self) -> Hints {
        Hints {
            title: self.title,
            channel: self.channel,
            channel_id: self.channel_id,
            duration: self.duration,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum PlayCommands {
    /// Start the mpv companion window.
    Start,
    /// Toggle play/pause on the running mpv companion.
    Toggle,
    /// Play a target (URL / path / clipboard; YouTube URLs auto-detected).
    /// YouTube videos resume where you stopped (a `t=` in the URL wins).
    Play {
        /// What to play (omit to resume).
        target: Option<String>,
        #[command(flatten)]
        meta: VideoMeta,
    },
    /// Append a target to the mpv companion's playlist (starts mpv when
    /// it isn't running) — one window, played in order.
    Queue {
        /// What to queue (omit to use the clipboard).
        target: Option<String>,
        #[command(flatten)]
        meta: VideoMeta,
    },
    /// Forget a YouTube video's history entry and resume point.
    Forget {
        /// The 11-character video id.
        #[arg(required_unless_present = "all")]
        id: Option<String>,
        /// Forget every video (clear the history).
        #[arg(long, conflicts_with = "id")]
        all: bool,
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
    /// Video-wallpaper engine — background layer surface, EGL + libmpv.
    Wallpaper {
        #[command(subcommand)]
        command: WallpaperCommands,
    },
    /// Hidden: the YouTube resume/history tracker `play` starts next to
    /// mpv (`crate::mpv::track`).
    #[command(name = "__track", hide = true)]
    Track,
    /// Hidden: mpv's `ytdl_hook` invokes this as its `ytdl_path` (wired up
    /// by `control::ensure_ytdl_shim`) — not meant to be run by hand.
    #[command(name = "__ytdlp", hide = true, trailing_var_arg = true)]
    Ytdlp {
        /// `trailing_var_arg` alone only stops swallowing hyphenated
        /// tokens as options once a prior positional value has already
        /// flipped clap into "trailing" mode — but `ytdl_hook` always
        /// passes `--no-warnings` as the very first argument, so without
        /// `allow_hyphen_values` clap rejects it as an unknown option
        /// before any value is ever consumed.
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum WallpaperCommands {
    /// Play a video wallpaper on the background layer.
    Start {
        /// Video file or URL (defaults to clipboard contents).
        src: Option<String>,
        /// Target output name (default: all outputs).
        #[arg(long)]
        output: Option<String>,
        /// Mute audio.
        #[arg(long)]
        mute: bool,
        /// Play once instead of looping.
        #[arg(long = "no-loop")]
        no_loop: bool,
        /// Scale mode: fit | fill | stretch.
        #[arg(long, default_value = "fill")]
        scale: String,
        /// Fork into the background.
        #[arg(long)]
        daemon: bool,
    },
    /// Stop the video wallpaper.
    Stop {
        /// Target output name (default: all).
        #[arg(long)]
        output: Option<String>,
    },
}

pub async fn execute(command: PlayCommands) -> anyhow::Result<()> {
    match command {
        PlayCommands::Start => control::start(),
        PlayCommands::Toggle => control::toggle(),
        PlayCommands::Play { target, meta } => {
            control::load(target.as_deref(), control::LoadMode::Replace, &meta.hints())
        }
        PlayCommands::Queue { target, meta } => {
            control::load(target.as_deref(), control::LoadMode::Queue, &meta.hints())
        }
        PlayCommands::Forget { id, all } => control::forget(if all { None } else { id.as_deref() }),
        PlayCommands::Track => track::run(),
        PlayCommands::Download { target } => control::download(target.as_deref()),
        PlayCommands::Stop => control::stop(),
        PlayCommands::Snap => control::snap(),
        PlayCommands::Pin => control::pin(),
        PlayCommands::Focus => control::focus(),
        PlayCommands::Wallpaper { command } => match command {
            WallpaperCommands::Start {
                src,
                output,
                mute,
                no_loop,
                scale,
                daemon,
            } => {
                let scale = ScaleMode::parse(&scale).ok_or_else(|| {
                    anyhow::anyhow!("geçersiz --scale: {scale} (fit|fill|stretch)")
                })?;
                let src = control::resolve_source(src.as_deref());
                if src.is_empty() {
                    anyhow::bail!(
                        "wallpaper: kaynak yok (argüman ver veya panoya bir yol/URL koy)"
                    );
                }
                let opts = paper::PaperOpts {
                    mute,
                    looping: !no_loop,
                    scale,
                };
                paper::run(&src, output.as_deref(), opts, daemon)
            }
            WallpaperCommands::Stop { output } => paper::stop(output.as_deref()),
        },
        PlayCommands::Ytdlp { args } => std::process::exit(ytdl_shim::run(&args)),
    }
}

#[cfg(test)]
mod tests {
    use crate::app::Cli;
    use clap::Parser;

    /// mpv's `ytdl_hook` invokes the shim with yt-dlp-style flags as the
    /// very first argument (e.g. `--no-warnings`), before any non-hyphen
    /// value has been seen. `trailing_var_arg` alone only starts absorbing
    /// hyphenated tokens once a prior positional value flips clap into
    /// "trailing" mode — the first token still gets matched against known
    /// long options and rejected. Regression for the real invocation
    /// `ytdl_hook` makes against `mshellctl play __ytdlp`.
    #[test]
    fn ytdlp_shim_accepts_leading_hyphen_args() {
        let cli = Cli::try_parse_from([
            "mshellctl",
            "play",
            "__ytdlp",
            "--no-warnings",
            "-J",
            "--flat-playlist",
            "--",
            "some-target",
        ]);
        assert!(cli.is_ok(), "{:?}", cli.err());
    }

    /// The launcher's `yt` provider invokes exactly these shapes.
    #[test]
    fn launcher_invocations_parse() {
        for argv in [
            &[
                "mshellctl",
                "play",
                "play",
                "https://www.youtube.com/watch?v=5C_HPTJg5ek",
                "--title",
                "Rust in 100 Seconds",
                "--channel",
                "Fireship",
                "--channel-id",
                "UCsBjURrPoezykLs9EqgamOA",
                "--duration",
                "149",
            ][..],
            &["mshellctl", "play", "queue", "https://youtu.be/5C_HPTJg5ek"],
            &["mshellctl", "play", "forget", "5C_HPTJg5ek"],
            &["mshellctl", "play", "forget", "--all"],
            &["mshellctl", "play", "__track"],
        ] {
            let cli = Cli::try_parse_from(argv);
            assert!(cli.is_ok(), "{argv:?}: {:?}", cli.err());
        }
        assert!(Cli::try_parse_from(["mshellctl", "play", "forget"]).is_err());
    }
}
