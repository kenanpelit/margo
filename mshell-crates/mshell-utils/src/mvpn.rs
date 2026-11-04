//! Parsing for the `mvpn` CLI's output, shared by the VPN bar pill, the VPN
//! menu, Settings → VPN and the VPN toast. (Each used to carry its own
//! copy of a string-search JSON "parser" that broke on `"key": "v"` spacing
//! or an escaped quote.)

use crate::shared_poll::{EventFeed, SharedPoll};
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

/// `mvpn status --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct VpnStatus {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub relay: String,
    #[serde(default)]
    pub location: String,
}

impl VpnStatus {
    /// Parse `mvpn status --json`; unparseable or empty output (no `mvpn`,
    /// daemon down) reads as disconnected.
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json.trim()).unwrap_or_default()
    }

    /// `Connected · <relay>[ · <location>]` or `Disconnected`.
    pub fn summary(&self) -> String {
        if !self.connected {
            return "Disconnected".to_string();
        }
        let mut s = format!("Connected · {}", self.relay);
        if !self.location.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.location);
        }
        s
    }
}

/// What the shared VPN poll reports: the tunnel plus the blocky DNS guard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VpnProbe {
    pub status: VpnStatus,
    /// `blocky.service` (the DNS guard) is active.
    pub blocky: bool,
}

/// One VPN source for the whole shell, shared by every pill and the VPN
/// toast. `mvpn status --json --follow` streams Mullvad daemon changes;
/// the 5 s probe only reads the in-memory status (and the blocky D-Bus
/// property). With an older mvpn or a failed stream it falls back to the
/// old CLI probe until the stream can restart.
pub static VPN_POLL: LazyLock<SharedPoll<VpnProbe>> = LazyLock::new(|| {
    SharedPoll::new(|| Duration::from_secs(5), || Box::pin(probe_vpn())).with_feed(EventFeed::new(
        || {
            let mut cmd = tokio::process::Command::new("mvpn");
            cmd.args(["status", "--json", "--follow"]);
            cmd
        },
        accept_status_line,
        || Duration::from_secs(5),
        Duration::from_secs(30),
    ))
});

static STREAM_STATUS: std::sync::Mutex<Option<VpnStatus>> = std::sync::Mutex::new(None);

fn accept_status_line(line: &str) -> bool {
    let Ok(status) = serde_json::from_str::<VpnStatus>(line) else {
        return false;
    };
    if let Ok(mut cur) = STREAM_STATUS.lock() {
        *cur = Some(status);
    }
    true
}

/// The streamed status, while the stream runs. A stopped stream's last
/// status is dropped, so a restarted one never serves a stale state before
/// its first line (the probe asks the CLI until then).
fn streamed_status(alive: bool, cache: &mut Option<VpnStatus>) -> Option<VpnStatus> {
    if alive {
        cache.clone()
    } else {
        *cache = None;
        None
    }
}

/// Last known visible location per relay. `mullvad status listen` reports a
/// connect before the daemon has looked up the visible location and does not
/// report again when the lookup lands, so a streamed status can stay
/// location-less until the next tunnel change.
static RELAY_LOCATION: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// A streamed status that is connected but misses its relay or location
/// needs one full `mvpn status` query (the location is cached per relay).
fn needs_location(status: &VpnStatus) -> bool {
    status.connected && (status.location.is_empty() || status.relay.is_empty())
}

/// Take the missing relay/location from a full `mvpn status` answer that
/// describes the same tunnel (an empty streamed relay matches any).
fn fill_from(mut status: VpnStatus, full: VpnStatus) -> VpnStatus {
    if full.connected && (status.relay.is_empty() || full.relay == status.relay) {
        if status.relay.is_empty() {
            status.relay = full.relay;
        }
        if status.location.is_empty() {
            status.location = full.location;
        }
    }
    status
}

/// Fill `status`'s missing location from `known` (relay, location) when it
/// is for the same relay.
fn with_known_location(mut status: VpnStatus, known: Option<&(String, String)>) -> VpnStatus {
    if needs_location(&status)
        && let Some((relay, location)) = known
        && *relay == status.relay
    {
        status.location = location.clone();
    }
    status
}

