//! Connection diagnostics: a Mullvad-side leak check + split-tunnel listing.

use super::{status, sys};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LeakReport {
    pub connected: bool,
    pub mullvad_exit: bool,
    pub exit_ip: String,
    pub relay: String,
}

/// Ask `am.i.mullvad.net` whether we're exiting through Mullvad. Uses the JSON
/// endpoint; `curl` is required (returns a not-connected report if absent).
pub fn leak_test() -> LeakReport {
    let st = status::query();
    let body = sys::out(
        "curl",
        &["-s", "--max-time", "8", "https://am.i.mullvad.net/json"],
    );
    let mullvad_exit = parse_is_mullvad(&body);
    let exit_ip = parse_ip(&body);
    LeakReport {
        connected: st.connected,
        mullvad_exit,
        exit_ip,
        relay: st.relay,
    }
}

fn parse_is_mullvad(json: &str) -> bool {
    // Cheap field scan — avoids a serde model for one bool.
    json.contains("\"mullvad_exit_ip\":true") || json.contains("\"mullvad_exit_ip\": true")
}

fn parse_ip(json: &str) -> String {
    // "ip":"1.2.3.4"
    if let Some(i) = json.find("\"ip\"") {
        let rest = &json[i + 4..];
        if let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            if let Some(end) = after.find('"') {
                return after[..end].to_string();
            }
        }
    }
    String::new()
}

/// Processes excluded from the tunnel (`mullvad split-tunnel list`).
pub fn split_tunnel() -> String {
    sys::mullvad(&["split-tunnel", "list"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mullvad_exit() {
        assert!(parse_is_mullvad(
            r#"{"ip":"1.2.3.4","mullvad_exit_ip":true}"#
        ));
        assert!(!parse_is_mullvad(r#"{"mullvad_exit_ip":false}"#));
    }

    #[test]
    fn extracts_ip() {
        assert_eq!(parse_ip(r#"{"ip":"185.1.2.3","city":"X"}"#), "185.1.2.3");
        assert_eq!(parse_ip("{}"), "");
    }
}
