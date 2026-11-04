//! Shared Valent (KDE Connect for GNOME) integration core, used by
//! the Valent bar pill and its panel menu. Ports the noctalia
//! `valent-connect` plugin: it talks to the `ca.andyholmes.Valent`
//! session-bus service and exposes device discovery, battery /
//! connectivity stats, and the find / ping / browse / share (file /
//! text) / clipboard push-pull / pair / unpair actions.
//!
//! The periodic [`probe`] (NameHasOwner, GetManagedObjects and the
//! `org.gtk.Actions.Describe` state reads) goes over mshell's shared
//! zbus session connection: it runs every few seconds, and the old
//! `gdbus` spawns cost a process plus a fresh bus connection each. Actions
//! use the same connection with typed `av` parameters (no GVariant text
//! escaping or subprocesses).
//!
//! [`scrcpy`] adds phone screen mirroring over adb (Valent has none).

pub(crate) mod scrcpy;

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ValentStoreFields};
use mshell_utils::shared_poll::SharedPoll;
use reactive_graph::prelude::GetUntracked;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;
use tracing::warn;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

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
        // `phone-disconnected-symbolic` isn't a real freedesktop icon, so
        // the bar pill rendered blank when a device was unreachable. Use
        // the always-present `phone-symbolic`; the disconnected state is
        // conveyed by the (absent) battery label + tooltip.
        "phone-symbolic"
    }
}

