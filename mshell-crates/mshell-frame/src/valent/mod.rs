//! Shared Valent (KDE Connect for GNOME) integration core, used by
//! the Valent bar pill and its panel menu. Ports the noctalia
//! `valent-connect` plugin: it talks to the `ca.andyholmes.Valent`
//! session-bus service through `gdbus` and exposes device discovery,
//! battery / connectivity stats, and the find / ping / browse /
//! share / pair / unpair actions.
//!
//! We shell out to `gdbus` (rather than wiring a zbus proxy) for the
//! same reason the plugin does: Valent's per-device API is a
//! `org.gtk.Actions` GActionGroup whose Describe/Activate payloads are
//! deeply-nested `av` variants that are far easier to drive via the
//! CLI's GVariant text form than to model in Rust.

use regex::Regex;
use std::sync::LazyLock;
use tracing::warn;

const DEST: &str = "ca.andyholmes.Valent";
const ROOT_PATH: &str = "/ca/andyholmes/Valent";

// Device `State` is a uint32 bitmask (mirrors the plugin).
const STATE_CONNECTED: u32 = 1;
const STATE_PAIRED: u32 = 2;
const STATE_PAIR_INCOMING: u32 = 4;
const STATE_PAIR_OUTGOING: u32 = 8;

/// One paired or discoverable device.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Device {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Connected / on the network right now.
    pub(crate) reachable: bool,
    pub(crate) paired: bool,
    /// We requested pairing and are waiting on the peer.
    pub(crate) pair_requested: bool,
    /// The peer requested pairing and is waiting on us.
    pub(crate) pair_incoming: bool,
    /// `None` until the battery plugin reports in.
    pub(crate) battery_charge: Option<i32>,
    pub(crate) battery_charging: bool,
    /// Cellular type label (5G / LTE / …); empty when unknown.
    pub(crate) network_type: String,
    /// 0–4 signal bars, `-1` unknown.
    pub(crate) network_strength: i32,
}

impl Device {
    pub(crate) fn connection_icon(&self) -> &'static str {
        if self.reachable {
            "phone-symbolic"
        } else {
            "phone-disconnected-symbolic"
        }
    }
}

/// A full snapshot of the Valent daemon + its devices.
#[derive(Debug, Clone, Default)]
pub(crate) struct ValentReport {
    /// `ca.andyholmes.Valent` is on the session bus.
    pub(crate) daemon_available: bool,
    /// Devices, sorted by name.
    pub(crate) devices: Vec<Device>,
}

impl ValentReport {
    /// Pick the "main" device: the sticky/configured id if present,
    /// else the first reachable one, else the first.
    pub(crate) fn main_device(&self, preferred_id: &str) -> Option<&Device> {
        if self.devices.is_empty() {
            return None;
        }
        if !preferred_id.is_empty() {
            if let Some(d) = self.devices.iter().find(|d| d.id == preferred_id) {
                return Some(d);
            }
        }
        self.devices
            .iter()
            .find(|d| d.reachable)
            .or_else(|| self.devices.first())
    }

    /// Bar-pill icon for the main device (or an error glyph when the
    /// daemon is down).
    pub(crate) fn pill_icon(&self, preferred_id: &str) -> &'static str {
        if !self.daemon_available {
            return "dialog-warning-symbolic";
        }
        match self.main_device(preferred_id) {
            None => "phone-disconnected-symbolic",
            Some(d) => d.connection_icon(),
        }
    }
}

// ── Object-path escaping ────────────────────────────────────────
// Each non-alphanumeric byte becomes `_<lowercase 2-hex>`, matching
// Valent's D-Bus object-path encoding.
fn escape_object_path(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push('_');
            out.push_str(&format!("{b:02x}"));
        }
    }
    out
}

fn device_path(id: &str) -> String {
    format!("{ROOT_PATH}/Device/{}", escape_object_path(id))
}

// ── Probe ───────────────────────────────────────────────────────

/// Full probe: daemon presence → managed devices → per-device battery
/// + connectivity for the paired/reachable ones.
pub(crate) async fn probe() -> ValentReport {
    if !daemon_available().await {
        return ValentReport::default();
    }

    let managed = match run(&[
        "call", "--session", "--dest", DEST, "--object-path", ROOT_PATH, "--method",
        "org.freedesktop.DBus.ObjectManager.GetManagedObjects",
    ])
    .await
    {
        Ok(out) => out,
        Err(e) => {
            warn!(error = %e, "valent: GetManagedObjects failed");
            return ValentReport::default();
        }
    };

    let mut devices = parse_devices(&managed);

    for dev in devices.iter_mut() {
        if dev.paired && dev.reachable {
            fetch_battery(dev).await;
            fetch_connectivity(dev).await;
        }
    }

    devices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    ValentReport { daemon_available: true, devices }
}

async fn daemon_available() -> bool {
    match run(&[
        "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path",
        "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames",
    ])
    .await
    {
        Ok(out) => out.contains(DEST),
        Err(_) => false,
    }
}

