//! Wi-Fi hotspot over NetworkManager: start/stop an access point with
//! `nmcli device wifi hotspot` and list the connected clients
//! (`iw dev <if> station dump` merged with `ip neigh` for their IPs).
//!
//! Shared by the Network menu's Hotspot section and the Control Center
//! Hotspot tile. Everything that parses tool output is a pure function
//! tested against output captured on the owner's machine; the async
//! wrappers only spawn the tools and feed the parsers.
//!
//! The AP profile is created under a fixed connection name
//! ([`CONNECTION_NAME`]) so stop/inspect never has to guess, but a
//! hotspot started elsewhere is still detected: any active
//! `802-11-wireless` connection whose mode is `ap` counts.
//!
//! The password is never in the shell config or on an argv: it is a
//! secret of the `margo-hotspot` NetworkManager profile, set over D-Bus
//! ([`crate::hotspot_secret`], Settings → Network → Hotspot). Starting
//! activates that profile (`nmcli connection up`, no secret involved);
//! without a profile `nmcli device wifi hotspot` creates one and
//! NetworkManager generates the key. A password still in the old
//! `network.hotspot_password` key is moved into the profile once and
//! cleared from the config ([`migrate_config_password`]).

use std::collections::BTreeMap;
use std::time::Duration;
use tokio::process::Command;

/// NetworkManager connection name of the AP profile margo creates.
pub const CONNECTION_NAME: &str = "margo-hotspot";

/// A `nmcli -t -f DEVICE,TYPE,STATE device status` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NmDevice {
    pub name: String,
    pub kind: String,
    pub state: String,
}

/// A `nmcli -t -f NAME,TYPE,DEVICE connection show --active` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveConnection {
    pub name: String,
    pub kind: String,
    pub device: String,
}

/// One station from `iw dev <if> station dump`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Station {
    pub mac: String,
    pub signal_dbm: Option<i32>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub connected_secs: Option<u64>,
}

/// A connected hotspot client (station data + its IP when known).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HotspotClient {
    pub mac: String,
    pub ip: Option<String>,
    pub signal_dbm: Option<i32>,
    pub connected_secs: Option<u64>,
}

/// The running access point.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveHotspot {
    pub connection: String,
    pub interface: String,
    pub ssid: String,
    /// The AP's own IPv4 address with prefix (e.g. `10.42.0.1/24`).
    pub address: String,
}

/// Everything the UI needs for one refresh.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HotspotStatus {
    /// `nmcli` is installed.
    pub available: bool,
    /// The Wi-Fi interface a hotspot would start on.
    pub wifi_interface: Option<String>,
    pub active: Option<ActiveHotspot>,
    pub clients: Vec<HotspotClient>,
}

impl HotspotStatus {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Tile / row subtitle: `Off`, `On · 2 clients`, `Unavailable`.
    pub fn subtitle(&self) -> String {
        if !self.available {
            return "Unavailable".to_string();
        }
        if self.wifi_interface.is_none() && self.active.is_none() {
            return "No Wi-Fi device".to_string();
        }
        match (&self.active, self.clients.len()) {
            (None, _) => "Off".to_string(),
            (Some(_), 0) => "On · no clients".to_string(),
            (Some(_), 1) => "On · 1 client".to_string(),
            (Some(_), n) => format!("On · {n} clients"),
        }
    }
}

/// Split one `nmcli -t` line on unescaped `:` and unescape `\:` / `\\`
/// (an SSID or connection name may itself contain a colon).
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            ':' => fields.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    fields.push(cur);
    fields
}

pub fn parse_device_status(out: &str) -> Vec<NmDevice> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let f = split_terse(l);
            (f.len() >= 3).then(|| NmDevice {
                name: f[0].clone(),
                kind: f[1].clone(),
                state: f[2].clone(),
            })
        })
        .collect()
}

