//! mplay — margo's native mpv companion: window control + video wallpaper.

mod geometry;
mod margo;
mod mpv_ipc;
mod ytdl;

fn main() -> anyhow::Result<()> {
    println!("mplay");
    Ok(())
}
