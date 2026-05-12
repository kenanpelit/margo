//! Audio volume / mute via `wpctl` (WirePlumber CLI).
//!
//! Stage-4 implementation. `wpctl` is the standard front-end for
//! PipeWire and ships with WirePlumber, which is the default audio
//! session manager on Wayland setups including this one. A future
//! stage may switch to pipewire-rs for an event-driven feed, but
//! polling at 1 Hz is fine for a bar slider and keeps the dep tree
//! tiny.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// 0..=100.
    pub volume_percent: u8,
    pub muted: bool,
}

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

pub fn current() -> Option<Snapshot> {
    let out = Command::new("wpctl")
        .args(["get-volume", SINK])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    // wpctl prints either "Volume: 0.50" or "Volume: 0.50 [MUTED]".
    let mut parts = s.split_whitespace();
    let _label = parts.next()?; // "Volume:"
    let frac: f64 = parts.next()?.parse().ok()?;
    let muted = s.contains("[MUTED]");
    Some(Snapshot {
        volume_percent: (frac * 100.0).round().clamp(0.0, 150.0) as u8,
        muted,
    })
}

pub fn set_volume(percent: u8) {
    let pct = percent.min(150);
    let _ = Command::new("wpctl")
        .args(["set-volume", SINK, &format!("{}%", pct)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn toggle_mute() {
    let _ = Command::new("wpctl")
        .args(["set-mute", SINK, "toggle"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Microphone (default audio source) snapshot. Same wpctl interface
/// as the sink, just keyed off `@DEFAULT_AUDIO_SOURCE@`.
pub fn source_current() -> Option<Snapshot> {
    let out = Command::new("wpctl")
        .args(["get-volume", SOURCE])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut parts = s.split_whitespace();
    let _label = parts.next()?;
    let frac: f64 = parts.next()?.parse().ok()?;
    let muted = s.contains("[MUTED]");
    Some(Snapshot {
        volume_percent: (frac * 100.0).round().clamp(0.0, 150.0) as u8,
        muted,
    })
}

pub fn source_set_volume(percent: u8) {
    let pct = percent.min(150);
    let _ = Command::new("wpctl")
        .args(["set-volume", SOURCE, &format!("{}%", pct)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn source_toggle_mute() {
    let _ = Command::new("wpctl")
        .args(["set-mute", SOURCE, "toggle"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