async fn cli_status() -> VpnStatus {
    match tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await
    {
        Ok(out) => VpnStatus::parse(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => VpnStatus::default(),
    }
}

async fn probe_vpn() -> VpnProbe {
    let cached = STREAM_STATUS
        .lock()
        .ok()
        .and_then(|mut s| streamed_status(VPN_POLL.feed_alive(), &mut s));
    let mut status = match cached {
        Some(status) => status,
        None => cli_status().await,
    };
    let known = RELAY_LOCATION.lock().ok().and_then(|k| k.clone());
    status = with_known_location(status, known.as_ref());
    if needs_location(&status) {
        status = fill_from(status, cli_status().await);
    }
    if status.connected
        && !status.location.is_empty()
        && let Ok(mut k) = RELAY_LOCATION.lock()
    {
        *k = Some((status.relay.clone(), status.location.clone()));
    }
    // The blocky DNS guard: a D-Bus property read on the shared system-bus
    // connection, not a `systemctl` spawn.
    let blocky = match mshell_services::systemd::system_bus().await {
        Ok(conn) => mshell_services::systemd::unit_active(&conn, "blocky.service")
            .await
            .unwrap_or(false),
        Err(_) => false,
    };
    VpnProbe { status, blocky }
}

/// One `mvpn fav list` row: `<ping label>  <relay>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpnFav {
    pub relay: String,
    pub ping: String,
}

/// Parse `mvpn fav list` (the relay is the last token; everything before it
/// is the ping label).
pub fn parse_fav_list(out: &str) -> Vec<VpnFav> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let relay = line.split_whitespace().last()?.to_string();
            let ping = line[..line.len() - relay.len()].trim().to_string();
            Some(VpnFav { relay, ping })
        })
        .collect()
}

/// Value of `key` in `mvpn toggles` output (`key=value` lines); empty when
/// absent.
pub fn toggle_value(toggles: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    toggles
        .lines()
        .find_map(|l| l.trim().strip_prefix(prefix.as_str()))
        .unwrap_or("")
        .to_string()
}

// ── Tunnel throughput (VPN menu graph) ─────────────────────────────────────

/// The VPN tunnel among the kernel's interfaces (`/sys/class/net` names):
/// Mullvad's `wg0-mullvad` first, then any other Mullvad / WireGuard /
/// tun device. `None` = no tunnel up.
pub fn pick_tunnel_interface(names: &[String]) -> Option<String> {
    let rank = |n: &str| -> Option<u8> {
        if n == "wg0-mullvad" {
            Some(0)
        } else if n.contains("mullvad") {
            Some(1)
        } else if n.starts_with("wg") {
            Some(2)
        } else if n.starts_with("tun") {
            Some(3)
        } else {
            None
        }
    };
    names
        .iter()
        .filter_map(|n| rank(n).map(|r| (r, n)))
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
        .map(|(_, n)| n.clone())
}

/// Bytes/s between two counter readings. A counter that went backwards
/// (tunnel re-created between samples) reads as 0, not a huge spike.
pub fn counter_rate(prev: u64, cur: u64, secs: f64) -> u64 {
    if secs <= 0.0 || cur < prev {
        return 0;
    }
    ((cur - prev) as f64 / secs).round() as u64
}