/// A full snapshot of the Valent daemon + its devices.
#[derive(Debug, Clone, Default, PartialEq)]
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
        if !preferred_id.is_empty()
            && let Some(d) = self.devices.iter().find(|d| d.id == preferred_id)
        {
            return Some(d);
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
        // Always a real, theme-present icon so the pill never renders
        // blank — previously `phone-disconnected-symbolic` (nonexistent)
        // made the whole widget disappear when no device was paired.
        match self.main_device(preferred_id) {
            None => "phone-symbolic",
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

/// One D-Bus property / state dictionary.
type Props = HashMap<String, OwnedValue>;
/// `org.freedesktop.DBus.ObjectManager.GetManagedObjects` reply.
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

const DEVICE_IFACE: &str = "ca.andyholmes.Valent.Device";

/// The bar pills' shared Valent poll: one loop for every bar, every
/// `valent.poll_interval_secs`, only while a pill is mapped.
pub(crate) static VALENT_POLL: LazyLock<SharedPoll<ValentReport>> =
    LazyLock::new(|| SharedPoll::new(poll_interval, || Box::pin(probe())));

fn poll_interval() -> Duration {
    let secs = config_manager()
        .config()
        .valent()
        .poll_interval_secs()
        .get_untracked();
    Duration::from_secs(secs.max(1) as u64)
}

/// Full probe: daemon presence → managed devices → per-device battery
/// + connectivity for the paired/reachable ones.
pub(crate) async fn probe() -> ValentReport {
    let conn = match mshell_services::systemd::session_bus().await {
        Ok(conn) => conn,
        Err(e) => {
            warn!(error = %e, "valent: no session bus");
            return ValentReport::default();
        }
    };
    match probe_on(&conn).await {
        Ok(report) => report,
        Err(e) => {
            warn!(error = %e, "valent: probe failed");
            ValentReport::default()
        }
    }
}

async fn probe_on(conn: &zbus::Connection) -> zbus::Result<ValentReport> {
    let has_owner: bool = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(DEST,),
        )
        .await?
        .body()
        .deserialize()?;
    if !has_owner {
        return Ok(ValentReport::default());
    }

    let managed: ManagedObjects = conn
        .call_method(
            Some(DEST),
            ROOT_PATH,
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await?
        .body()
        .deserialize()?;

    let mut devices = devices_from_managed(&managed);
    for dev in devices.iter_mut() {
        if dev.paired && dev.reachable {
            if let Some(state) = describe_state(conn, &dev.id, "battery.state").await {
                apply_battery(dev, &state);
            }
            if let Some(state) = describe_state(conn, &dev.id, "connectivity_report.state").await {
                apply_connectivity(dev, &state);
            }
        }
    }
    devices.sort_by_key(|a| a.name.to_lowercase());

    Ok(ValentReport {
        daemon_available: true,
        devices,
    })
}

/// The devices among the managed objects (objects under `…/Device/`
/// carrying the Device interface).
fn devices_from_managed(managed: &ManagedObjects) -> Vec<Device> {
    let mut devices: Vec<Device> = managed
        .iter()
        .filter_map(|(path, ifaces)| {
            let tail = path.as_str().split_once("/Device/")?.1;
            Some(device_from_props(tail, ifaces.get(DEVICE_IFACE)?))
        })
        .collect();
    // HashMap order is arbitrary; keep the result deterministic.
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

/// A device from its Device-interface properties. `path_tail` (the escaped
/// id in the object path) stands in for a missing `Id`.
fn device_from_props(path_tail: &str, props: &Props) -> Device {
    let id = prop(props, "Id")
        .and_then(as_str)
        .unwrap_or_else(|| path_tail.to_string());
    let name = prop(props, "Name")
        .and_then(as_str)
        .unwrap_or_else(|| id.clone());
    let state = prop(props, "State").and_then(as_u32).unwrap_or(0);
    Device {
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
    }
}

/// `org.gtk.Actions.Describe(action)` → `(enabled, param type, [state])`;
/// the state of the battery / connectivity actions is an `a{sv}`.
async fn describe_state(conn: &zbus::Connection, id: &str, action: &str) -> Option<Props> {
    let path = device_path(id);
    let reply = conn
        .call_method(
            Some(DEST),
            path.as_str(),
            Some("org.gtk.Actions"),
            "Describe",
            &(action,),
        )
        .await
        .ok()?;
    let (_enabled, _param, state): (bool, zbus::zvariant::Signature, Vec<OwnedValue>) =
        reply.body().deserialize().ok()?;
    state_props(state.first()?)
}

/// The `a{sv}` inside an action state (unwrapping variants).
fn state_props(v: &Value<'_>) -> Option<Props> {
    match v {
        Value::Value(inner) => state_props(inner),
        Value::Dict(_) => Props::try_from(v.try_clone().ok()?).ok(),
        _ => None,
    }
}

/// `battery.state` → `{'charging': <b>, 'percentage': <d>, …}`.
fn apply_battery(dev: &mut Device, state: &Props) {
    if let Some(pct) = prop(state, "percentage").and_then(as_f64) {
        dev.battery_charge = Some(pct.round() as i32);
    }
    if let Some(charging) = prop(state, "charging").and_then(as_bool) {
        dev.battery_charging = charging;
    }
}

/// `connectivity_report.state` →
/// `{'network-type': <s>, 'signal-strength': <x>}` (an unusable strength
/// reads as `-1`, unknown).
fn apply_connectivity(dev: &mut Device, state: &Props) {
    if let Some(t) = prop(state, "network-type").and_then(as_str) {
        dev.network_type = t;
    }
    if let Some(v) = prop(state, "signal-strength") {
        dev.network_strength = as_i64(v)
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| *n >= 0)
            .unwrap_or(-1);
    }
}

fn prop<'p>(props: &'p Props, key: &str) -> Option<&'p Value<'static>> {
    props.get(key).map(|v| &**v)
}

/// `v` without any variant wrapping.
fn unwrap_variant<'v>(v: &'v Value<'v>) -> &'v Value<'v> {
    match v {
        Value::Value(inner) => unwrap_variant(inner),
        other => other,
    }
}

