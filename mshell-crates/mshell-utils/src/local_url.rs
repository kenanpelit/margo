//! Browser links for local listeners: the one rule for turning a bound
//! address + port into the URL a user can click, shared by the CPU
//! dashboard's Ports tab, the Podman menu's port chips / host services and
//! the SSH port forwards.
//!
//! A socket bound to every interface (`0.0.0.0`, `::`, `*`, empty) or to
//! loopback is opened as `localhost`; a socket bound to one specific
//! address keeps that address (`192.168.1.50:3000` → `http://192.168.1.50:3000`,
//! IPv6 bracketed). Link-local and multicast binds get no link — a browser
//! cannot reach them without an interface scope, and nothing serves HTTP on
//! a multicast group. HTTPS is assumed only on 443; anything else opens as
//! `http://` and the browser can retry.

use std::net::IpAddr;

/// The host part of a link to a socket bound on `bind`, or `None` when
/// a browser cannot reach that address.
pub fn browser_host(bind: &str) -> Option<String> {
    let bind = bind.trim().trim_start_matches('[').trim_end_matches(']');
    // `%iface` scope (`fe80::1%wlan0`, `127.0.0.53%lo`).
    let (bind, scope) = match bind.split_once('%') {
        Some((addr, scope)) => (addr, Some(scope)),
        None => (bind, None),
    };
    if bind.is_empty() || bind == "*" || bind.eq_ignore_ascii_case("localhost") {
        return Some("localhost".to_string());
    }
    let ip: IpAddr = bind.parse().ok()?;
    if ip.is_unspecified() || ip.is_loopback() || scope == Some("lo") {
        return Some("localhost".to_string());
    }
    if ip.is_multicast() {
        return None;
    }
    match ip {
        IpAddr::V4(v4) if v4.is_link_local() || v4.is_broadcast() => None,
        IpAddr::V4(v4) => Some(v4.to_string()),
        // fe80::/10 — unreachable without the interface scope.
        IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80 => None,
        IpAddr::V6(v6) => Some(format!("[{v6}]")),
    }
}

/// `http(s)://<host>:<port>` for a TCP listener bound on `bind`, or `None`
/// when it is not browsable (port 0, link-local / multicast bind).
pub fn url(bind: &str, port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }
    let host = browser_host(bind)?;
    let scheme = if port == 443 { "https" } else { "http" };
    Some(format!("{scheme}://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_and_loopback_open_as_localhost() {
        for bind in [
            "",
            "*",
            "0.0.0.0",
            "::",
            "[::]",
            "127.0.0.1",
            "127.0.0.53",
            "::1",
            "[::1]",
            "localhost",
            "127.0.0.53%lo",
        ] {
            assert_eq!(
                url(bind, 8080).as_deref(),
                Some("http://localhost:8080"),
                "{bind:?}"
            );
        }
    }

    #[test]
    fn a_specific_address_keeps_itself() {
        assert_eq!(
            url("192.168.1.50", 3000).as_deref(),
            Some("http://192.168.1.50:3000")
        );
        assert_eq!(
            url("2001:db8::7", 9090).as_deref(),
            Some("http://[2001:db8::7]:9090")
        );
        assert_eq!(
            url("[2001:db8::7]", 9090).as_deref(),
            Some("http://[2001:db8::7]:9090")
        );
    }

    #[test]
    fn https_only_on_443() {
        assert_eq!(
            url("0.0.0.0", 443).as_deref(),
            Some("https://localhost:443")
        );
        assert_eq!(
            url("0.0.0.0", 8443).as_deref(),
            Some("http://localhost:8443")
        );
    }

    #[test]
    fn unreachable_binds_get_no_link() {
        assert_eq!(url("fe80::1%wlan0", 8080), None);
        assert_eq!(url("169.254.1.2", 80), None);
        assert_eq!(url("224.0.0.251", 5353), None);
        assert_eq!(url("ff02::fb", 5353), None);
        assert_eq!(url("255.255.255.255", 80), None);
        assert_eq!(url("0.0.0.0", 0), None);
        assert_eq!(url("not-an-ip", 80), None);
    }
}
