//! Native mpv companion window control — `mshellctl play …`'s backend.
//! Ported from `mplay`'s controller (window lifecycle + placement only;
//! `play`/`download`/the video-wallpaper engine stay on `mplay`, a
//! separate yt-dlp-adjacent scope).

pub mod control;
pub mod geometry;
pub mod ipc;
pub mod margo_client;