/// The interface to start the hotspot on: the configured one when set,
/// else the first real Wi-Fi device (`wifi`, not `wifi-p2p`) that is not
/// `unavailable` / `unmanaged`.
pub fn pick_wifi_interface(devices: &[NmDevice], configured: &str) -> Option<String> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    devices
        .iter()
        .find(|d| d.kind == "wifi" && d.state != "unavailable" && d.state != "unmanaged")
        .map(|d| d.name.clone())
}

pub fn parse_active_connections(out: &str) -> Vec<ActiveConnection> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let f = split_terse(l);
            (f.len() >= 3).then(|| ActiveConnection {
                name: f[0].clone(),
                kind: f[1].clone(),
                device: f[2].clone(),
            })
        })
        .collect()
}

/// Parse `nmcli -g 802-11-wireless.mode,802-11-wireless.ssid,IP4.ADDRESS
/// connection show <name>` (one value per line, in field order). Returns
/// `(mode, ssid, ipv4)`.
pub fn parse_connection_details(out: &str) -> (String, String, String) {
    let mut lines = out.lines().map(|l| l.trim().to_string());
    let mode = lines.next().unwrap_or_default();
    let ssid = lines.next().unwrap_or_default();
    // IP4.ADDRESS may list several addresses separated by " | ".
    let ip = lines
        .next()
        .unwrap_or_default()
        .split(" | ")
        .next()
        .unwrap_or_default()
        .to_string();
    (mode, ssid, ip)
}

pub fn parse_iw_stations(out: &str) -> Vec<Station> {
    let mut stations: Vec<Station> = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("Station ") {
            let mac = rest.split_whitespace().next().unwrap_or_default();
            stations.push(Station {
                mac: mac.to_ascii_lowercase(),
                ..Station::default()
            });
            continue;
        }
        let Some(cur) = stations.last_mut() else {
            continue;
        };
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        let first = value.split_whitespace().next().unwrap_or_default();
        match key.trim() {
            "signal" => cur.signal_dbm = first.parse().ok(),
            "rx bytes" => cur.rx_bytes = first.parse().unwrap_or(0),
            "tx bytes" => cur.tx_bytes = first.parse().unwrap_or(0),
            "connected time" => cur.connected_secs = first.parse().ok(),
            _ => {}
        }
    }
    stations
}

/// `ip neigh show dev <if>` → MAC → IPv4. Entries in `FAILED` /
/// `INCOMPLETE` state carry no `lladdr` and are skipped.
pub fn parse_ip_neigh(out: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some(pos) = parts.iter().position(|p| *p == "lladdr") else {
            continue;
        };
        let (Some(ip), Some(mac)) = (parts.first(), parts.get(pos + 1)) else {
            continue;
        };
        if ip.contains('.') {
            map.insert(mac.to_ascii_lowercase(), (*ip).to_string());
        }
    }
    map
}

/// Associated stations are the source of truth for "connected"; `ip
/// neigh` only contributes their IPs (a stale neighbour entry for a
/// client that already left must not resurrect it). Sorted by IP, then
/// MAC, so the list doesn't reshuffle between refreshes.
pub fn merge_clients(stations: &[Station], neigh: &BTreeMap<String, String>) -> Vec<HotspotClient> {
    let mut clients: Vec<HotspotClient> = stations
        .iter()
        .map(|s| HotspotClient {
            mac: s.mac.clone(),
            ip: neigh.get(&s.mac).cloned(),
            signal_dbm: s.signal_dbm,
            connected_secs: s.connected_secs,
        })
        .collect();
    clients.sort_by(|a, b| {
        let key = |c: &HotspotClient| {
            c.ip.as_deref()
                .and_then(|ip| ip.parse::<std::net::Ipv4Addr>().ok())
                .map(u32::from)
                .unwrap_or(u32::MAX)
        };
        key(a).cmp(&key(b)).then_with(|| a.mac.cmp(&b.mac))
    });
    clients
}

