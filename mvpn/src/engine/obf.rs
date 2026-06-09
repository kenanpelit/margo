//! Anti-censorship / obfuscation control.
//!
//! Newer `mullvad` renamed `obfuscation` → `anti-censorship`. Modes:
//! `auto, off, wireguard-port, udp2tcp, shadowsocks, quic, lwo`. We expose the
//! common ones plus `cycle` and a `hunt443` convenience.

use super::{actions, sys};

/// Modes we cycle through (a useful subset of the daemon's enum).
pub const CYCLE: &[&str] = &["auto", "udp2tcp", "shadowsocks", "quic", "off"];

/// Current mode from `anti-censorship get` (the `mode:` line), lowercased.
pub fn current() -> String {
    parse_mode(&sys::mullvad(&["anti-censorship", "get"]))
}

pub fn parse_mode(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if let Some(m) = t.strip_prefix("mode:") {
            return m.trim().to_lowercase();
        }
        // Older `obfuscation get` form, just in case.
        if let Some(m) = t.strip_prefix("Obfuscation mode:") {
            return m.trim().to_lowercase();
        }
    }
    String::new()
}

/// Set a mode then reconnect so it takes effect.
pub fn set(mode: &str) -> bool {
    sys::mullvad_ok(&["anti-censorship", "set", "mode", mode]) && {
        let _ = actions::disconnect();
        actions::connect()
    }
}

/// Advance to the next mode in [`CYCLE`] from the current one.
pub fn cycle() -> Option<String> {
    let cur = current();
    let idx = CYCLE.iter().position(|m| *m == cur).unwrap_or(usize::MAX);
    let next = CYCLE[idx.wrapping_add(1) % CYCLE.len()];
    if set(next) {
        Some(next.to_string())
    } else {
        None
    }
}

/// Convenience: force udp2tcp (tunnels WireGuard over TCP, which can land on
/// 443) and reconnect — the practical "get me out of a restricted network" path.
pub fn hunt443() -> bool {
    set("udp2tcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_form() {
        let s = "mode: auto\nudp2tcp settings: any port\n";
        assert_eq!(parse_mode(s), "auto");
    }

    #[test]
    fn parses_old_form() {
        assert_eq!(parse_mode("Obfuscation mode: Udp2Tcp"), "udp2tcp");
    }

    #[test]
    fn empty_when_absent() {
        assert_eq!(parse_mode("nothing here"), "");
    }
}
