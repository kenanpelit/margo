//! Rich media notifications via `notify-send` (album art when available).

use super::Meta;
use super::status::{Command, Status};
use std::process::Command as Proc;

const SYNC_ID: &str = "string:x-canonical-private-synchronous:osc-media";

fn have_notify() -> bool {
    Proc::new("sh")
        .args(["-c", "command -v notify-send"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn send(title: &str, body: &str, icon: &str, urgency: &str) {
    if !have_notify() {
        return;
    }
    let _ = Proc::new("notify-send")
        .args([
            "-a",
            "osc-media",
            "-u",
            urgency,
            "-t",
            "3200",
            "-h",
            SYNC_ID,
            "-i",
            icon,
            title,
            body,
        ])
        .status();
}

/// Friendly player name for the notification title.
pub fn player_pretty(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for (k, v) in [
        ("spotify", "Spotify"),
        ("vlc", "VLC"),
        ("mpv", "MPV"),
        ("mpd", "MPD"),
        ("firefox", "Firefox"),
        ("chromium", "Chromium"),
        ("chrome", "Chrome"),
        ("brave", "Brave"),
        ("zen", "Zen"),
        ("vivaldi", "Vivaldi"),
        ("edge", "Edge"),
    ] {
        if lower.starts_with(k) {
            return v.to_string();
        }
    }
    // Fallback: first dotted segment, title-cased.
    let head = name
        .split('.')
        .next()
        .unwrap_or(name)
        .replace(['_', '-'], " ");
    let mut c = head.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => name.to_string(),
    }
}

fn player_icon(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let icon = if lower.starts_with("spotify") {
        "spotify"
    } else if lower.starts_with("vlc") {
        "vlc"
    } else if lower.starts_with("mpv") {
        "mpv"
    } else if lower.starts_with("mpd") {
        "audio-x-generic"
    } else if lower.starts_with("firefox") {
        "firefox"
    } else if lower.starts_with("chromium") || lower.starts_with("chrome") {
        "chromium"
    } else if lower.starts_with("brave") {
        "brave-browser"
    } else if lower.starts_with("vivaldi") {
        "vivaldi"
    } else if lower.starts_with("edge") {
        "microsoft-edge"
    } else {
        "audio-x-generic"
    };
    icon.to_string()
}

/// Album-art file (from a `file://` artUrl) if it exists, else a themed
/// per-player icon name.
fn resolve_icon(art_url: &str, name: &str) -> String {
    if let Some(path) = art_url.strip_prefix("file://") {
        let path = path.replace("%20", " ");
        if std::path::Path::new(&path).is_file() {
            return path;
        }
    } else if art_url.starts_with('/') && std::path::Path::new(art_url).is_file() {
        return art_url.to_string();
    }
    player_icon(name)
}

/// The post-command media notification.
pub fn media(name: &str, cmd: Command, status: Status, meta: &Meta) {
    let pretty = player_pretty(name);
    let title = if cmd.is_status() {
        format!("{pretty} · {}", status.label())
    } else {
        format!("{pretty} · {}", cmd.label())
    };
    let mut body = format!("Durum: {}", status.label());
    if !meta.title.is_empty() {
        body.push_str(&format!("\nParça: {}", meta.title));
    }
    if !meta.artist.is_empty() {
        body.push_str(&format!("\nSanatçı: {}", meta.artist));
    }
    if !meta.album.is_empty() {
        body.push_str(&format!("\nAlbüm: {}", meta.album));
    }
    send(&title, &body, &resolve_icon(&meta.art_url, name), "normal");
}

/// An informational popup (e.g. "Spotify starting").
pub fn info(title: &str, body: &str, icon: &str) {
    send(title, body, icon, "normal");
}

/// A failure popup.
pub fn error(body: &str) {
    send("Medya kontrolü", body, "dialog-error", "critical");
}