/// SSID to broadcast: the configured one, else `<hostname>-hotspot`.
pub fn effective_ssid(configured: &str, hostname: &str) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        return configured.to_string();
    }
    let host = hostname.trim();
    if host.is_empty() {
        "margo-hotspot".to_string()
    } else {
        format!("{host}-hotspot")
    }
}

/// WPA2 rules NetworkManager would reject later with a less helpful
/// message: SSID 1–32 bytes, password empty (NM keeps/generates one) or
/// 8–63 characters.
pub fn validate(ssid: &str, password: &str) -> Result<(), String> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("Hotspot name must be 1–32 bytes".to_string());
    }
    let n = password.chars().count();
    if n != 0 && !(8..=63).contains(&n) {
        return Err("Hotspot password must be 8–63 characters".to_string());
    }
    Ok(())
}

/// `nmcli` argv creating and starting the AP when no `margo-hotspot`
/// profile exists yet: NetworkManager generates the key.
pub fn create_args(iface: &str, ssid: &str) -> Vec<String> {
    [
        "device",
        "wifi",
        "hotspot",
        "ifname",
        iface,
        "con-name",
        CONNECTION_NAME,
        "ssid",
        ssid,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `nmcli` argv pointing the existing profile at the configured SSID (and
/// keeping it off autoconnect) before it is started.
pub fn retarget_args(ssid: &str) -> Vec<String> {
    [
        "connection",
        "modify",
        CONNECTION_NAME,
        "802-11-wireless.ssid",
        ssid,
        "connection.autoconnect",
        "no",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `nmcli` argv starting the existing profile on `iface`; the key is the
/// profile's stored secret.
pub fn up_args(iface: &str) -> Vec<String> {
    ["connection", "up", CONNECTION_NAME, "ifname", iface]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Client row title: its IP when known, else the MAC.
pub fn client_title(c: &HotspotClient) -> String {
    c.ip.clone().unwrap_or_else(|| c.mac.clone())
}

/// Client row detail: `MAC · -50 dBm · 30 min` (parts omitted when
/// unknown; the MAC is omitted when it is already the title).
pub fn client_detail(c: &HotspotClient) -> String {
    let mut parts = Vec::new();
    if c.ip.is_some() {
        parts.push(c.mac.clone());
    }
    if let Some(dbm) = c.signal_dbm {
        parts.push(format!("{dbm} dBm"));
    }
    if let Some(secs) = c.connected_secs {
        parts.push(match secs {
            0..=59 => format!("{secs} s"),
            60..=3599 => format!("{} min", secs / 60),
            _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
        });
    }
    parts.join(" · ")
}

// ── Async wrappers ─────────────────────────────────────────────────────────

const NMCLI_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(30);

async fn run(bin: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let fut = Command::new(bin).args(args).kill_on_drop(true).output();
    match tokio::time::timeout(timeout, fut).await {
        Err(_) => Err(format!("{bin} timed out")),
        Ok(Err(e)) => Err(format!("{bin}: {e}")),
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(Ok(o)) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if err.is_empty() {
                format!("{bin} exited with {}", o.status)
            } else {
                err
            })
        }
    }
}

/// Find the running AP among the active Wi-Fi connections.
async fn find_active() -> Option<ActiveHotspot> {
    let out = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "NAME,TYPE,DEVICE",
            "connection",
            "show",
            "--active",
        ],
        NMCLI_TIMEOUT,
    )
    .await
    .ok()?;
    for conn in parse_active_connections(&out)
        .into_iter()
        .filter(|c| c.kind == "802-11-wireless")
    {
        let Ok(details) = run(
            "nmcli",
            &[
                "-g",
                "802-11-wireless.mode,802-11-wireless.ssid,IP4.ADDRESS",
                "connection",
                "show",
                &conn.name,
            ],
            NMCLI_TIMEOUT,
        )
        .await
        else {
            continue;
        };
        let (mode, ssid, address) = parse_connection_details(&details);
        if mode == "ap" {
            return Some(ActiveHotspot {
                connection: conn.name,
                interface: conn.device,
                ssid,
                address,
            });
        }
    }
    None
}

/// Connected clients of the AP on `iface` (empty when `iw` is missing).
pub async fn fetch_clients(iface: &str) -> Vec<HotspotClient> {
    if !margo_fs::path::on_path("iw") {
        return Vec::new();
    }
    let stations = run("iw", &["dev", iface, "station", "dump"], NMCLI_TIMEOUT)
        .await
        .map(|o| parse_iw_stations(&o))
        .unwrap_or_default();
    let neigh = if margo_fs::path::on_path("ip") {
        run("ip", &["neigh", "show", "dev", iface], NMCLI_TIMEOUT)
            .await
            .map(|o| parse_ip_neigh(&o))
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    merge_clients(&stations, &neigh)
}

/// One full refresh: availability, interface, active AP and its clients.
pub async fn probe(configured_iface: &str) -> HotspotStatus {
    if !margo_fs::path::on_path("nmcli") {
        return HotspotStatus::default();
    }
    let devices = run(
        "nmcli",
        &["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"],
        NMCLI_TIMEOUT,
    )
    .await
    .map(|o| parse_device_status(&o))
    .unwrap_or_default();
    let active = find_active().await;
    let clients = match &active {
        Some(a) if !a.interface.is_empty() => fetch_clients(&a.interface).await,
        _ => Vec::new(),
    };
    HotspotStatus {
        available: true,
        wifi_interface: pick_wifi_interface(&devices, configured_iface),
        active,
        clients,
    }
}

/// Start the AP. `ssid` should already be [`effective_ssid`]-resolved.
/// An existing `margo-hotspot` profile is retargeted to `ssid` and started
/// with its stored key; otherwise nmcli creates one (generated key). The
/// profile is set to not auto-connect, so a reboot doesn't silently turn
/// the laptop into an access point.
pub async fn start(iface: &str, ssid: &str) -> Result<(), String> {
    validate(ssid, "")?;
    let nmcli = |args: Vec<String>, timeout| async move {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        run("nmcli", &argv, timeout).await
    };
    if crate::hotspot_secret::profile_exists()
        .await
        .unwrap_or(false)
    {
        nmcli(retarget_args(ssid), NMCLI_TIMEOUT).await?;
        nmcli(up_args(iface), START_TIMEOUT).await?;
        return Ok(());
    }
    nmcli(create_args(iface, ssid), START_TIMEOUT).await?;
    let _ = run(
        "nmcli",
        &[
            "connection",
            "modify",
            CONNECTION_NAME,
            "connection.autoconnect",
            "no",
        ],
        NMCLI_TIMEOUT,
    )
    .await;
    Ok(())
}

/// Stop the AP connection (the one found by [`probe`]).
pub async fn stop(connection: &str) -> Result<(), String> {
    run("nmcli", &["connection", "down", connection], NMCLI_TIMEOUT)
        .await
        .map(|_| ())
}

// ── Config-aware entry points (Network menu + Control Center tile) ─────────

/// `network.hotspot_interface` from the shell config.
pub fn configured_interface() -> String {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, NetworkConfigStoreFields};
    use reactive_graph::traits::GetUntracked;
    config_manager()
        .config()
        .network()
        .hotspot_interface()
        .get_untracked()
}

/// Flip the hotspot: stop `active` when one runs, else start one on
/// `wifi_interface` with the configured SSID/password. Failures are also
/// surfaced as a toast (the tile and the menu both call this).
pub async fn toggle(
    active: Option<ActiveHotspot>,
    wifi_interface: Option<String>,
) -> Result<(), String> {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, NetworkConfigStoreFields};
    use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
    use reactive_graph::traits::GetUntracked;

    let result = match (active, wifi_interface) {
        (Some(a), _) => stop(&a.connection).await,
        (None, Some(iface)) => {
            let net = config_manager().config().network();
            let ssid = effective_ssid(&net.hotspot_ssid().get_untracked(), &hostname());
            // A password still in the config goes into the profile first;
            // if that fails, do not start with a different key.
            match migrate_config_password().await {
                Ok(()) => start(&iface, &ssid).await,
                Err(e) => Err(e),
            }
        }
        (None, None) => Err("No Wi-Fi device for the hotspot".to_string()),
    };
    if let Err(e) = &result {
        tracing::warn!(error = %e, "hotspot toggle failed");
        push_toast(ToastEvent {
            icon: "network-wireless-hotspot-symbolic".to_string(),
            title: "Hotspot failed".to_string(),
            body: Some(e.clone()),
            severity: ToastSeverity::Warn,
        });
    }
    result
}

// Serialise migration and explicit Settings saves, so a slow migration can
// never overwrite a newer password the user just saved.
static SECRET_CHANGES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Save a new password from Settings in NetworkManager, and clear any legacy
/// config value only after the save succeeds. Never writes the new key to YAML.
pub async fn set_password(password: &str) -> Result<(), String> {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, NetworkConfigStoreFields};
    use reactive_graph::traits::GetUntracked;

    let _guard = SECRET_CHANGES.lock().await;
    let ssid = effective_ssid(
        &config_manager()
            .config()
            .network()
            .hotspot_ssid()
            .get_untracked(),
        &hostname(),
    );
    crate::hotspot_secret::store_password(&ssid, password).await?;
    clear_legacy_password().await
}

