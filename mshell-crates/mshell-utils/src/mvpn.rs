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

async fn probe_vpn() -> VpnProbe {
    let cached = STREAM_STATUS
        .lock()
        .ok()
        .and_then(|mut s| streamed_status(VPN_POLL.feed_alive(), &mut s));
    let status = match cached {
        Some(status) => status,
        None => match tokio::process::Command::new("mvpn")
            .args(["status", "--json"])
            .output()
            .await
        {
            Ok(out) => VpnStatus::parse(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => VpnStatus::default(),
        },
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
