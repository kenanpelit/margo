//! Audio-backend health watchdog.
//!
//! `wayle-audio`'s PulseAudio backend (crates.io 0.1.2) has no reconnect: if
//! the libpulse context drops once — e.g. a transient pipewire-pulse
//! disconnect during Bluetooth-audio churn at login — `PulseBackend::run`
//! breaks out of its loop and the backend task ends for good. From then on
//! the command processor is torn down, so every `mshellctl audio …` and every
//! volume-key dispatch is a silent no-op (the command is accepted and answered
//! `Ok`, then dropped), and the reactive `Property`s freeze at their last
//! snapshot. Symptom cluster: the volume OSD never shows and the volume keys
//! look dead, while brightness keeps working and `wpctl` still drives the sink
//! fine — PipeWire is healthy, only the mshell→wayle path is severed.
//!
//! We can't reconnect in place: `audio_service()` is a process-global
//! `OnceLock` and dozens of live `.watch()` subscriptions point at its
//! `Property`s, and wayle exposes no API to re-spawn the backend on an existing
//! service. The cure is a fresh `mshell` (re-inits `AudioService` with a new
//! context). This watchdog probes the backend and, on a confirmed death, raises
//! a single actionable notification offering a one-click shell restart.

use std::time::Duration;

use tracing::{info, warn};
use wayle_audio::Error as AudioError;

use crate::{audio_service, tokio_rt};

/// Delay before the first probe — give the backend time to enumerate at least
/// one device (we need a device key to probe with) and settle past startup.
const STARTUP_GRACE: Duration = Duration::from_secs(10);

/// How often to probe the backend's liveness once running.
const PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// Consecutive failed probes before declaring the backend dead. Guards the
/// microsecond window between `run()` exiting and the command processor
/// tearing down, plus any one-off transient.
const DEATH_THRESHOLD: u32 = 2;

/// Spawn the audio-backend health watchdog on the services runtime. Call once,
/// after [`crate::init_services`]. The task probes periodically and exits after
/// it has raised the restart notification once — the backend cannot self-heal,
/// so there is nothing left to watch.
pub fn spawn_audio_health_watchdog() {
    tokio_rt().spawn(async move {
        tokio::time::sleep(STARTUP_GRACE).await;

        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::time::sleep(PROBE_INTERVAL).await;

            let svc = audio_service();

            // A device key to probe with: the (possibly frozen) default output,
            // else any known output. `None` → no device enumerated yet, so
            // there is nothing meaningful to probe this tick.
            let Some(key) = svc
                .default_output
                .get()
                .map(|d| d.key)
                .or_else(|| svc.output_devices.get().first().map(|d| d.key))
            else {
                continue;
            };

            // The probe round-trips a `GetDevice` through the backend command
            // channel. `CommandChannelDisconnected` means the command loop is
            // gone (send failed / responder dropped) → backend dead. `Ok` or
            // `DeviceNotFound` both mean the loop answered → alive.
            let dead = matches!(
                svc.output_device(key).await,
                Err(AudioError::CommandChannelDisconnected)
            );

            if !dead {
                consecutive_failures = 0;
                continue;
            }

            consecutive_failures += 1;
            if consecutive_failures >= DEATH_THRESHOLD {
                warn!(
                    "audio backend unresponsive (wayle-audio lost its PulseAudio \
                     connection and cannot reconnect); notifying for a shell restart"
                );
                notify_backend_dead();
                return;
            }
        }
    });
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
