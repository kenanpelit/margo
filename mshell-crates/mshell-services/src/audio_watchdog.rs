//! Audio-backend health watchdog.
//!
//! `wayle-audio`'s PulseAudio backend (crates.io 0.1.2) has no reconnect. When
//! the PipeWire audio server restarts under it — e.g. `dcli sync` re-applying
//! the pipewire user services, or a transient drop during Bluetooth churn at
//! login — its libpulse context dies, but the backend task keeps running: its
//! mainloop never quits, so it goes on answering `mshellctl audio …` from a now
//! frozen device store while every write (volume / mute / switch) is a silent
//! no-op and the reactive `Property`s never update again. Symptom: the volume
//! OSD never shows and the volume keys look dead, while brightness keeps working
//! and `wpctl` still drives the sink — PipeWire is healthy, only the
//! mshell→wayle path is severed.
//!
//! Because the command channel stays alive (it answers from the stale store), a
//! command round-trip can't see the failure. The reliable signal is the audio
//! server **restarting** — its systemd `MainPID` changing out from under the
//! connection wayle opened at startup. We keep a command-channel probe too, for
//! the rarer case where the backend task genuinely exits.
//!
//! We can't reconnect in place: `audio_service()` is a process-global
//! `OnceLock` and dozens of live `.watch()` subscriptions point at its
//! `Property`s, and wayle exposes no API to re-spawn the backend on an existing
//! service. The cure is a fresh `mshell` (re-inits `AudioService` with a new
//! context). On a confirmed death this watchdog raises a single actionable
//! notification offering a one-click shell restart.

use std::time::Duration;

use tracing::{info, warn};
use wayle_audio::Error as AudioError;

use crate::{audio_service, tokio_rt};

/// Delay before the first probe — give the backend time to enumerate at least
/// one device (we need a device key to probe with) and settle past startup.
const STARTUP_GRACE: Duration = Duration::from_secs(10);

/// How often to probe the backend's liveness once running.
const PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// Consecutive failed command-channel probes before declaring the backend
/// dead via that (secondary) signal. Guards the brief window between the
/// backend task exiting and the command processor tearing down, plus any
/// one-off transient.
const DEATH_THRESHOLD: u32 = 2;

/// The PipeWire / PulseAudio user services whose restart severs wayle's
/// connection. A change in any one's `MainPID` means it was restarted.
const AUDIO_SERVICES: &[&str] = &[
    "pipewire-pulse.service",
    "pipewire.service",
    "pulseaudio.service",
];

/// Spawn the audio-backend health watchdog on the services runtime. Call once,
/// after [`crate::init_services`]. The task probes periodically and exits after
/// it has raised the restart notification once — the backend cannot self-heal,
/// so there is nothing left to watch.
pub fn spawn_audio_health_watchdog() {
    tokio_rt().spawn(async move {
        tokio::time::sleep(STARTUP_GRACE).await;

        // Snapshot the audio server's PIDs. wayle opened its libpulse
        // connection at startup; if any of these restarts later (a different
        // MainPID), that connection is dead with no way back.
        // Run the blocking `systemctl` probe on the blocking pool, not on
        // one of the (few) shared async workers this runtime has.
        let baseline_pids = tokio::task::spawn_blocking(audio_service_pids)
            .await
            .unwrap_or_default();

        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::time::sleep(PROBE_INTERVAL).await;

            // Primary signal: did the audio server restart under us? This is the
            // real failure mode — wayle keeps answering from its frozen device
            // store after the context dies, so the command probe below can't see
            // it. A PID change is definitive, so it needs no transient guard.
            let current_pids = tokio::task::spawn_blocking(audio_service_pids)
                .await
                .unwrap_or_default();
            let restarted = audio_server_restarted(&baseline_pids, &current_pids);

            // Secondary signal: the command channel itself went away (the
            // backend task actually exited). `CommandChannelDisconnected` =
            // send failed / responder dropped; `Ok` / `DeviceNotFound` = alive.
            let channel_dead = {
                let svc = audio_service();
                match svc
                    .default_output
                    .get()
                    .map(|d| d.key)
                    .or_else(|| svc.output_devices.get().first().map(|d| d.key))
                {
                    Some(key) => matches!(
                        svc.output_device(key).await,
                        Err(AudioError::CommandChannelDisconnected)
                    ),
                    None => false,
                }
            };

            if channel_dead {
                consecutive_failures += 1;
            } else {
                consecutive_failures = 0;
            }

            if restarted || consecutive_failures >= DEATH_THRESHOLD {
                warn!(
                    restarted,
                    "audio backend unresponsive (wayle-audio lost its PulseAudio \
                     connection and cannot reconnect); notifying for a shell restart"
                );
                notify_backend_dead();
                return;
            }
        }
    });
}