async fn clear_legacy_password() -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    relm4::gtk::glib::MainContext::default().invoke(move || {
        let manager = mshell_config::config_manager::config_manager();
        manager.update_config(|c| c.network.hotspot_password.clear());
        // Do not leave the old plaintext waiting in the debounced writer
        // after reporting a successful migration.
        manager.flush();
        let _ = tx.send(());
    });
    rx.await
        .map_err(|_| "Could not clear the legacy hotspot setting".to_string())
}

/// Move a password still in the old `network.hotspot_password` key into the
/// NetworkManager profile, then clear the key (on the GTK main loop, where
/// the config store lives). A no-op when the key is empty; on failure the
/// key is kept so the next start retries.
pub async fn migrate_config_password() -> Result<(), String> {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, NetworkConfigStoreFields};
    use reactive_graph::traits::GetUntracked;

    let _guard = SECRET_CHANGES.lock().await;
    let net = config_manager().config().network();
    let password = zeroize::Zeroizing::new(net.clone().hotspot_password().get_untracked());
    if password.is_empty() {
        return Ok(());
    }
    let ssid = effective_ssid(&net.hotspot_ssid().get_untracked(), &hostname());
    crate::hotspot_secret::store_password(&ssid, &password)
        .await
        .map_err(|e| format!("Could not save the hotspot password in NetworkManager: {e}"))?;
    clear_legacy_password().await?;
    tracing::info!("hotspot: moved the password from the shell config into NetworkManager");
    Ok(())
}

