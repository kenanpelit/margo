//! MPRIS player state via `playerctl`.
//!
//! Stage-5 implementation. playerctl picks the "best" running player
//! (the one with the most recent activity) automatically, which is
//! the behaviour the eww config relied on too. A native zbus
//! MPRIS2 client is on the cards for later — for now this keeps the
//! dep tree light and lets us iterate the UI without binding to a
//! specific player.

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    /// `mpris:artUrl` from the player's metadata. May be a
    /// `file://`, `https://` or empty string. Stage-10: only
    /// `file://` paths are honoured by the GtkImage loader; remote
    /// HTTPS art is queued for a follow-up patch that adds an
    /// async fetcher.
    pub art_url: String,
}

pub fn current() -> Option<Snapshot> {
    let status = run_playerctl(&["status"])?;
    let status = status.trim();
    // "No players found" prints to stderr; success + empty stdout
    // means no metadata, just bail.
    if status.is_empty() {
        return None;
    }
    let playing = status == "Playing";
    let title = run_playerctl(&["metadata", "title"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let artist = run_playerctl(&["metadata", "artist"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let art_url = run_playerctl(&["metadata", "mpris:artUrl"])
        .unwrap_or_default()
        .trim()
        .to_string();
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    Some(Snapshot {
        playing,
        title,
        artist,
        art_url,
    })
}

fn run_playerctl(args: &[&str]) -> Option<String> {
    let out = Command::new("playerctl")
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn play_pause() {
    let _ = Command::new("playerctl")
        .arg("play-pause")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn next() {
    let _ = Command::new("playerctl")
        .arg("next")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn previous() {
    let _ = Command::new("playerctl")
        .arg("previous")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