fn as_str(v: &Value<'_>) -> Option<String> {
    match unwrap_variant(v) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn as_bool(v: &Value<'_>) -> Option<bool> {
    match unwrap_variant(v) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_i64(v: &Value<'_>) -> Option<i64> {
    match unwrap_variant(v) {
        Value::U8(n) => Some(i64::from(*n)),
        Value::I16(n) => Some(i64::from(*n)),
        Value::U16(n) => Some(i64::from(*n)),
        Value::I32(n) => Some(i64::from(*n)),
        Value::U32(n) => Some(i64::from(*n)),
        Value::I64(n) => Some(*n),
        Value::U64(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn as_u32(v: &Value<'_>) -> Option<u32> {
    as_i64(v).and_then(|n| u32::try_from(n).ok())
}

fn as_f64(v: &Value<'_>) -> Option<f64> {
    match unwrap_variant(v) {
        Value::F64(f) => Some(*f),
        other => as_i64(other).map(|n| n as f64),
    }
}

// ── Actions ─────────────────────────────────────────────────────

/// Fire a parameterless GAction on a device (find / ping / browse /
/// pair / unpair). Fire-and-forget — errors are logged, not surfaced.
async fn activate(device_id: &str, action: &str) {
    activate_param(device_id, action, None).await;
}

/// `org.gtk.Actions.Activate` body: `s av a{sv}`. A parameterless action
/// gets an empty `av`; share actions get one string variant.
fn action_body<'a>(
    action: &'a str,
    param: Option<&'a str>,
) -> (&'a str, Vec<Value<'a>>, HashMap<&'a str, Value<'a>>) {
    (
        action,
        param.into_iter().map(Value::from).collect(),
        HashMap::new(),
    )
}

async fn activate_param(device_id: &str, action: &str, param: Option<&str>) {
    let result = async {
        let conn = mshell_services::systemd::session_bus().await?;
        conn.call_method(
            Some(DEST),
            device_path(device_id).as_str(),
            Some("org.gtk.Actions"),
            "Activate",
            &action_body(action, param),
        )
        .await?;
        Ok::<_, zbus::Error>(())
    }
    .await;
    if let Err(e) = result {
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
/// a `file://` URI. It is passed as a typed string variant, so quotes and
/// backslashes are ordinary data.
pub(crate) async fn share_file(device_id: String, path: String) {
    let uri = if path.starts_with("file://") {
        path
    } else {
        format!("file://{path}")
    };
    activate_param(&device_id, "share.uri", Some(&uri)).await;
}

/// Share arbitrary text with the device (`share.text` GAction).
pub(crate) async fn share_text(device_id: String, text: String) {
    activate_param(&device_id, "share.text", Some(&text)).await;
}

/// Push the local clipboard to the device.
pub(crate) async fn clipboard_push(device_id: String) {
    activate(&device_id, "clipboard.push").await;
}

/// Pull the device's clipboard into the local one.
pub(crate) async fn clipboard_pull(device_id: String) {
    activate(&device_id, "clipboard.pull").await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        v.into().try_to_owned().unwrap()
    }

    fn props(entries: &[(&str, OwnedValue)]) -> Props {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.try_clone().unwrap()))
            .collect()
    }

    fn object(path: &str, ifaces: Vec<(&str, Props)>) -> (OwnedObjectPath, HashMap<String, Props>) {
        (
            OwnedObjectPath::try_from(path).unwrap(),
            ifaces
                .into_iter()
                .map(|(i, p)| (i.to_string(), p))
                .collect(),
        )
    }

    fn device_props(id: &str, name: &str, state: u32) -> Props {
        props(&[
            ("IconName", ov("phone-symbolic")),
            ("Id", ov(id)),
            ("Name", ov(name)),
            ("State", ov(state)),
        ])
    }

    /// A paired, connected phone and a discovered, unpaired laptop.
    fn managed() -> ManagedObjects {
        [
            object(
                "/ca/andyholmes/Valent/Device/3f2a91c0",
                vec![(DEVICE_IFACE, device_props("3f2a91c0", "Pixel 7", 3))],
            ),
            object(
                "/ca/andyholmes/Valent/Device/laptop_2d01",
                vec![(DEVICE_IFACE, device_props("laptop-01", "ThinkPad", 1))],
            ),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn parses_managed_objects() {
        let devices = devices_from_managed(&managed());
        assert_eq!(devices.len(), 2);
        let phone = &devices[0];
        assert_eq!(
            (phone.id.as_str(), phone.name.as_str()),
            ("3f2a91c0", "Pixel 7")
        );
        assert!(phone.reachable && phone.paired);
        assert!(!phone.pair_requested && !phone.pair_incoming);
        assert_eq!(phone.battery_charge, None);
        assert_eq!(phone.network_strength, -1);
        let laptop = &devices[1];
        assert_eq!(
            (laptop.id.as_str(), laptop.name.as_str()),
            ("laptop-01", "ThinkPad")
        );
        assert!(laptop.reachable && !laptop.paired);
    }

    #[test]
    fn state_bits_map_to_flags() {
        let with_state = |state: u32| {
            let d = device_from_props("x", &props(&[("Id", ov("x")), ("State", ov(state))]));
            (d.reachable, d.paired, d.pair_incoming, d.pair_requested)
        };
        assert_eq!(with_state(0), (false, false, false, false));
        assert_eq!(with_state(STATE_CONNECTED), (true, false, false, false));
        assert_eq!(with_state(STATE_PAIRED), (false, true, false, false));
        assert_eq!(
            with_state(STATE_CONNECTED | STATE_PAIR_INCOMING),
            (true, false, true, false)
        );
        assert_eq!(
            with_state(STATE_CONNECTED | STATE_PAIR_OUTGOING),
            (true, false, false, true)
        );
    }

    #[test]
    fn missing_properties_fall_back() {
        let d = device_from_props("abc_2d1", &props(&[("IconName", ov("phone-symbolic"))]));
        assert_eq!(d.id, "abc_2d1", "id falls back to the object-path tail");
        assert_eq!(d.name, d.id, "name falls back to the id");
        assert!(!d.reachable && !d.paired, "no State means not connected");
        // A wrongly typed property is ignored rather than misread.
        let d = device_from_props("p", &props(&[("Name", ov(7u32)), ("State", ov("3"))]));
        assert_eq!((d.name.as_str(), d.paired), ("p", false));
    }

    #[test]
    fn non_device_objects_are_skipped() {
        let managed: ManagedObjects = [
            // The manager's own path is not a device.
            object(
                "/ca/andyholmes/Valent",
                vec![("org.gtk.Actions", Props::new())],
            ),
            // A device path without the Device interface.
            object(
                "/ca/andyholmes/Valent/Device/x",
                vec![("org.freedesktop.DBus.Properties", Props::new())],
            ),
            // The Device interface among others; names with quotes and
            // non-ASCII need no escaping any more.
            object(
                "/ca/andyholmes/Valent/Device/y",
                vec![
                    ("org.example.Other", props(&[("Name", ov("not it"))])),
                    (DEVICE_IFACE, device_props("y", "Kenan's Çağrı {tab}", 2)),
                ],
            ),
        ]
        .into_iter()
        .collect();
        let devices = devices_from_managed(&managed);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Kenan's Çağrı {tab}");
        assert!(devices[0].paired && !devices[0].reachable);
        assert!(devices_from_managed(&ManagedObjects::new()).is_empty());
    }

    #[test]
    fn battery_state() {
        let mut d = Device::default();
        apply_battery(
            &mut d,
            &props(&[
                ("charging", ov(true)),
                ("percentage", ov(63.6f64)),
                ("icon-name", ov("battery-good")),
            ]),
        );
        assert_eq!((d.battery_charge, d.battery_charging), (Some(64), true));
        apply_battery(
            &mut d,
            &props(&[("charging", ov(false)), ("percentage", ov(100.0f64))]),
        );
        assert_eq!((d.battery_charge, d.battery_charging), (Some(100), false));
        // Nothing reported: unchanged.
        apply_battery(&mut d, &Props::new());
        assert_eq!(d.battery_charge, Some(100));
    }

    #[test]
    fn connectivity_state() {
        let mut d = Device {
            network_strength: -1,
            ..Default::default()
        };
        apply_connectivity(
            &mut d,
            &props(&[("network-type", ov("LTE")), ("signal-strength", ov(3i64))]),
        );
        assert_eq!((d.network_type.as_str(), d.network_strength), ("LTE", 3));
        apply_connectivity(
            &mut d,
            &props(&[("network-type", ov("5G")), ("signal-strength", ov(4i32))]),
        );
        assert_eq!((d.network_type.as_str(), d.network_strength), ("5G", 4));
        apply_connectivity(&mut d, &props(&[("signal-strength", ov("strong"))]));
        assert_eq!(d.network_strength, -1, "unusable strength is unknown");
    }

    #[test]
    fn action_state_dict_is_found_inside_variants() {
        let dict = props(&[("percentage", ov(50.0f64))]);
        let as_value = Value::from(
            dict.iter()
                .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                .collect::<HashMap<_, _>>(),
        );
        let wrapped = Value::Value(Box::new(as_value.try_clone().unwrap()));
        for v in [&as_value, &wrapped] {
            let got = state_props(v).expect("dict");
            assert_eq!(prop(&got, "percentage").and_then(as_f64), Some(50.0));
        }
        assert!(state_props(&Value::from(3u32)).is_none());
    }

    /// Read-only probe against the live session bus (`cargo test -- --ignored
    /// live_probe`): checks the zbus calls and reply types against a running
    /// Valent. Not part of the normal run.
    #[tokio::test]
    #[ignore]
    async fn live_probe() {
        let conn = zbus::Connection::session().await.expect("session bus");
        let report = probe_on(&conn).await.expect("probe");
        eprintln!("{report:?}");
    }

    #[test]
    fn object_path_escaping() {
        assert_eq!(escape_object_path("abc123"), "abc123");
        assert_eq!(escape_object_path("a-b.c"), "a_2db_2ec");
        assert_eq!(
            escape_object_path("a_b"),
            "a_5fb",
            "underscore itself is escaped"
        );
        assert_eq!(escape_object_path(""), "");
        assert_eq!(escape_object_path("ü"), "_c3_bc", "per UTF-8 byte");
        assert_eq!(device_path("x-1"), "/ca/andyholmes/Valent/Device/x_2d1");
    }

    #[test]
    fn action_parameters_are_typed_strings_not_gvariant_text() {
        use zbus::zvariant::{LE, serialized::Context, to_bytes};
        for text in ["hi", "it's", r#"say "x""#, r"C:\dir", "a\nb", "ü"] {
            let body = action_body("share.text", Some(text));
            let encoded = to_bytes(Context::new_dbus(LE, 0), &body).unwrap();
            let ((action, values, platform), _): ((String, Vec<OwnedValue>, Props), _) =
                encoded.deserialize().unwrap();
            assert_eq!(action, "share.text");
            assert_eq!(values.len(), 1);
            assert_eq!(as_str(&values[0]), Some(text.into()));
            assert!(platform.is_empty());
        }
        let (_, values, _) = action_body("ping.ping", None);
        assert!(values.is_empty());
    }

    fn device(id: &str, reachable: bool) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            reachable,
            network_strength: -1,
            ..Default::default()
        }
    }

    #[test]
    fn main_device_preference() {
        let report = ValentReport {
            daemon_available: true,
            devices: vec![device("a", false), device("b", true), device("c", true)],
        };
        assert_eq!(
            report.main_device("c").unwrap().id,
            "c",
            "configured id wins"
        );
        assert_eq!(
            report.main_device("").unwrap().id,
            "b",
            "else first reachable"
        );
        assert_eq!(report.main_device("gone").unwrap().id, "b");
        let offline = ValentReport {
            daemon_available: true,
            devices: vec![device("a", false), device("b", false)],
        };
        assert_eq!(offline.main_device("").unwrap().id, "a", "else first");
        assert!(ValentReport::default().main_device("a").is_none());
    }

    #[test]
    fn pill_icon_never_blank() {
        assert_eq!(
            ValentReport::default().pill_icon(""),
            "dialog-warning-symbolic"
        );
        let empty = ValentReport {
            daemon_available: true,
            devices: Vec::new(),
        };
        assert_eq!(empty.pill_icon(""), "phone-symbolic");
    }
}
