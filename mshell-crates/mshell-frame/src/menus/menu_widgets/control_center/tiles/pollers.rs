//! The background watchers and pollers the tile grid starts on its first
//! reveal. Every poller waits on the shared [`RevealGate`], so a closed
//! control center neither wakes nor spawns `mctl` / `systemctl` / `podman`.

use super::tile_probes::{
    podman_machines_cached, podman_tile_state, read_disk_usage, read_mirror_tile_state,
    read_ufw_tile_state, twilight_subtitle, valent_state_from_report,
};
use super::{ControlCenterTilesCommandOutput, ControlCenterTilesModel};
use crate::menus::reveal_poll::RevealGate;
use mshell_utils::battery::{spawn_battery_online_watcher, spawn_battery_watcher};
use mshell_utils::denoise;
use mshell_utils::hotspot;
use mshell_utils::idle::spawn_idle_inhibitor_watcher;
use mshell_utils::notifications::spawn_dnd_watcher;
use relm4::ComponentSender;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Slower cadence for the shell-out probes (UFW / Podman) which change
/// rarely and cost a subprocess each.
const SLOW_POLL_INTERVAL: Duration = Duration::from_secs(10);
const STARTUP_DELAY: Duration = Duration::from_millis(200);

/// Start the tile watchers and pollers (once, on the first reveal).
pub(super) fn start_watchers(
    model: &ControlCenterTilesModel,
    sender: &ComponentSender<ControlCenterTilesModel>,
) {
    // Keep Awake watcher
    spawn_idle_inhibitor_watcher(sender, || ControlCenterTilesCommandOutput::KeepAwakeChanged);

    // DND watcher
    spawn_dnd_watcher(sender, || ControlCenterTilesCommandOutput::DndChanged);

    // Battery watchers
    spawn_battery_watcher(sender, || ControlCenterTilesCommandOutput::BatteryChanged);
    spawn_battery_online_watcher(sender, || ControlCenterTilesCommandOutput::BatteryChanged);

    // Twilight poller — reads the full status so the tile can
    // show the active profile (mode + temperature), not just
    // an on/off flag. Skips probing while the panel is hidden.
    let revealed = model.revealed.clone();
    sender.command(move |out, shutdown| async move {
        use crate::twilight;
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            let delay = if first { STARTUP_DELAY } else { POLL_INTERVAL };
            first = false;
            tokio::select! {
                () = &mut shutdown_fut => break,
                () = tile_tick(&revealed, delay) => {}
            }
            if let Some(status) = twilight::probe().await {
                let subtitle = twilight_subtitle(&status);
                let _ = out.send(ControlCenterTilesCommandOutput::TwilightChanged(
                    status.enabled,
                    subtitle,
                ));
            }
            // Mirror rides the same cadence: one cheap IPC
            // `get`, and it can end on its own (unplug).
            let (sub, active) = read_mirror_tile_state().await;
            let _ = out.send(ControlCenterTilesCommandOutput::MirrorRefreshed(
                sub, active,
            ));
        }
    });

    // Firewall (UFW) poller — runs on the slow cadence and
    // only while the panel is visible (the `revealed` gate).
    let revealed = model.revealed.clone();
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            let delay = if first {
                STARTUP_DELAY
            } else {
                SLOW_POLL_INTERVAL
            };
            first = false;
            tokio::select! {
                () = &mut shutdown_fut => break,
                () = tile_tick(&revealed, delay) => {}
            }
            let (ufw_sub, ufw_active) = read_ufw_tile_state().await;
            let _ = out.send(ControlCenterTilesCommandOutput::UfwRefreshed(
                ufw_sub, ufw_active,
            ));
        }
    });

    // Podman tile: follows the shared poll (re-evaluated on
    // every change it reports), machines cached for a minute.
    let mut podman_rx = model.podman_poll.receiver();
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            let summary = podman_rx.borrow_and_update().clone();
            if let Some(summary) = summary {
                let machines = podman_machines_cached().await;
                let (sub, active) = podman_tile_state(machines.as_deref(), &summary);
                let _ = out.send(ControlCenterTilesCommandOutput::PodmanRefreshed(
                    sub, active,
                ));
            }
            tokio::select! {
                () = &mut shutdown_fut => break,
                r = podman_rx.changed() => if r.is_err() { break },
            }
        }
    });

    // Disk refresh, plus the hotspot (nmcli) and noise
    // suppression (pactl) probes: once on reveal, then every
    // 30 s while the panel is visible. Both tiles also
    // re-probe right after their own toggle.
    let revealed = model.revealed.clone();
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            let delay = if first {
                Duration::ZERO
            } else {
                Duration::from_secs(30)
            };
            first = false;
            tokio::select! {
                () = &mut shutdown_fut => break,
                () = tile_tick(&revealed, delay) => {}
            }
            let usage = read_disk_usage();
            let _ = out.send(ControlCenterTilesCommandOutput::DiskRefreshed(usage));
            let _ = out.send(ControlCenterTilesCommandOutput::HotspotRefreshed(
                hotspot::probe(&hotspot::configured_interface()).await,
            ));
            let _ = out.send(ControlCenterTilesCommandOutput::DenoiseRefreshed(
                denoise::probe().await,
            ));
        }
    });

    // Subtitle poller for Wi-Fi / BT / VPN / audio tiles
    // (every 5s while the panel is visible)
    let revealed = model.revealed.clone();
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            let delay = if first { STARTUP_DELAY } else { POLL_INTERVAL };
            first = false;
            tokio::select! {
                () = &mut shutdown_fut => break,
                () = tile_tick(&revealed, delay) => {}
            }
            let _ = out.send(ControlCenterTilesCommandOutput::SubtitlesRefreshed);
        }
    });

    // Valent tile: follows the shared Valent poll.
    let mut valent_rx = model.valent_poll.receiver();
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            let report = valent_rx.borrow_and_update().clone();
            if let Some(report) = report {
                let (subtitle, connected) = valent_state_from_report(&report);
                let _ = out.send(ControlCenterTilesCommandOutput::ValentStateRefreshed(
                    subtitle, connected,
                ));
            }
            tokio::select! {
                () = &mut shutdown_fut => break,
                r = valent_rx.changed() => if r.is_err() { break },
            }
        }
    });
}

/// Wait out `period` of revealed time (no timer while hidden). Reopening
/// the panel re-probes after [`STARTUP_DELAY`], like the first open, so
/// the probes don't race the reveal animation.
async fn tile_tick(revealed: &RevealGate, period: Duration) {
    if revealed.tick(period).await {
        tokio::time::sleep(STARTUP_DELAY).await;
    }
}
