//! Shared Valent (KDE Connect for GNOME) integration core, used by
//! the Valent bar pill and its panel menu. Ports the noctalia
//! `valent-connect` plugin: it talks to the `ca.andyholmes.Valent`
//! session-bus service through `gdbus` and exposes device discovery,
//! battery / connectivity stats, and the find / ping / browse /
//! share (file / text) / clipboard push-pull / pair / unpair actions.
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
        // `phone-disconnected-symbolic` isn't a real freedesktop icon, so
        // the bar pill rendered blank when a device was unreachable. Use
        // the always-present `phone-symbolic`; the disconnected state is
        // conveyed by the (absent) battery label + tooltip.
        "phone-symbolic"
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

/// Full probe: daemon presence → managed devices → per-device battery
/// + connectivity for the paired/reachable ones.
pub(crate) async fn probe() -> ValentReport {
    if !daemon_available().await {
        return ValentReport::default();
    }

    let managed = match run(&[
        "call",
        "--session",
        "--dest",
        DEST,
        "--object-path",
        ROOT_PATH,
        "--method",
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

    devices.sort_by_key(|a| a.name.to_lowercase());

    ValentReport {
        daemon_available: true,
        devices,
    }
}

async fn daemon_available() -> bool {
    match run(&[
        "call",
        "--session",
        "--dest",
        "org.freedesktop.DBus",
        "--object-path",
        "/org/freedesktop/DBus",
        "--method",
        "org.freedesktop.DBus.ListNames",
    ])
    .await
    {
        Ok(out) => out.contains(DEST),
        Err(_) => false,
    }
}

fn parse_devices(raw: &str) -> Vec<Device> {
    static DEV_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"objectpath\s+'([^']+/Device/[^']+)'").expect("valent device regex")
    });
    static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'Id':\s*<").unwrap());
    static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'Name':\s*<").unwrap());
    static STATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'State':\s*<uint32\s+(\d+)>").unwrap());
    const DEVICE_IFACE: &str = "'ca.andyholmes.Valent.Device'";

    // Each device object runs from its `objectpath '…'` to the next one.
    let objects: Vec<(usize, usize, &str)> = DEV_RE
        .captures_iter(raw)
        .filter_map(|cap| {
            let whole = cap.get(0)?;
            Some((whole.start(), whole.end(), cap.get(1)?.as_str()))
        })
        .collect();

    let mut devices = Vec::new();
    for (i, &(_, body_start, path)) in objects.iter().enumerate() {
        let body_end = objects.get(i + 1).map_or(raw.len(), |o| o.0);
        let body = &raw[body_start..body_end];
        // The Device interface's property dict, wherever it sits among the
        // object's interfaces.
        let Some(props) = body.find(DEVICE_IFACE).and_then(|at| {
            let after = &body[at + DEVICE_IFACE.len()..];
            gvariant_dict_body(&after[after.find('{')?..])
        }) else {
            continue;
        };

        let id = gvariant_string_after(props, &ID_RE)
            .unwrap_or_else(|| path.split("/Device/").nth(1).unwrap_or(path).to_string());
        let name = gvariant_string_after(props, &NAME_RE).unwrap_or_else(|| id.clone());
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

/// The GVariant string literal right after the first match of `key_re`
/// (which ends just before the literal), unescaped.
fn gvariant_string_after(text: &str, key_re: &Regex) -> Option<String> {
    let m = key_re.find(text)?;
    gvariant_string_literal(&text[m.end()..])
}

/// Parse a GVariant text-format string literal at the start of `s`:
/// `'…'`, or `"…"` (what `g_variant_print` uses when the string contains
/// a `'`), with backslash escapes. `None` if it isn't one or never ends.
fn gvariant_string_literal(s: &str) -> Option<String> {
    let quote = s.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let mut out = String::new();
    let mut chars = s.chars().skip(1);
    while let Some(c) = chars.next() {
        if c == quote {
            return Some(out);
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let esc = chars.next()?;
        let hex_digits = match esc {
            'u' => 4,
            'U' => 8,
            _ => 0,
        };
        if hex_digits == 0 {
            out.push(match esc {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                'a' => '\u{7}',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'v' => '\u{b}',
                other => other,
            });
            continue;
        }
        let mut code = 0u32;
        for _ in 0..hex_digits {
            code = code * 16 + chars.next()?.to_digit(16)?;
        }
        out.push(char::from_u32(code)?);
    }
    None
}

/// Contents of the `{…}` block at the start of `s`, skipping braces that
/// sit inside quoted strings. `None` when `s` doesn't start with `{` or
/// the block never closes.
fn gvariant_dict_body(s: &str) -> Option<&str> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `org.gtk.Actions.Describe battery.state` →
/// `(… [<{'charging': <false>, 'percentage': <64.0>, …}>] …)`.
async fn fetch_battery(dev: &mut Device) {
    let path = device_path(&dev.id);
    let Ok(out) = describe(&path, "battery.state").await else {
        return;
    };
    let (charge, charging) = parse_battery(&out);
    if charge.is_some() {
        dev.battery_charge = charge;
    }
    if let Some(charging) = charging {
        dev.battery_charging = charging;
    }
}

/// `(percentage, charging)` from a `battery.state` Describe reply.
fn parse_battery(out: &str) -> (Option<i32>, Option<bool>) {
    static PCT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'percentage':\s*<([0-9.]+)>").unwrap());
    static CHG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'charging':\s*<(true|false)>").unwrap());
    let charge = PCT_RE
        .captures(out)
        .and_then(|c| c[1].parse::<f64>().ok())
        .map(|pct| pct.round() as i32);
    let charging = CHG_RE.captures(out).map(|c| &c[1] == "true");
    (charge, charging)
}

/// `org.gtk.Actions.Describe connectivity_report.state` →
/// `… {'network-type': <'LTE'>, 'signal-strength': <int64 3>} …`.
async fn fetch_connectivity(dev: &mut Device) {
    let path = device_path(&dev.id);
    let Ok(out) = describe(&path, "connectivity_report.state").await else {
        return;
    };
    let (network_type, strength) = parse_connectivity(&out);
    if let Some(t) = network_type {
        dev.network_type = t;
    }
    if let Some(s) = strength {
        dev.network_strength = s;
    }
}

/// `(network type, signal strength)` from a `connectivity_report.state`
/// Describe reply. An unparsable strength reads as `-1` (unknown).
fn parse_connectivity(out: &str) -> (Option<String>, Option<i32>) {
    static TYPE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'network-type':\s*<'([^']+)'>").unwrap());
    static SIG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'signal-strength':\s*<(?:int64\s+)?(\d+)>").unwrap());
    let network_type = TYPE_RE.captures(out).map(|c| c[1].to_string());
    let strength = SIG_RE.captures(out).map(|c| c[1].parse().unwrap_or(-1));
    (network_type, strength)
}

