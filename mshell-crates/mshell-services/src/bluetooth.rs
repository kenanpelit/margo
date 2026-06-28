//! Native Bluetooth auto-connect + audio routing.
//!
//! Replaces the external `bt-autoconnect.service` + `bt-autoconnect-once`
//! + `bluetooth_toggle` scripts. Driven by `BluetoothConfig`:
//!
//! - At login, [`spawn_autoconnect_startup`] waits the configured delay
//!   then tries each device in order (with a few retries) until one
//!   connects, and routes audio to it.
//! - [`toggle`] / [`connect_configured`] / [`disconnect_configured`] back
//!   the `mshellctl bluetooth …` verbs and the Settings "Toggle now"
//!   button (bind the verb to F10 to replace the old toggle key).
//!
//! Everything runs on the shared wayle [`tokio_rt`](crate::tokio_rt),
//! the same runtime that owns the wayle service singletons.

use crate::{audio_service, bluetooth_service, tokio_rt};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{BluetoothConfig, ConfigStoreFields};
use reactive_graph::traits::GetUntracked;
use std::sync::Arc;
use std::time::Duration;
use wayle_bluetooth::core::device::Device;

/// Connect attempts at login before giving up (matches the old
/// `bt-autoconnect-once` default).
const STARTUP_ATTEMPTS: u32 = 4;
/// Delay between failed login connect attempts.
const STARTUP_RETRY_DELAY: Duration = Duration::from_secs(8);
/// How long to wait for `connected = true` after issuing connect.
const CONNECT_WAIT: Duration = Duration::from_secs(12);
/// How long to wait for the adapter to report powered after enabling it.
const ADAPTER_WAIT: Duration = Duration::from_secs(5);
/// How long to wait for the bluez PipeWire node to appear before routing.
const AUDIO_NODE_WAIT: Duration = Duration::from_secs(8);

/// Snapshot the current Bluetooth config (untracked — we're off the
/// reactive graph here).
fn config() -> BluetoothConfig {
    config_manager().config().bluetooth().get_untracked()
}

/// Normalise a MAC for matching: uppercase, `:`/`-` → `_`. bluez PipeWire
/// nodes embed the address this way (`bluez_output.F4_9D_8A_3D_CB_30.1`).
fn mac_underscored(mac: &str) -> String {
    mac.trim().to_ascii_uppercase().replace([':', '-'], "_")
}

/// Colon-form, uppercased — the form bluez D-Bus + `connected` report.
fn mac_colon(mac: &str) -> String {
    mac.trim().to_ascii_uppercase().replace('-', ":")
}

/// Find a discovered/paired device by MAC (case-insensitive).
fn find_device(mac: &str) -> Option<Arc<Device>> {
    let want = mac_colon(mac);
    bluetooth_service()?
        .devices
        .get()
        .into_iter()
        .find(|d| d.address.get().to_ascii_uppercase() == want)
}

/// True if any configured device currently reports connected.
fn any_configured_connected(cfg: &BluetoothConfig) -> bool {
    let Some(bt) = bluetooth_service() else {
        return false;
    };
    let connected: Vec<String> = bt
        .connected
        .get()
        .into_iter()
        .map(|a| a.to_ascii_uppercase())
        .collect();
    cfg.devices
        .iter()
        .any(|d| connected.contains(&mac_colon(&d.mac)))
}

/// Fire-and-forget notification, gated on the config flag. Uses the same
/// synchronous-replace hint as the audio toasts so repeats don't stack.
fn notify(cfg: &BluetoothConfig, summary: &str, body: &str) {
    if !cfg.notifications {
        return;
    }
    let summary = summary.to_string();
    let body = body.to_string();
    tokio_rt().spawn(async move {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                "bluetooth-symbolic",
                "-h",
                "string:x-canonical-private-synchronous:mshell-bluetooth",
                &summary,
                &body,
            ])
            .status()
            .await;
    });
}

/// Power the adapter on if it is off, then wait (bounded) for it to report
/// enabled. Returns whether an adapter is usable.
async fn ensure_adapter_on() -> bool {
    let Some(bt) = bluetooth_service() else {
        return false;
    };
    if !bt.available.get() {
        return false;
    }
    if bt.enabled.get() {
        return true;
    }
    if bt.enable().await.is_err() {
        return false;
    }
    let probe = bt.clone();
    wait_until(ADAPTER_WAIT, move || probe.enabled.get()).await
}