/// `(rx_bytes, tx_bytes)` of `iface` from sysfs, `None` when it's gone.
pub fn read_iface_counters(iface: &str) -> Option<(u64, u64)> {
    let read = |f: &str| -> Option<u64> {
        std::fs::read_to_string(format!("/sys/class/net/{iface}/statistics/{f}"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

/// Interface names from `/sys/class/net`.
pub fn list_interfaces() -> Vec<String> {
    std::fs::read_dir("/sys/class/net")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_tunnel_interface() {
        // `ip -br link` on the owner's machine.
        let names: Vec<String> = ["lo", "wlan0", "enp0s13f0u3u2", "enp0s31f6", "wg0-mullvad"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            pick_tunnel_interface(&names).as_deref(),
            Some("wg0-mullvad")
        );
        let other: Vec<String> = ["lo", "tun0", "wg1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(pick_tunnel_interface(&other).as_deref(), Some("wg1"));
        let none: Vec<String> = ["lo", "wlan0"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_tunnel_interface(&none), None);
    }

    #[test]
    fn counter_rates() {
        assert_eq!(counter_rate(1_000, 3_000, 2.0), 1_000);
        assert_eq!(counter_rate(5_000, 100, 2.0), 0, "reset counter");
        assert_eq!(counter_rate(0, 100, 0.0), 0);
    }

    #[test]
    fn parses_compact_status() {
        let s = VpnStatus::parse(
            r#"{"connected":true,"relay":"de-fra-wg-002","location":"Germany, Frankfurt"}"#,
        );
        assert!(s.connected);
        assert_eq!(s.relay, "de-fra-wg-002");
        assert_eq!(
            s.summary(),
            "Connected · de-fra-wg-002 · Germany, Frankfurt"
        );
    }

    /// The old string search required `"key":"` exactly and stopped at the
    /// first quote; real JSON doesn't promise either.
    #[test]
    fn parses_spaced_and_escaped_status() {
        let s = VpnStatus::parse(
            "{\n  \"connected\": true,\n  \"relay\": \"se-sto\",\n  \"location\": \"A \\\"B\\\" C\"\n}",
        );
        assert!(s.connected);
        assert_eq!(s.relay, "se-sto");
        assert_eq!(s.location, "A \"B\" C");
    }

    #[test]
    fn missing_or_garbage_is_disconnected() {
        assert_eq!(VpnStatus::parse(""), VpnStatus::default());
        assert_eq!(
            VpnStatus::parse("mvpn: daemon not running"),
            VpnStatus::default()
        );
        let s = VpnStatus::parse(r#"{"connected":false}"#);
        assert_eq!(s.summary(), "Disconnected");
        let s = VpnStatus::parse(r#"{"connected":true,"relay":"x"}"#);
        assert_eq!(s.summary(), "Connected · x");
    }

    #[test]
    fn stream_status_is_used_only_while_the_stream_runs() {
        let up = VpnStatus::parse(r#"{"connected":true,"relay":"x"}"#);
        let mut cache = Some(up.clone());
        assert_eq!(streamed_status(true, &mut cache), Some(up));
        // Stream stopped: fall back to the CLI and forget the old value.
        assert_eq!(streamed_status(false, &mut cache), None);
        assert_eq!(streamed_status(true, &mut cache), None);
    }

    /// `mvpn status --json --follow` prints mvpn's full status struct;
    /// extra fields are ignored.
    #[test]
    fn parses_a_follow_line() {
        let s = VpnStatus::parse(
            r#"{"connected":true,"connecting":false,"state":"Connected","relay":"de-fra-wg-002","location":"Germany, Frankfurt. IPv4: 1.2.3.4","ipv4":"1.2.3.4","country":"Germany","city":"Frankfurt","tunnel_type":"WireGuard"}"#,
        );
        assert!(s.connected);
        assert_eq!(s.relay, "de-fra-wg-002");
    }

    /// The daemon's change stream reports a connect before the location is
    /// known and never again for the lookup; the last known location for the
    /// same relay fills the gap, a different relay's never does.
    #[test]
    fn a_streamed_connect_without_location_takes_the_relays_known_location() {
        let streamed =
            VpnStatus::parse(r#"{"connected":true,"relay":"de-fra-wg-002","location":""}"#);
        assert!(needs_location(&streamed));
        let known = (
            "de-fra-wg-002".to_string(),
            "Germany, Frankfurt. IPv4: 1.2.3.4".to_string(),
        );
        let filled = with_known_location(streamed.clone(), Some(&known));
        assert_eq!(filled.location, known.1);
        assert!(!needs_location(&filled));
        let other = ("se-sto-wg-001".to_string(), "Sweden, Stockholm".to_string());
        assert!(needs_location(&with_known_location(streamed, Some(&other))));
        // Disconnected never asks.
        assert!(!needs_location(&VpnStatus::default()));
    }

    /// An older mvpn streamed a reconnect's final block with neither relay
    /// nor location; the full status fills both.
    #[test]
    fn a_streamed_status_without_relay_is_filled_from_the_full_status() {
        let streamed = VpnStatus::parse(r#"{"connected":true,"relay":"","location":""}"#);
        assert!(needs_location(&streamed));
        let full = VpnStatus::parse(
            r#"{"connected":true,"relay":"de-fra-wg-002","location":"Germany, Frankfurt. IPv4: 1.2.3.4"}"#,
        );
        let filled = fill_from(streamed, full.clone());
        assert_eq!(filled, full);
        // A different relay's location is never borrowed.
        let other = VpnStatus::parse(r#"{"connected":true,"relay":"se-sto-wg-001","location":""}"#);
        assert_eq!(fill_from(other.clone(), full).location, "");
    }

    #[test]
    fn parses_fav_list() {
        let favs = parse_fav_list("     45 ms  de-fra-wg-002\n    N/A    fr-par-wg-001\n\n");
        assert_eq!(
            favs,
            vec![
                VpnFav {
                    relay: "de-fra-wg-002".into(),
                    ping: "45 ms".into()
                },
                VpnFav {
                    relay: "fr-par-wg-001".into(),
                    ping: "N/A".into()
                },
            ]
        );
    }

    #[test]
    fn reads_toggle_values() {
        let t = "lockdown=on\n autoconnect=off\nobf=udp2tcp\n";
        assert_eq!(toggle_value(t, "lockdown"), "on");
        assert_eq!(toggle_value(t, "autoconnect"), "off");
        assert_eq!(toggle_value(t, "obf"), "udp2tcp");
        assert_eq!(toggle_value(t, "quantum"), "");
    }
}
