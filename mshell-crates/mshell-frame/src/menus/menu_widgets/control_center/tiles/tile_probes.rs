//! Service-state probes for the control-center tiles: the pure/async helpers
//! that read live system state (battery, disk, twilight, UFW, podman, Wi-Fi,
//! Bluetooth, audio, VPN, Valent, airplane) into the `(subtitle, is_active)`
//! shape the tile widgets render. Kept separate from `tiles.rs` so the widget
//! wiring stays readable and these I/O helpers are independently reviewable.

use super::*;

pub(super) fn read_battery() -> BatterySnapshot {
    let service = battery_service();
    let dev = &service.device;
    let present = dev.is_present.get();
    if !present {
        return BatterySnapshot::default();
    }
    let percent = dev.percentage.get().round().clamp(0.0, 100.0) as u8;
    let state = dev.state.get();
    let on_ac = line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(false);
    BatterySnapshot {
        present,
        percent,
        state,
        on_ac,
    }
}

pub(super) fn read_disk_usage() -> DiskUsage {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path = CString::new("/").unwrap();
    let mut stat: MaybeUninit<libc::statvfs64> = MaybeUninit::uninit();
    let rc = unsafe { libc::statvfs64(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return DiskUsage::default();
    }
    let s = unsafe { stat.assume_init() };
    let block = s.f_frsize;
    let total = s.f_blocks * block;
    let avail = s.f_bavail * block;
    let used = total.saturating_sub(avail);
    DiskUsage {
        used_bytes: used,
        total_bytes: total,
    }
}

pub(super) fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

// ── Twilight helpers ───────────────────────────────────────────────────────

/// Flip the twilight filter (right-click quick toggle), settle briefly,
/// then re-probe so the caller can refresh the tile to the new state.
pub(super) async fn toggle_twilight_and_probe() -> Option<(bool, String)> {
    let _ = tokio::process::Command::new("mctl")
        .args(["twilight", "toggle"])
        .status()
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let status = crate::twilight::probe().await?;
    Some((status.enabled, twilight_subtitle(&status)))
}

/// The tile subtitle = the active twilight profile. Off → "Off"; otherwise
/// the source mode plus the current colour temperature (e.g.
/// "Schedule · 3500K"), falling back to just the mode when margo isn't
/// reporting a live temperature yet.
pub(super) fn twilight_subtitle(s: &crate::twilight::TwilightStatus) -> String {
    if !s.enabled {
        return "Off".to_string();
    }
    let mode = twilight_mode_label(&s.mode);
    match s.current_temp_k {
        Some(k) => format!("{mode} · {k}K"),
        None => mode.to_string(),
    }
}

/// Source-mode id → human label (mirrors the Twilight menu's selector).
pub(super) fn twilight_mode_label(mode: &str) -> &'static str {
    match mode {
        "geo" => "Auto",
        "manual" => "Manual",
        "static" => "Static",
        "schedule" => "Schedule",
        _ => "On",
    }
}

// ── Firewall (UFW) helpers ──────────────────────────────────────────────────

/// (subtitle, is_active). Active = UFW enabled. Unprivileged read via the
/// shared bar-pill helper (`systemctl is-active ufw.service`).
pub(super) async fn read_ufw_tile_state() -> (String, bool) {
    let summary = fetch_ufw_status_only().await;
    match summary.status {
        Some(UfwStatus::Active) => ("Active".to_string(), true),
        Some(UfwStatus::Inactive) => ("Inactive".to_string(), false),
        _ => ("Unavailable".to_string(), false),
    }
}

// ── Podman helpers ──────────────────────────────────────────────────────────

struct PodmanMachine {
    name: String,
    running: bool,
}

