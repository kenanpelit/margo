//! mplay — margo's native mpv companion: window control + video wallpaper.

mod cli;
mod control;
mod geometry;
mod margo;
mod media;
mod mpv_ipc;
mod paper;
mod ytdl;
mod ytdl_shim;

use anyhow::{Result, anyhow};
use clap::Parser;
use cli::{Cli, Command, WallpaperCmd};
use geometry::ScaleMode;
use paper::PaperOpts;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Start => control::start(),
        Command::Toggle => control::toggle(),
        Command::Play { url } => control::play(url.as_deref()),
        Command::Download { url } => control::download(url.as_deref()),
        Command::Snap => control::snap(),
        Command::Pin => control::pin(),
        Command::Focus => control::focus(),
        Command::Stop => control::stop(),
        Command::Media { action, player } => {
            let cmd = media::status::Command::parse(&action).ok_or_else(|| {
                anyhow!("geçersiz media komutu: {action} (toggle|play|pause|stop|next|prev|status)")
            })?;
            media::run(cmd, player.as_deref())
        }
        Command::Wallpaper(w) => match w {
            WallpaperCmd::Start {
                src,
                output,
                mute,
                no_loop,
                scale,
                daemon,
            } => {
                let scale = ScaleMode::parse(&scale)
                    .ok_or_else(|| anyhow!("geçersiz --scale: {scale} (fit|fill|stretch)"))?;
                let src = control::resolve_source(src.as_deref());
                if src.is_empty() {
                    return Err(anyhow!(
                        "wallpaper: kaynak yok (argüman ver veya panoya bir yol/URL koy)"
                    ));
                }
                let opts = PaperOpts {
                    mute,
                    looping: !no_loop,
                    scale,
                };
                paper::run(&src, output.as_deref(), opts, daemon)
            }
            WallpaperCmd::Stop { output } => paper::stop(output.as_deref()),
        },
        Command::Ytdlp { args } => std::process::exit(ytdl_shim::run(&args)),
    }
}
