//! Native mpv companion — `mshellctl play …`'s backend. Ported from
//! `mplay`'s controller: window lifecycle, placement, playback, and the
//! yt-dlp shim. The video-wallpaper engine (`mplay`'s `paper` module — a
//! standalone embedded-libmpv EGL/Wayland renderer) stays on `mplay`
//! pending a decision on embedding it in mshell's own GTK4/EGL stack.

pub mod control;
pub mod geometry;
pub mod ipc;
pub mod margo_client;
pub mod ytdl;
pub mod ytdl_shim;
