//! Service-state probes for the control-center tiles: the pure/async helpers
//! that read live system state (battery, disk, twilight, UFW, podman, Wi-Fi,
//! Bluetooth, audio, VPN, Valent, airplane) into the `(subtitle, is_active)`
//! shape the tile widgets render. Kept separate from the widget wiring so these I/O helpers are
//! independently reviewable; their tests live here too.

use crate::bars::bar_widgets::podman::{PodmanSummary, fetch_podman_summary};
use crate::bars::bar_widgets::ufw::{Status as UfwStatus, fetch_ufw_status_only};
use mshell_services::{
    audio_service, battery_service, bluetooth_service, line_power_service, network_service,
};
use std::time::Duration;
use wayle_battery::types::DeviceState;

#[derive(Debug, Clone, Default)]
pub(crate) struct DiskUsage {
    pub(super) used_bytes: u64,
    pub(super) total_bytes: u64,
}

impl DiskUsage {
    pub(super) fn format(&self) -> String {
        let used = bytes_to_gib(self.used_bytes);
        let total = bytes_to_gib(self.total_bytes);
        let pct = if self.total_bytes > 0 {
            (self.used_bytes as f64 / self.total_bytes as f64 * 100.0).round() as u64
        } else {
            0
        };
        format!("{used:.1}G / {total:.1}G ({pct}%)")
    }
}

#[derive(Debug, Clone)]
pub(super) struct BatterySnapshot {
    pub(super) present: bool,
    pub(super) percent: u8,
    pub(super) state: DeviceState,
    pub(super) on_ac: bool,
}

impl Default for BatterySnapshot {
    fn default() -> Self {
        Self {
            present: false,
            percent: 0,
            state: DeviceState::Unknown,
            on_ac: false,
        }
    }
}

