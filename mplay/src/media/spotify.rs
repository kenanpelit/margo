//! Spotify autostart: when Spotify is the explicit target but not yet a
//! live MPRIS player, launch it and wait (bounded) for it to appear.

use super::status::Command;
use super::{mpris, notify};
use std::process::{Command as Proc, Stdio};
use std::thread::sleep;
use std::time::Duration;

fn find_spotify() -> Option<String> {
    mpris::list()
        .into_iter()
        .find(|p| p.to_ascii_lowercase().contains("spotify"))
}

fn have(tool: &str) -> bool {
    Proc::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn running() -> bool {
    Proc::new("pgrep")
        .args(["-x", "spotify"])
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Resolve a live Spotify MPRIS player, autostarting Spotify if needed for
/// a control/status command. Returns the player name, or `None`.
pub fn ensure_ready(cmd: Command) -> Option<String> {
    if let Some(p) = find_spotify() {
        return Some(p);
    }
    // Only autostart for commands that make sense on a cold player.
    if !matches!(
        cmd,
        Command::Toggle | Command::Play | Command::Next | Command::Prev | Command::Status
    ) {
        return None;
    }
    if !have("spotify") {
        return None;
    }

    if !running() {
        notify::info(
            "Spotify · Başlatılıyor",
            "Spotify açılıyor, hazır olunca komut gönderilecek.",
            "spotify",
        );
        let _ = Proc::new("spotify")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    let timeout_s: u64 = std::env::var("SPOTIFY_START_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    for _ in 0..(timeout_s * 4) {
        if let Some(p) = find_spotify() {
            return Some(p);
        }
        sleep(Duration::from_millis(250));
    }
    find_spotify()
}