/// The machine's hostname, for the default SSID.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured on the owner's ThinkPad (NetworkManager 1.5x).
    const DEVICE_STATUS: &str = "wlan0:wifi:connected\n\
lo:loopback:connected (externally)\n\
wg0-mullvad:wireguard:connected (externally)\n\
p2p-dev-wlan0:wifi-p2p:disconnected\n\
enp0s13f0u3u2:ethernet:unavailable\n\
enp0s31f6:ethernet:unavailable\n";

    const ACTIVE: &str = "Ken_5:802-11-wireless:wlan0\n\
lo:loopback:lo\n\
wg0-mullvad:wireguard:wg0-mullvad\n";

    // `iw dev wlan0 station dump` (MAC anonymised), managed mode — the
    // block format is identical in AP mode, one block per client.
    const STATION_DUMP: &str = "Station aa:bb:cc:dd:ee:01 (on wlan0)\n\
\tauthorized:\tyes\n\
\tinactive time:\t5 ms\n\
\trx bytes:\t172478056\n\
\trx packets:\t340194\n\
\ttx bytes:\t506445705\n\
\tsignal:  \t-50 [-50, -51] dBm\n\
\tsignal avg:\t-52 dBm\n\
\tbeacon signal avg:\t-53 dBm\n\
\ttx bitrate:\t866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n\
\tconnected time:\t1843 seconds\n\
Station AA:BB:CC:DD:EE:02 (on wlan0)\n\
\trx bytes:\t10\n\
\ttx bytes:\t20\n\
\tsignal:  \t-71 dBm\n";

    const NEIGH: &str = "10.42.0.73 lladdr aa:bb:cc:dd:ee:02 REACHABLE \n\
10.42.0.12 lladdr aa:bb:cc:dd:ee:01 DELAY \n\
10.42.0.99 lladdr aa:bb:cc:dd:ee:ff STALE \n\
10.42.0.50  FAILED\n\
fe80::1 lladdr aa:bb:cc:dd:ee:01 router REACHABLE\n";

    #[test]
    fn terse_split_unescapes_colons() {
        assert_eq!(
            split_terse(r"My\:Net:802-11-wireless:wlan0"),
            vec!["My:Net", "802-11-wireless", "wlan0"]
        );
        assert_eq!(split_terse(r"a\\b:c"), vec![r"a\b", "c"]);
    }

    #[test]
    fn picks_real_wifi_not_p2p() {
        let devs = parse_device_status(DEVICE_STATUS);
        assert_eq!(devs.len(), 6);
        assert_eq!(pick_wifi_interface(&devs, ""), Some("wlan0".to_string()));
        assert_eq!(
            pick_wifi_interface(&devs, " wlp2s0 "),
            Some("wlp2s0".to_string())
        );
        let no_wifi = parse_device_status("enp0s31f6:ethernet:connected\n");
        assert_eq!(pick_wifi_interface(&no_wifi, ""), None);
    }

    #[test]
    fn active_connections_parse() {
        let a = parse_active_connections(ACTIVE);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].name, "Ken_5");
        assert_eq!(a[0].kind, "802-11-wireless");
        assert_eq!(a[0].device, "wlan0");
    }

    #[test]
    fn connection_details_mode_ssid_ip() {
        let (m, s, ip) = parse_connection_details("infrastructure\nKen_5\n192.168.0.100/24\n");
        assert_eq!(
            (m.as_str(), s.as_str(), ip.as_str()),
            ("infrastructure", "Ken_5", "192.168.0.100/24")
        );
        let (m, _, ip) =
            parse_connection_details("ap\nlaptop-hotspot\n10.42.0.1/24 | 10.0.0.1/8\n");
        assert_eq!(m, "ap");
        assert_eq!(ip, "10.42.0.1/24");
        assert_eq!(
            parse_connection_details(""),
            (String::new(), String::new(), String::new())
        );
    }

    #[test]
    fn station_dump_blocks() {
        let s = parse_iw_stations(STATION_DUMP);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].mac, "aa:bb:cc:dd:ee:01");
        assert_eq!(s[0].signal_dbm, Some(-50));
        assert_eq!(s[0].rx_bytes, 172_478_056);
        assert_eq!(s[0].tx_bytes, 506_445_705);
        assert_eq!(s[0].connected_secs, Some(1843));
        // MAC lower-cased; missing connected time stays None.
        assert_eq!(s[1].mac, "aa:bb:cc:dd:ee:02");
        assert_eq!(s[1].signal_dbm, Some(-71));
        assert_eq!(s[1].connected_secs, None);
        assert!(parse_iw_stations("").is_empty());
    }

    #[test]
    fn neigh_skips_failed_and_v6() {
        let n = parse_ip_neigh(NEIGH);
        assert_eq!(n.len(), 3);
        assert_eq!(
            n.get("aa:bb:cc:dd:ee:01").map(String::as_str),
            Some("10.42.0.12")
        );
    }

    #[test]
    fn merge_uses_stations_as_truth_and_sorts_by_ip() {
        let clients = merge_clients(&parse_iw_stations(STATION_DUMP), &parse_ip_neigh(NEIGH));
        // The stale aa:..:ff neighbour is not a client.
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].ip.as_deref(), Some("10.42.0.12"));
        assert_eq!(clients[1].ip.as_deref(), Some("10.42.0.73"));
        // A station without a neighbour entry sorts last with no IP.
        let only = merge_clients(&parse_iw_stations(STATION_DUMP), &BTreeMap::new());
        assert!(only.iter().all(|c| c.ip.is_none()));
    }

    #[test]
    fn ssid_default_and_validation() {
        assert_eq!(effective_ssid("", "thinkpad"), "thinkpad-hotspot");
        assert_eq!(effective_ssid("  Mine ", "thinkpad"), "Mine");
        assert_eq!(effective_ssid("", ""), "margo-hotspot");
        assert!(validate("x", "").is_ok());
        assert!(validate("x", "12345678").is_ok());
        assert!(validate("x", "short").is_err());
        assert!(validate("", "12345678").is_err());
        assert!(validate(&"s".repeat(33), "").is_err());
        assert!(validate("x", &"p".repeat(64)).is_err());
    }

    #[test]
    fn start_argvs_are_fixed_and_never_carry_a_secret() {
        assert_eq!(
            create_args("wlan0", "n"),
            [
                "device",
                "wifi",
                "hotspot",
                "ifname",
                "wlan0",
                "con-name",
                CONNECTION_NAME,
                "ssid",
                "n"
            ]
        );
        assert_eq!(
            retarget_args("n"),
            [
                "connection",
                "modify",
                CONNECTION_NAME,
                "802-11-wireless.ssid",
                "n",
                "connection.autoconnect",
                "no"
            ]
        );
        assert_eq!(
            up_args("wlan0"),
            ["connection", "up", CONNECTION_NAME, "ifname", "wlan0"]
        );
    }

    #[test]
    fn the_password_never_reaches_an_nmcli_argv() {
        // Any local user can read /proc/<pid>/cmdline. The secret goes to
        // NetworkManager over D-Bus only (hotspot_secret.rs).
        let src = include_str!("hotspot.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "\"password\"",
            "wifi-sec.psk",
            "802-11-wireless-security.psk",
        ] {
            assert!(
                !prod.contains(forbidden),
                "hotspot.rs passes {forbidden} to nmcli"
            );
        }
        // The only reader of the old config key is the one-time migration.
        assert_eq!(prod.matches("hotspot_password()").count(), 1);
    }

    #[test]
    fn client_row_text() {
        let mut c = HotspotClient {
            mac: "aa:bb".into(),
            ip: Some("10.42.0.12".into()),
            signal_dbm: Some(-50),
            connected_secs: Some(1843),
        };
        assert_eq!(client_title(&c), "10.42.0.12");
        assert_eq!(client_detail(&c), "aa:bb · -50 dBm · 30 min");
        c.ip = None;
        c.connected_secs = Some(7260);
        assert_eq!(client_title(&c), "aa:bb");
        assert_eq!(client_detail(&c), "-50 dBm · 2 h 1 min");
        c.signal_dbm = None;
        c.connected_secs = Some(5);
        assert_eq!(client_detail(&c), "5 s");
    }

    #[test]
    fn subtitles() {
        let mut st = HotspotStatus::default();
        assert_eq!(st.subtitle(), "Unavailable");
        st.available = true;
        assert_eq!(st.subtitle(), "No Wi-Fi device");
        st.wifi_interface = Some("wlan0".into());
        assert_eq!(st.subtitle(), "Off");
        st.active = Some(ActiveHotspot::default());
        assert_eq!(st.subtitle(), "On · no clients");
        st.clients = vec![HotspotClient::default()];
        assert_eq!(st.subtitle(), "On · 1 client");
        st.clients.push(HotspotClient::default());
        assert_eq!(st.subtitle(), "On · 2 clients");
    }
}