/// Poll `cond` until true or `timeout` elapses (250 ms granularity).
async fn wait_until(timeout: Duration, cond: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Issue connect to a device and wait for it to report connected.
async fn connect_device(dev: &Arc<Device>) -> bool {
    // Trust so bluez reconnects without prompting; best-effort.
    let _ = dev.set_trusted(true).await;
    if dev.connect().await.is_err() {
        return false;
    }
    let addr = dev.address.get();
    wait_until(CONNECT_WAIT, move || {
        find_device(&addr)
            .map(|d| d.connected.get())
            .unwrap_or(false)
    })
    .await
}

/// Route the default audio output (and optionally input) to the bluez node
/// belonging to `mac`. Waits briefly for the PipeWire node to show up.
async fn route_audio(cfg: &BluetoothConfig, mac: &str) {
    if !cfg.route_audio_output && !cfg.route_audio_input {
        return;
    }
    let needle = mac_underscored(mac);
    let colon = mac_colon(mac);

    if cfg.route_audio_output {
        wait_until(AUDIO_NODE_WAIT, || {
            audio_service()
                .output_devices
                .get()
                .iter()
                .any(|d| device_matches(&d.name.get(), &d.properties.get(), &needle, &colon))
        })
        .await;
        if let Some(dev) = audio_service()
            .output_devices
            .get()
            .into_iter()
            .find(|d| device_matches(&d.name.get(), &d.properties.get(), &needle, &colon))
            && dev.set_as_default().await.is_ok()
        {
            notify(
                cfg,
                "Bluetooth audio",
                &format!("Output → {}", dev.description.get()),
            );
        }
    }

    if cfg.route_audio_input
        && let Some(dev) = audio_service()
            .input_devices
            .get()
            .into_iter()
            .find(|d| device_matches(&d.name.get(), &d.properties.get(), &needle, &colon))
        && dev.set_as_default().await.is_ok()
    {
        notify(
            cfg,
            "Bluetooth audio",
            &format!("Input → {}", dev.name.get()),
        );
    }
}

/// Does this PipeWire node belong to the target MAC? Matches the
/// underscored MAC in the node name, or the colon MAC in any bluez
/// property value (`api.bluez5.address`).
fn device_matches(
    name: &str,
    props: &std::collections::HashMap<String, String>,
    needle_underscored: &str,
    needle_colon: &str,
) -> bool {
    if name.to_ascii_uppercase().contains(needle_underscored) {
        return true;
    }
    props
        .values()
        .any(|v| v.to_ascii_uppercase() == *needle_colon)
}

/// Try the configured devices in order; first one that connects wins, and
/// audio is routed to it. Powers the adapter on first if needed.
pub async fn connect_configured() -> bool {
    let cfg = config();
    if cfg.devices.is_empty() {
        return false;
    }
    if !ensure_adapter_on().await {
        notify(&cfg, "Bluetooth", "No usable adapter");
        return false;
    }
    for d in &cfg.devices {
        let Some(dev) = find_device(&d.mac) else {
            continue;
        };
        let label = if d.name.is_empty() {
            d.mac.clone()
        } else {
            d.name.clone()
        };
        if connect_device(&dev).await {
            notify(&cfg, "Bluetooth connected", &label);
            route_audio(&cfg, &d.mac).await;
            return true;
        }
    }
    false
}

/// Disconnect every currently-connected configured device.
pub async fn disconnect_configured() -> bool {
    let cfg = config();
    let mut any = false;
    for d in &cfg.devices {
        if let Some(dev) = find_device(&d.mac)
            && dev.connected.get()
            && dev.disconnect().await.is_ok()
        {
            any = true;
            let label = if d.name.is_empty() {
                d.mac.clone()
            } else {
                d.name.clone()
            };
            notify(&cfg, "Bluetooth disconnected", &label);
        }
    }
    any
}

/// Smart toggle (F10 replacement):
/// - adapter off → power on + connect the configured device(s)
/// - on + a configured device connected → disconnect it
/// - on + nothing connected → connect
pub async fn toggle() {
    let cfg = config();
    let Some(bt) = bluetooth_service() else {
        notify(&cfg, "Bluetooth", "No adapter");
        return;
    };
    if !bt.available.get() {
        notify(&cfg, "Bluetooth", "No adapter");
        return;
    }
    if !bt.enabled.get() {
        connect_configured().await;
        return;
    }
    if any_configured_connected(&cfg) {
        disconnect_configured().await;
    } else {
        connect_configured().await;
    }
}

/// At login: if auto-connect is on and devices are configured, spawn a
/// bounded retry loop (delay first, then a few attempts) on the wayle
/// runtime. One-shot — toggling config applies on the next start / via
/// the `mshellctl bluetooth` verbs.
pub fn spawn_autoconnect_startup() {
    let cfg = config();
    if !cfg.autoconnect_enabled || cfg.devices.is_empty() {
        return;
    }
    tokio_rt().spawn(async move {
        tokio::time::sleep(Duration::from_secs(cfg.autoconnect_delay_secs as u64)).await;
        for attempt in 1..=STARTUP_ATTEMPTS {
            if connect_configured().await {
                return;
            }
            if attempt < STARTUP_ATTEMPTS {
                tracing::info!(
                    "bluetooth autoconnect: attempt {attempt}/{STARTUP_ATTEMPTS} failed; \
                     retrying in {}s",
                    STARTUP_RETRY_DELAY.as_secs()
                );
                tokio::time::sleep(STARTUP_RETRY_DELAY).await;
            }
        }
        tracing::warn!("bluetooth autoconnect: gave up after {STARTUP_ATTEMPTS} attempts");
    });
}