/// `MainPID` of each [`AUDIO_SERVICES`] unit, as systemd reports it (one entry
/// per service, in order; `"0"` = not running / absent). Empty on query
/// failure, which callers treat as "no change" so a transient `systemctl`
/// hiccup can't trip a false alarm.
fn audio_service_pids() -> Vec<String> {
    let mut args = vec!["--user", "show", "--value", "-p", "MainPID"];
    args.extend_from_slice(AUDIO_SERVICES);
    std::process::Command::new("systemctl")
        .args(&args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// True if any service that was running at `baseline` (a non-zero PID) now
/// reports a different `MainPID` — i.e. it restarted (or stopped). Services
/// that weren't running then are ignored, so a non-PipeWire setup never trips
/// this. A short/empty `current` (query failure) is treated as no change.
fn audio_server_restarted(baseline: &[String], current: &[String]) -> bool {
    if baseline.is_empty() || baseline.len() != current.len() {
        return false;
    }
    baseline
        .iter()
        .zip(current)
        .any(|(base, cur)| base != "0" && !base.is_empty() && base != cur)
}

/// Raise a persistent, critical notification offering a one-click shell
/// restart. `notify_rust`'s action wait blocks, so it is parked on a dedicated
/// OS thread; the thread ends when the user acts on or dismisses the toast.
fn notify_backend_dead() {
    std::thread::spawn(|| {
        let handle = match notify_rust::Notification::new()
            .summary("Audio control unresponsive")
            .body(
                "The audio service lost its connection — volume keys and the \
                 on-screen display stopped working. Restart the shell to fix it.",
            )
            .icon("audio-volume-muted-symbolic")
            .hint(notify_rust::Hint::Urgency(notify_rust::Urgency::Critical))
            // "default" makes tapping the toast itself restart; "restart" is the
            // labelled button shown in the notification centre. Same handler.
            .action("default", "Restart shell")
            .action("restart", "Restart shell")
            .timeout(notify_rust::Timeout::Never)
            .show()
        {
            Ok(handle) => handle,
            Err(err) => {
                warn!(?err, "audio watchdog: failed to show restart notification");
                return;
            }
        };

        handle.wait_for_action(|action| {
            if action == "default" || action == "restart" {
                info!("audio watchdog: user accepted shell restart");
                if let Err(err) = std::process::Command::new("systemctl")
                    .args(["--user", "restart", "mshell"])
                    .spawn()
                {
                    warn!(?err, "audio watchdog: failed to restart mshell");
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::audio_server_restarted;

    fn pids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn restarted_running_service_is_detected() {
        // pipewire-pulse went 100 -> 200 (restart); the rest unchanged.
        assert!(audio_server_restarted(
            &pids(&["100", "5", "0"]),
            &pids(&["200", "5", "0"])
        ));
    }

    #[test]
    fn a_running_service_stopping_counts() {
        // nonzero -> 0: it went away, the connection is dead either way.
        assert!(audio_server_restarted(
            &pids(&["100", "5", "0"]),
            &pids(&["0", "5", "0"])
        ));
    }

    #[test]
    fn services_not_running_at_baseline_are_ignored() {
        // An absent ("0") service later getting a PID is not a restart of a
        // connection we ever depended on (e.g. PulseAudio on a PipeWire box).
        assert!(!audio_server_restarted(
            &pids(&["100", "5", "0"]),
            &pids(&["100", "5", "999"])
        ));
    }

    #[test]
    fn stable_pids_are_not_a_restart() {
        assert!(!audio_server_restarted(
            &pids(&["100", "5", "0"]),
            &pids(&["100", "5", "0"])
        ));
    }

    #[test]
    fn query_failure_is_treated_as_no_change() {
        // Empty `current` (a `systemctl` hiccup) must not raise a false alarm,
        // and an empty baseline (non-systemd / failed at start) never trips.
        assert!(!audio_server_restarted(
            &pids(&["100", "5", "0"]),
            &pids(&[])
        ));
        assert!(!audio_server_restarted(
            &pids(&[]),
            &pids(&["100", "5", "0"])
        ));
    }
}
