//! mpv backend, reusing the JSON IPC socket client (`crate::mpv_ipc`).

use super::status::{Command, Status};
use crate::mpv_ipc;
use std::process::{Command as Proc, Stdio};

pub fn available() -> bool {
    mpv_ipc::socket_ready()
        || Proc::new("pgrep")
            .args(["-x", "mpv"])
            .stdout(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

pub fn status() -> Status {
    if !mpv_ipc::socket_ready() {
        return Status::Unknown;
    }
    if mpv_ipc::get_bool("idle-active") == Some(true) {
        return Status::Stopped;
    }
    match mpv_ipc::get_bool("pause") {
        Some(true) => Status::Paused,
        Some(false) => Status::Playing,
        None => Status::Unknown,
    }
}

/// `(title, artist, album)` best-effort from mpv properties.
pub fn metadata() -> (Option<String>, Option<String>, Option<String>) {
    let title = mpv_ipc::get_string("media-title").or_else(|| {
        mpv_ipc::get_string("path").map(|p| {
            p.rsplit('/')
                .next()
                .unwrap_or(&p)
                .split('?')
                .next()
                .unwrap_or(&p)
                .to_string()
        })
    });
    let artist = mpv_ipc::get_string("metadata/by-key/Artist");
    let album = mpv_ipc::get_string("metadata/by-key/Album");
    (title, artist, album)
}

pub fn control(cmd: Command) -> bool {
    let sock = mpv_ipc::socket_path();
    // All string-form commands so they go through the &[&str] sender
    // (mpv's `set`/`cycle` coerce the string values).
    let res = match cmd {
        Command::Toggle => mpv_ipc::send_command(&sock, &["cycle", "pause"]),
        Command::Play => mpv_ipc::send_command(&sock, &["set", "pause", "no"]),
        Command::Pause => mpv_ipc::send_command(&sock, &["set", "pause", "yes"]),
        Command::Stop => mpv_ipc::send_command(&sock, &["stop"]),
        Command::Next => mpv_ipc::send_command(&sock, &["playlist-next"]),
        Command::Prev => mpv_ipc::send_command(&sock, &["playlist-prev"]),
        Command::Status => return true,
    };
    res.is_ok()
}
