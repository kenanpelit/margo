//! Parsing for the `mvpn` CLI's output, shared by the VPN bar pill, the VPN
//! menu, Settings → VPN and the VPN toast. (Each used to carry its own
//! copy of a string-search JSON "parser" that broke on `"key": "v"` spacing
//! or an escaped quote.)

use crate::shared_poll::SharedPoll;
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

/// `mvpn` has no push API, so this polls every 5 s. One loop for the whole
/// shell, shared by every VPN pill and the VPN toast, running only while one
/// of them is active (a pill mapped, or the toast switched on). Each pill
/// used to run its own `mvpn status` + `systemctl is-active` per bar, and the
/// toast producer a third `mvpn status`.
pub static VPN_POLL: LazyLock<SharedPoll<VpnProbe>> =
    LazyLock::new(|| SharedPoll::new(|| Duration::from_secs(5), || Box::pin(probe_vpn())));

async fn probe_vpn() -> VpnProbe {
    let status = match tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await
    {
        Ok(out) => VpnStatus::parse(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => VpnStatus::default(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

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