pub(super) fn read_battery() -> BatterySnapshot {
    let Some(service) = battery_service() else {
        return BatterySnapshot::default();
    };
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

// ── Mirror helpers ─────────────────────────────────────────────────────────

/// The Mirror tile's `(subtitle, is_active)`: the running `source → target`
/// pair, else why margo refused the last request, else "Off".
pub(super) fn mirror_tile_state(s: &mshell_margo_client::ctl::MirrorStatus) -> (String, bool) {
    if !s.supported {
        return ("Unavailable".to_string(), false);
    }
    if let Some((source, target)) = &s.active {
        return (format!("{source} \u{2192} {target}"), true);
    }
    match s.error.as_deref() {
        Some("mirroring needs two enabled outputs") => {
            ("Needs a second display".to_string(), false)
        }
        Some(_) => ("Couldn't mirror".to_string(), false),
        None => ("Off".to_string(), false),
    }
}

pub(super) async fn read_mirror_tile_state() -> (String, bool) {
    let status = tokio::task::spawn_blocking(mshell_margo_client::ctl::mirror_status)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    mirror_tile_state(&status)
}

/// Toggle mirroring (laptop panel onto the external display), settle
/// briefly, then re-probe.
pub(super) async fn toggle_mirror_and_probe() -> (String, bool) {
    if let Err(e) =
        mshell_margo_client::ctl::dispatch_async("toggle_mirror".into(), Vec::new()).await
    {
        tracing::warn!(error = %e, "mirror toggle failed");
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    read_mirror_tile_state().await
}

// ── Twilight helpers ───────────────────────────────────────────────────────

/// Flip the twilight filter (right-click quick toggle), settle briefly,
/// then re-probe so the caller can refresh the tile to the new state.
pub(super) async fn toggle_twilight_and_probe() -> Option<(bool, String)> {
    if let Err(e) =
        mshell_margo_client::ctl::dispatch_async("twilight_toggle".into(), Vec::new()).await
    {
        tracing::warn!(error = %e, "twilight toggle failed");
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodmanMachine {
    pub(super) name: String,
    pub(super) running: bool,
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
    let machines = podman_machines_cached().await;
    if let Some(state) = machines_tile_state(machines.as_deref()) {
        return state;
    }
    podman_tile_state(None, &fetch_podman_summary().await)
}

/// How long `podman machine list` is trusted: machines rarely change, and
/// the tile follows container changes through the shared poll anyway.
const MACHINES_TTL: Duration = Duration::from_secs(60);

static MACHINES: std::sync::Mutex<Option<(std::time::Instant, Option<Vec<PodmanMachine>>)>> =
    std::sync::Mutex::new(None);

/// [`podman_machines`], cached for [`MACHINES_TTL`].
pub(super) async fn podman_machines_cached() -> Option<Vec<PodmanMachine>> {
    if let Ok(cache) = MACHINES.lock()
        && let Some((at, machines)) = cache.as_ref()
        && at.elapsed() < MACHINES_TTL
    {
        return machines.clone();
    }
    let machines = podman_machines().await;
    if let Ok(mut cache) = MACHINES.lock() {
        *cache = Some((std::time::Instant::now(), machines.clone()));
    }
    machines
}

/// The tile state from machines alone: running ones by name, or
/// "Stopped"; `None` when there are none (show containers instead).
fn machines_tile_state(machines: Option<&[PodmanMachine]>) -> Option<(String, bool)> {
    let machines = machines.filter(|m| !m.is_empty())?;
    let running: Vec<&str> = machines
        .iter()
        .filter(|m| m.running)
        .map(|m| m.name.as_str())
        .collect();
    Some(if running.is_empty() {
        ("Stopped".to_string(), false)
    } else {
        (running.join(", "), true)
    })
}

/// Podman tile `(subtitle, active)`: machines first; without any, the
/// container activity of the shared [`PodmanSummary`].
pub(super) fn podman_tile_state(
    machines: Option<&[PodmanMachine]>,
    s: &PodmanSummary,
) -> (String, bool) {
    if let Some(state) = machines_tile_state(machines) {
        return state;
    }
    if s.error.is_some() {
        ("Unavailable".to_string(), false)
    } else if s.running_containers > 0 {
        (format!("{} running", s.running_containers), true)
    } else {
        ("Idle".to_string(), false)
    }
}

/// Stop every running podman machine (`podman machine stop <name>`).
pub(super) async fn stop_podman_machines() {
    // The tile re-reads the machines right after.
    if let Ok(mut cache) = MACHINES.lock() {
        *cache = None;
    }
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
    let Some(network) = network_service() else {
        return (String::new(), false);
    };
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
    if let Some(dev) = audio_service().and_then(|a| a.default_output.get()) {
        return dev.description.get();
    }
    "No device".to_string()
}

pub(super) fn read_mic_subtitle() -> String {
    if let Some(dev) = audio_service().and_then(|a| a.default_input.get()) {
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
    if let Some(wifi) = network_service().and_then(|n| n.wifi.get()) {
        let enabled = wifi.enabled.get();
        // Airplane mode is "on" when Wi-Fi is disabled
        (!enabled, true)
    } else {
        (false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PodmanMachine, mirror_tile_state, podman_tile_state, twilight_mode_label,
        twilight_subtitle, valent_state_from_report,
    };
    use crate::bars::bar_widgets::podman::PodmanSummary;

    /// The Podman tile reads the shared poll's summary: machines win,
    /// containers otherwise.
    #[test]
    fn podman_tile_from_shared_summary() {
        let busy = PodmanSummary {
            running_containers: 2,
            ..Default::default()
        };
        assert_eq!(podman_tile_state(None, &busy), ("2 running".into(), true));
        assert_eq!(
            podman_tile_state(Some(&[]), &PodmanSummary::default()),
            ("Idle".into(), false)
        );
        let broken = PodmanSummary {
            error: Some("no podman".into()),
            ..Default::default()
        };
        assert_eq!(
            podman_tile_state(None, &broken),
            ("Unavailable".into(), false)
        );
        let machines = [
            PodmanMachine {
                name: "dev".into(),
                running: true,
            },
            PodmanMachine {
                name: "old".into(),
                running: false,
            },
        ];
        assert_eq!(
            podman_tile_state(Some(&machines), &busy),
            ("dev".into(), true)
        );
        assert_eq!(
            podman_tile_state(Some(&machines[1..]), &busy),
            ("Stopped".into(), false)
        );
    }
    use crate::twilight::TwilightStatus;
    use crate::valent::{Device, ValentReport};

    fn device(name: &str, reachable: bool, paired: bool) -> Device {
        Device {
            name: name.to_string(),
            reachable,
            paired,
            ..Default::default()
        }
    }

    #[test]
    fn twilight_mode_labels() {
        assert_eq!(twilight_mode_label("geo"), "Auto");
        assert_eq!(twilight_mode_label("manual"), "Manual");
        assert_eq!(twilight_mode_label("static"), "Static");
        assert_eq!(twilight_mode_label("schedule"), "Schedule");
        assert_eq!(twilight_mode_label("???"), "On"); // fallback differs from twilight.rs
    }

    #[test]
    fn twilight_subtitle_off_when_disabled() {
        let s = TwilightStatus {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(twilight_subtitle(&s), "Off");
    }

    #[test]
    fn twilight_subtitle_mode_with_and_without_temp() {
        let s = TwilightStatus {
            enabled: true,
            mode: "schedule".into(),
            current_temp_k: Some(3500),
            ..Default::default()
        };
        assert_eq!(twilight_subtitle(&s), "Schedule \u{b7} 3500K");

        let s2 = TwilightStatus {
            enabled: true,
            mode: "geo".into(),
            current_temp_k: None,
            ..Default::default()
        };
        assert_eq!(twilight_subtitle(&s2), "Auto");
    }

    #[test]
    fn mirror_tile_shows_the_pair_the_refusal_or_off() {
        use mshell_margo_client::ctl::MirrorStatus;
        let on = MirrorStatus {
            active: Some(("eDP-1".into(), "DP-3".into())),
            error: None,
            supported: true,
        };
        assert_eq!(
            mirror_tile_state(&on),
            ("eDP-1 \u{2192} DP-3".to_string(), true)
        );
        let refused = MirrorStatus {
            active: None,
            error: Some("mirroring needs two enabled outputs".into()),
            supported: true,
        };
        assert_eq!(
            mirror_tile_state(&refused),
            ("Needs a second display".to_string(), false)
        );
        let idle = MirrorStatus {
            supported: true,
            ..Default::default()
        };
        assert_eq!(mirror_tile_state(&idle), ("Off".to_string(), false));
        assert_eq!(
            mirror_tile_state(&MirrorStatus::default()),
            ("Unavailable".to_string(), false)
        );
    }

    #[test]
    fn valent_state_daemon_unavailable() {
        let r = ValentReport {
            daemon_available: false,
            ..Default::default()
        };
        assert_eq!(
            valent_state_from_report(&r),
            ("Unavailable".to_string(), false)
        );
    }

    #[test]
    fn valent_state_no_devices_vs_not_reachable() {
        let r = ValentReport {
            daemon_available: true,
            devices: vec![],
        };
        assert_eq!(
            valent_state_from_report(&r),
            ("No devices".to_string(), false)
        );

        let r = ValentReport {
            daemon_available: true,
            devices: vec![device("Phone", false, true)],
        };
        assert_eq!(
            valent_state_from_report(&r),
            ("Not reachable".to_string(), false)
        );
    }

    #[test]
    fn valent_state_one_and_many_connected() {
        let r = ValentReport {
            daemon_available: true,
            devices: vec![device("Phone", true, true)],
        };
        assert_eq!(valent_state_from_report(&r), ("Phone".to_string(), true));

        let r = ValentReport {
            daemon_available: true,
            devices: vec![device("A", true, true), device("B", true, true)],
        };
        assert_eq!(
            valent_state_from_report(&r),
            ("2 connected".to_string(), true)
        );
    }
}
