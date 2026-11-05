//! `mullvad status --json`: the tunnel state with the details the text
//! status leaves out — the error cause, whether lockdown is blocking, the
//! visible location's coordinates and whether it is a Mullvad exit.
//!
//! Layout (mullvad-cli 2026.5, `talpid-types` `TunnelState` serialised):
//! ```json
//! {"state":"connected","details":{"location":{"country":"Germany",
//!   "city":"Frankfurt","latitude":50.1,"longitude":8.6,
//!   "mullvad_exit_ip":true,"hostname":"de-fra-wg-002",...},...}}
//! {"state":"disconnected","details":{"location":{...},"locked_down":true}}
//! {"state":"error","details":{"cause":{"reason":"auth_failed",
//!   "details":"INVALID_ACCOUNT"},"block_failure":null}}
//! ```

use serde_json::Value;

use super::sys;

/// The visible location the daemon reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub country: String,
    pub city: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// The address is a Mullvad exit (so not where the user really is).
    pub mullvad_exit: bool,
}

/// One `mullvad status --json` answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tunnel {
    /// `disconnected` / `connecting` / `connected` / `disconnecting` / `error`.
    pub state: String,
    /// Disconnected with lockdown on: all traffic is blocked.
    pub locked_down: bool,
    /// `error` state: the cause (`set_firewall_policy_error`, `auth_failed`, …).
    pub error_reason: String,
    /// `error` state: the cause's detail (`INVALID_ACCOUNT`, `generic`, …).
    pub error_detail: String,
    pub place: Option<Place>,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Parse `mullvad status --json`; `None` for anything that is not a
/// tunnel state (empty output from a stopped daemon, an error message).
pub fn parse(json: &str) -> Option<Tunnel> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let state = text(&v, "state");
    if !matches!(
        state.as_str(),
        "disconnected" | "connecting" | "connected" | "disconnecting" | "error"
    ) {
        return None;
    }
    let details = v.get("details").cloned().unwrap_or(Value::Null);
    let place = details
        .get("location")
        .filter(|l| l.is_object())
        .map(|l| Place {
            country: text(l, "country"),
            city: text(l, "city"),
            latitude: l.get("latitude").and_then(Value::as_f64),
            longitude: l.get("longitude").and_then(Value::as_f64),
            mullvad_exit: l.get("mullvad_exit_ip").and_then(Value::as_bool) == Some(true),
        });
    let cause = details.get("cause").cloned().unwrap_or(Value::Null);
    // The detail is a string (`"INVALID_ACCOUNT"`) or `{"reason": "…"}`.
    let error_detail = match cause.get("details") {
        Some(Value::String(s)) => s.clone(),
        Some(d) => text(d, "reason"),
        None => String::new(),
    };
    Some(Tunnel {
        locked_down: details.get("locked_down").and_then(Value::as_bool) == Some(true),
        error_reason: text(&cause, "reason"),
        error_detail,
        place,
        state,
    })
}

/// The daemon's tunnel state, or `None` (daemon down, no `mullvad`).
pub fn query() -> Option<Tunnel> {
    parse(&sys::mullvad(&["status", "--json"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured here (mullvad-cli 2026.5), addresses and coordinates redacted.
    const CONNECTED: &str = include_str!("../../tests/fixtures/status-connected.json");

    #[test]
    fn parses_a_connected_tunnel() {
        let t = parse(CONNECTED).expect("tunnel");
        assert_eq!(t.state, "connected");
        let p = t.place.expect("place");
        assert_eq!(
            (p.country.as_str(), p.city.as_str()),
            ("Germany", "Frankfurt")
        );
        assert!(p.mullvad_exit);
        assert_eq!(p.latitude, Some(50.11092));
        assert!(!t.locked_down);
        assert!(t.error_reason.is_empty());
    }

    #[test]
    fn parses_blocked_and_error_states() {
        let t = parse(
            r#"{"state":"disconnected","details":{"location":{"latitude":41.0,"longitude":29.0,"mullvad_exit_ip":false,"city":"Istanbul","country":"Turkey"},"locked_down":true}}"#,
        )
        .expect("tunnel");
        assert!(t.locked_down);
        let p = t.place.expect("place");
        assert!(!p.mullvad_exit);
        assert_eq!(p.longitude, Some(29.0));
        let t = parse(
            r#"{"state":"error","details":{"cause":{"reason":"set_firewall_policy_error","details":{"reason":"generic"}},"block_failure":{"reason":"generic"}}}"#,
        )
        .expect("tunnel");
        assert_eq!(
            (t.error_reason.as_str(), t.error_detail.as_str()),
            ("set_firewall_policy_error", "generic")
        );
        assert!(t.place.is_none());
        let t = parse(
            r#"{"state":"error","details":{"cause":{"reason":"auth_failed","details":"EXPIRED_ACCOUNT"}}}"#,
        )
        .expect("tunnel");
        assert_eq!(t.error_detail, "EXPIRED_ACCOUNT");
    }

    #[test]
    fn rejects_what_is_not_a_tunnel_state() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("Error: Management RPC server or client error"), None);
        assert_eq!(parse(r#"{"state":"weird"}"#), None);
    }
}