fn parse_devices(raw: &str) -> Vec<Device> {
    static DEV_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"objectpath\s+'([^']+/Device/[^']+)'\s*:\s*\{'ca\.andyholmes\.Valent\.Device'\s*:\s*\{([^}]*)\}",
        )
        .expect("valent device regex")
    });
    static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'Id':\s*<'([^']+)'>").unwrap());
    static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'Name':\s*<'([^']+)'>").unwrap());
    static STATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'State':\s*<uint32\s+(\d+)>").unwrap());

    let mut devices = Vec::new();
    for cap in DEV_RE.captures_iter(raw) {
        let path = &cap[1];
        let props = &cap[2];

        let id = ID_RE
            .captures(props)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| {
                path.split("/Device/").nth(1).unwrap_or(path).to_string()
            });
        let name = NAME_RE
            .captures(props)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| id.clone());
        let state: u32 = STATE_RE
            .captures(props)
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(0);

        devices.push(Device {
            id,
            name,
            reachable: state & STATE_CONNECTED != 0,
            paired: state & STATE_PAIRED != 0,
            pair_requested: state & STATE_PAIR_OUTGOING != 0,
            pair_incoming: state & STATE_PAIR_INCOMING != 0,
            battery_charge: None,
            battery_charging: false,
            network_type: String::new(),
            network_strength: -1,
        });
    }
    devices
}

/// `org.gtk.Actions.Describe battery.state` →
/// `(… [<{'charging': <false>, 'percentage': <64.0>, …}>] …)`.
async fn fetch_battery(dev: &mut Device) {
    static PCT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'percentage':\s*<([0-9.]+)>").unwrap());
    static CHG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'charging':\s*<(true|false)>").unwrap());

    let path = device_path(&dev.id);
    let Ok(out) = describe(&path, "battery.state").await else {
        return;
    };
    if let Some(c) = PCT_RE.captures(&out) {
        if let Ok(pct) = c[1].parse::<f64>() {
            dev.battery_charge = Some(pct.round() as i32);
        }
    }
    if let Some(c) = CHG_RE.captures(&out) {
        dev.battery_charging = &c[1] == "true";
    }
}

/// `org.gtk.Actions.Describe connectivity_report.state` →
/// `… {'network-type': <'LTE'>, 'signal-strength': <int64 3>} …`.
async fn fetch_connectivity(dev: &mut Device) {
    static TYPE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'network-type':\s*<'([^']+)'>").unwrap());
    static SIG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'signal-strength':\s*<(?:int64\s+)?(\d+)>").unwrap());

    let path = device_path(&dev.id);
    let Ok(out) = describe(&path, "connectivity_report.state").await else {
        return;
    };
    if let Some(c) = TYPE_RE.captures(&out) {
        dev.network_type = c[1].to_string();
    }
    if let Some(c) = SIG_RE.captures(&out) {
        dev.network_strength = c[1].parse().unwrap_or(-1);
    }
}

async fn describe(path: &str, action: &str) -> Result<String, String> {
    run(&[
        "call", "--session", "--dest", DEST, "--object-path", path, "--method",
        "org.gtk.Actions.Describe", action,
    ])
    .await
}

// ── Actions ─────────────────────────────────────────────────────

/// Fire a parameterless GAction on a device (find / ping / browse /
/// pair / unpair). Fire-and-forget — errors are logged, not surfaced.
async fn activate(device_id: &str, action: &str) {
    activate_param(device_id, action, "@av []").await;
}

async fn activate_param(device_id: &str, action: &str, param: &str) {
    let path = device_path(device_id);
    if let Err(e) = run(&[
        "call", "--session", "--dest", DEST, "--object-path", &path, "--method",
        "org.gtk.Actions.Activate", action, param, "{}",
    ])
    .await
    {
        warn!(error = %e, action, "valent: activate failed");
    }
}

pub(crate) async fn find_my_phone(device_id: String) {
    activate(&device_id, "findmyphone.ring").await;
}

pub(crate) async fn ping(device_id: String) {
    activate(&device_id, "ping.ping").await;
}

pub(crate) async fn browse_files(device_id: String) {
    activate(&device_id, "sftp.browse").await;
}

pub(crate) async fn pair(device_id: String) {
    activate(&device_id, "pair").await;
}

pub(crate) async fn unpair(device_id: String) {
    activate(&device_id, "unpair").await;
}

/// Share a local file with the device. `path` is a filesystem path or
/// a `file://` URI. We pass it as a GVariant `av` holding one
/// double-quoted string so paths with single quotes are safe.
pub(crate) async fn share_file(device_id: String, path: String) {
    let uri = if path.starts_with("file://") {
        path
    } else {
        format!("file://{path}")
    };
    let escaped = uri.replace('\\', "\\\\").replace('"', "\\\"");
    let param = format!("[<\"{escaped}\">]");
    activate_param(&device_id, "share.uri", &param).await;
}

/// Kick Valent's discovery by clearing then re-clearing its
/// `device-addresses` gsetting (the plugin's refresh trick).
pub(crate) async fn refresh_discovery() {
    let _ = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(
            "gsettings set ca.andyholmes.Valent device-addresses \"['']\"; \
             gsettings set ca.andyholmes.Valent device-addresses \"[]\"",
        )
        .output()
        .await;
}

// ── gdbus helper ────────────────────────────────────────────────

async fn run(args: &[&str]) -> Result<String, String> {
    match tokio::process::Command::new("gdbus").args(args).output().await {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(format!("gdbus spawn: {e}")),
    }
}