/// Parse `podman machine list --format json`. `None` when podman is
/// missing or machines aren't supported on this host.
async fn podman_machines() -> Option<Vec<PodmanMachine>> {
    let out = tokio::process::Command::new("podman")
        .args(["machine", "list", "--format", "json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let arr = v.as_array()?;
    Some(
        arr.iter()
            .map(|m| PodmanMachine {
                name: m
                    .get("Name")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                running: m.get("Running").and_then(|x| x.as_bool()).unwrap_or(false),
            })
            .collect(),
    )
}

/// (subtitle, is_active). Prefers podman machine state (the user asked for
/// the active machine name); falls back to a running-container summary on
/// hosts with no machines (native rootless podman).
pub(super) async fn read_podman_tile_state() -> (String, bool) {
    if let Some(machines) = podman_machines().await {
        let running: Vec<String> = machines
            .iter()
            .filter(|m| m.running)
            .map(|m| m.name.clone())
            .collect();
        if !running.is_empty() {
            return (running.join(", "), true);
        }
        if !machines.is_empty() {
            return ("Stopped".to_string(), false);
        }
    }
    // No machines configured → show container activity instead.
    let s = fetch_podman_summary().await;
    if s.error.is_some() {
        return ("Unavailable".to_string(), false);
    }
    if s.running_containers > 0 {
        (format!("{} running", s.running_containers), true)
    } else {
        ("Idle".to_string(), false)
    }
}

/// Stop every running podman machine (`podman machine stop <name>`).
pub(super) async fn stop_podman_machines() {
    if let Some(machines) = podman_machines().await {
        for m in machines.iter().filter(|m| m.running) {
            let _ = tokio::process::Command::new("podman")
                .args(["machine", "stop", &m.name])
                .status()
                .await;
        }
    }
}

// ── Expandable-tile subtitle / state helpers ─────────────────────────────────

/// Returns (subtitle, is_connected). Connected = has an SSID.
pub(super) fn read_wifi_state() -> (String, bool) {
    let network = network_service();
    if let Some(wifi) = network.wifi.get() {
        if let Some(ssid) = wifi.ssid.get() {
            return (ssid, true);
        }
        if wifi.enabled.get() {
            return ("Not connected".to_string(), false);
        }
        return ("Off".to_string(), false);
    }
    ("Unavailable".to_string(), false)
}

/// Returns (subtitle, is_connected). Connected = at least one device connected.
pub(super) fn read_bt_state() -> (String, bool) {
    let Some(bt) = bluetooth_service() else {
        return ("Unavailable".to_string(), false);
    };
    if !bt.available.get() {
        return ("Unavailable".to_string(), false);
    }
    if !bt.enabled.get() {
        return ("Off".to_string(), false);
    }
    let devices = bt.devices.get();
    let connected: Vec<_> = devices.iter().filter(|d| d.connected.get()).collect();
    match connected.len() {
        0 => ("On · no devices".to_string(), false),
        1 => (connected[0].alias.get(), true),
        n => (format!("{n} connected"), true),
    }
}

pub(super) fn read_audio_out_subtitle() -> String {
    let audio = audio_service();
    if let Some(dev) = audio.default_output.get() {
        return dev.description.get();
    }
    "No device".to_string()
}

pub(super) fn read_mic_subtitle() -> String {
    let audio = audio_service();
    if let Some(dev) = audio.default_input.get() {
        return dev.description.get();
    }
    "No device".to_string()
}

/// Returns (subtitle, is_connected). Connected = a VPN tunnel interface is up.
/// Vendor-neutral: detects OpenVPN (`tun*`), WireGuard (`wg*`, incl. Mullvad's
/// `wg*-mullvad`), NetworkManager VPNs, and PPP-based VPNs by scanning
/// `/sys/class/net` — no VPN-specific CLI. Cheap (a directory read).
pub(super) fn read_vpn_state() -> (String, bool) {
    match vpn_interface() {
        Some(iface) => (format!("Connected · {iface}"), true),
        None => ("Off".to_string(), false),
    }
}

/// Name of the first VPN tunnel interface present, if any.
fn vpn_interface() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.starts_with("tun")
                || n.starts_with("wg")
                || n.starts_with("wireguard")
                || n.starts_with("ppp")
        })
        .collect();
    names.sort();
    names.into_iter().next()
}

/// Compute (subtitle, is_connected) from a ValentReport.
pub(super) fn valent_state_from_report(report: &crate::valent::ValentReport) -> (String, bool) {
    if !report.daemon_available {
        return ("Unavailable".to_string(), false);
    }
    // Find a connected device: reachable + paired
    let connected: Vec<_> = report
        .devices
        .iter()
        .filter(|d| d.reachable && d.paired)
        .collect();
    match connected.len() {
        0 => {
            if report.devices.is_empty() {
                ("No devices".to_string(), false)
            } else {
                ("Not reachable".to_string(), false)
            }
        }
        1 => (connected[0].name.clone(), true),
        n => (format!("{n} connected"), true),
    }
}

pub(super) fn read_airplane_state() -> (bool, bool) {
    // Airplane mode = Wi-Fi is disabled. Returns (is_airplane_on, is_wifi_available).
    let network = network_service();
    if let Some(wifi) = network.wifi.get() {
        let enabled = wifi.enabled.get();
        // Airplane mode is "on" when Wi-Fi is disabled
        (!enabled, true)
    } else {
        (false, false)
    }
}
