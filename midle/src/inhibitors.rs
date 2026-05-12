//! Inhibitor tasks. Each periodically pushes its state into the
//! manager's mpsc channel; the manager flips the matching flag.
//!
//! Three sources:
//!   • `app_scan` — `/proc/<pid>/cmdline` regex match against
//!     `settings.inhibit_apps`.
//!   • `media_scan` — `pactl list sink-inputs` text parse; looks
//!     for `State: RUNNING`.
//!   • `logind_sleep` — D-Bus `PrepareForSleep` signal on the
//!     `org.freedesktop.login1.Manager` interface.

use crate::daemon::WaylandEvent;
use anyhow::{Context, Result};
use regex::Regex;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

// ── App inhibit ──────────────────────────────────────────────────────

pub async fn app_scan_loop(
    patterns: Vec<String>,
    interval: Duration,
    tx: mpsc::Sender<WaylandEvent>,
) {
    if patterns.is_empty() {
        return;
    }
    let regexes: Vec<Regex> = patterns
        .iter()
        .filter_map(|p| match Regex::new(p) {
            Ok(r) => Some(r),
            Err(e) => {
                warn!(pattern = %p, "skipping invalid inhibit_apps regex: {e}");
                None
            }
        })
        .collect();
    if regexes.is_empty() {
        return;
    }

    info!(count = regexes.len(), "app inhibit scan armed");
    let mut last_match: Option<String> = None;
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let new_match = tokio::task::spawn_blocking({
            let regexes = regexes.clone();
            move || scan_cmdlines(&regexes)
        })
        .await
        .unwrap_or(None);

        if new_match != last_match {
            last_match = new_match.clone();
            debug!(?new_match, "app inhibit changed");
            let _ = tx
                .send(WaylandEvent::AppInhibit(new_match))
                .await;
        }
    }
}

fn scan_cmdlines(regexes: &[Regex]) -> Option<String> {
    let procs = procfs::process::all_processes().ok()?;
    for p in procs.flatten() {
        let cmdline = match p.cmdline() {
            Ok(parts) => parts.join(" "),
            Err(_) => continue,
        };
        if cmdline.is_empty() {
            continue;
        }
        for re in regexes {
            if re.is_match(&cmdline) {
                return Some(re.as_str().to_string());
            }
        }
    }
    None
}

// ── Media inhibit ────────────────────────────────────────────────────

pub async fn media_scan_loop(interval: Duration, tx: mpsc::Sender<WaylandEvent>) {
    info!("media inhibit scan armed (pactl)");
    let mut last_playing = false;
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let playing = match check_pactl_running().await {
            Ok(v) => v,
            Err(e) => {
                warn!("pactl probe failed: {e}");
                continue;
            }
        };
        if playing != last_playing {
            last_playing = playing;
            debug!(playing, "media inhibit changed");
            let _ = tx.send(WaylandEvent::MediaInhibit(playing)).await;
        }
    }
}

async fn check_pactl_running() -> Result<bool> {
    // `pactl list sink-inputs` — text output, includes `State: RUNNING`
    // lines for streams that are actively producing audio. Covers
    // both PulseAudio and PipeWire-pulse.
    let out = Command::new("pactl")
        .arg("list")
        .arg("sink-inputs")
        .output()
        .await
        .context("spawn pactl")?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .any(|l| l.trim().starts_with("State:") && l.contains("RUNNING")))
}

// ── Logind PrepareForSleep ───────────────────────────────────────────

pub async fn logind_loop(prepare_command: Option<String>, tx: mpsc::Sender<WaylandEvent>) {
    if let Err(e) = run_logind_loop(prepare_command, tx).await {
        warn!("logind loop ended: {e:#}");
    }
}

async fn run_logind_loop(
    prepare_command: Option<String>,
    tx: mpsc::Sender<WaylandEvent>,
) -> Result<()> {
    use futures::StreamExt;
    use zbus::{Connection, Proxy};

    let conn = Connection::system()
        .await
        .context("connect to system D-Bus")?;
    info!("logind PrepareForSleep listener armed");

    let proxy = Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .context("logind Manager proxy")?;

    let mut signals = proxy
        .receive_signal("PrepareForSleep")
        .await
        .context("subscribe PrepareForSleep")?;

    while let Some(msg) = signals.next().await {
        let about_to_sleep: bool = match msg.body().deserialize() {
            Ok(v) => v,
            Err(e) => {
                warn!("decode PrepareForSleep payload: {e}");
                continue;
            }
        };
        info!(about_to_sleep, "PrepareForSleep");
        if about_to_sleep && let Some(cmd) = prepare_command.as_deref() {
            crate::actions::spawn_shell("prepare-sleep", cmd);
        }
        let _ = tx.send(WaylandEvent::PrepareForSleep(about_to_sleep)).await;
    }
    Ok(())
}