async fn describe(path: &str, action: &str) -> Result<String, String> {
    run(&[
        "call",
        "--session",
        "--dest",
        DEST,
        "--object-path",
        path,
        "--method",
        "org.gtk.Actions.Describe",
        action,
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
        "call",
        "--session",
        "--dest",
        DEST,
        "--object-path",
        &path,
        "--method",
        "org.gtk.Actions.Activate",
        action,
        param,
        "{}",
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
    activate_param(&device_id, "share.uri", &quoted_string_param(&uri)).await;
}

/// Share arbitrary text with the device (`share.text` GAction).
pub(crate) async fn share_text(device_id: String, text: String) {
    activate_param(&device_id, "share.text", &quoted_string_param(&text)).await;
}

/// Push the local clipboard to the device.
pub(crate) async fn clipboard_push(device_id: String) {
    activate(&device_id, "clipboard.push").await;
}

/// Pull the device's clipboard into the local one.
pub(crate) async fn clipboard_pull(device_id: String) {
    activate(&device_id, "clipboard.pull").await;
}

/// Build a GVariant `av` param holding one double-quoted string,
/// escaped so embedded backslashes/quotes are safe.
fn quoted_string_param(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("[<\"{escaped}\">]")
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
    match tokio::process::Command::new("gdbus")
        .args(args)
        .output()
        .await
    {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(format!("gdbus spawn: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gdbus call … GetManagedObjects` output for a paired, connected
    /// phone and a discovered, unpaired laptop.
    const MANAGED: &str = "({objectpath '/ca/andyholmes/Valent/Device/3f2a91c0': \
        {'ca.andyholmes.Valent.Device': {'IconName': <'phone-symbolic'>, \
        'Id': <'3f2a91c0'>, 'Name': <'Pixel 7'>, 'State': <uint32 3>, 'Type': <'phone'>}}, \
        objectpath '/ca/andyholmes/Valent/Device/laptop_2d01': \
        {'ca.andyholmes.Valent.Device': {'IconName': <'computer-symbolic'>, \
        'Id': <'laptop-01'>, 'Name': <'ThinkPad'>, 'State': <uint32 1>, 'Type': <'laptop'>}}},)";

    #[test]
    fn parses_managed_objects() {
        let devices = parse_devices(MANAGED);
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
            let raw = format!(
                "({{objectpath '/ca/andyholmes/Valent/Device/x': \
                 {{'ca.andyholmes.Valent.Device': {{'Id': <'x'>, 'State': <uint32 {state}>}}}}}},)"
            );
            let d = parse_devices(&raw).pop().unwrap();
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
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/abc_2d1': \
                   {'ca.andyholmes.Valent.Device': {'IconName': <'phone-symbolic'>}}},)";
        let d = parse_devices(raw).pop().unwrap();
        assert_eq!(d.id, "abc_2d1", "id falls back to the object-path tail");
        assert_eq!(d.name, d.id, "name falls back to the id");
        assert!(!d.reachable && !d.paired, "no State means not connected");
    }

    #[test]
    fn no_devices_or_garbage_yields_nothing() {
        assert!(parse_devices("(@a{oa{sa{sv}}} {},)").is_empty());
        assert!(parse_devices("").is_empty());
        assert!(
            parse_devices("Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown")
                .is_empty()
        );
        // The manager's own path is not a device.
        assert!(
            parse_devices("({objectpath '/ca/andyholmes/Valent': {'org.gtk.Actions': {}}},)")
                .is_empty()
        );
    }

    #[test]
    fn names_with_apostrophes_are_parsed() {
        // g_variant_print switches to double quotes when the string
        // contains a `'`; the old `'([^']+)'` regex fell back to the id.
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/p1': \
                   {'ca.andyholmes.Valent.Device': {'Id': <'p1'>, \
                   'Name': <\"Kenan's Pixel\">, 'State': <uint32 3>}}},)";
        assert_eq!(parse_devices(raw)[0].name, "Kenan's Pixel");
        // Both quote kinds: single-quoted with `\'`.
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/p2': \
                   {'ca.andyholmes.Valent.Device': {'Id': <'p2'>, \
                   'Name': <'Bob\\'s \"work\" phone'>, 'State': <uint32 1>}}},)";
        assert_eq!(parse_devices(raw)[0].name, "Bob's \"work\" phone");
    }

    #[test]
    fn names_with_braces_and_non_ascii_are_parsed() {
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/p3': \
                   {'ca.andyholmes.Valent.Device': {'Id': <'p3'>, \
                   'Name': <'Çağrı {tab}'>, 'State': <uint32 3>}}},)";
        let d = parse_devices(raw).pop().unwrap();
        assert_eq!(d.name, "Çağrı {tab}");
        assert!(d.reachable && d.paired, "props after the brace still read");
        assert_eq!(
            gvariant_string_literal(r"'tab\there \u00e7'").as_deref(),
            Some("tab\there ç")
        );
    }

    #[test]
    fn device_interface_need_not_come_first() {
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/p4': \
                   {'org.freedesktop.DBus.Properties': {}, \
                   'ca.andyholmes.Valent.Device': {'Id': <'p4'>, 'Name': <'Tab'>, \
                   'State': <uint32 2>}, \
                   'org.example.Other': {'Name': <'not the device name'>}}},)";
        let d = parse_devices(raw).pop().unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str()), ("p4", "Tab"));
        assert!(d.paired && !d.reachable);
    }

    #[test]
    fn objects_without_the_device_interface_are_skipped() {
        let raw = "({objectpath '/ca/andyholmes/Valent/Device/x': \
                   {'org.freedesktop.DBus.Properties': {}}, \
                   objectpath '/ca/andyholmes/Valent/Device/y': \
                   {'ca.andyholmes.Valent.Device': {'Id': <'y'>}}},)";
        let ids: Vec<_> = parse_devices(raw).into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["y"]);
    }

    #[test]
    fn unterminated_literals_and_blocks_are_rejected() {
        assert_eq!(gvariant_string_literal("'open"), None);
        assert_eq!(gvariant_string_literal("bare"), None);
        assert_eq!(gvariant_string_literal(r"'\u12'"), None);
        assert_eq!(gvariant_dict_body("{'a': <'}'>"), None);
        assert_eq!(gvariant_dict_body("x{}"), None);
        assert_eq!(gvariant_dict_body("{'a': <'}'>} tail"), Some("'a': <'}'>"));
    }

    #[test]
    fn battery_reply() {
        let out =
            "([<{'charging': <true>, 'percentage': <63.6>, 'icon-name': <'battery-good'>}>],)";
        assert_eq!(parse_battery(out), (Some(64), Some(true)));
        let out = "([<{'charging': <false>, 'percentage': <100.0>}>],)";
        assert_eq!(parse_battery(out), (Some(100), Some(false)));
        assert_eq!(parse_battery("([],)"), (None, None));
    }

    #[test]
    fn connectivity_reply() {
        let out = "([<{'network-type': <'LTE'>, 'signal-strength': <int64 3>}>],)";
        assert_eq!(parse_connectivity(out), (Some("LTE".into()), Some(3)));
        let out = "([<{'network-type': <'5G'>, 'signal-strength': <4>}>],)";
        assert_eq!(parse_connectivity(out), (Some("5G".into()), Some(4)));
        assert_eq!(parse_connectivity("([],)"), (None, None));
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
    fn string_param_is_escaped() {
        assert_eq!(quoted_string_param("hi"), r#"[<"hi">]"#);
        assert_eq!(quoted_string_param(r#"say "x""#), r#"[<"say \"x\"">]"#);
        assert_eq!(quoted_string_param(r"C:\dir"), r#"[<"C:\\dir">]"#);
        assert_eq!(quoted_string_param("it's"), r#"[<"it's">]"#);
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
