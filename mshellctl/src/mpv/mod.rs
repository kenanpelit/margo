//! Native mpv companion — `mshellctl play …`'s backend. Ported from
//! `mplay`'s controller: window lifecycle, placement, playback, the
//! yt-dlp shim, and the video-wallpaper engine (`paper` — its own EGL
//! context + raw Wayland client, entirely separate from `mshell`'s GTK4
//! process; see `paper`'s module doc for why that's safe).

pub mod control;
pub mod geometry;
pub mod ipc;
pub mod margo_client;
pub mod paper;
pub mod ytdl;
pub mod ytdl_shim;
