//! Parse `mullvad status` / `mullvad status -v` into a structured [`Status`].

use super::sys;

#[derive(Default, Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    pub connected: bool,
    pub connecting: bool,
    /// First status line verbatim (e.g. "Connected", "Disconnected").
    pub state: String,
    pub relay: String,
    /// Raw "Country, City" visible-location string.
    pub location: String,
    pub ipv4: String,
    pub country: String,
    pub city: String,
    /// "WireGuard" / "OpenVPN", inferred from the tunnel interface / relay id.
    pub tunnel_type: String,
}

/// Query the daemon and parse. Plain `status` already carries the Relay +
/// Visible-location lines and gives a clean relay id (no address suffix).
pub fn query() -> Status {
    parse(&sys::mullvad(&["status"]))
}

/// Pure parser (testable without the daemon).
///
/// Real `mullvad status` layout:
/// ```text
/// Connected
///     Relay:            de-fra-wg-002
///     Tunnel interface: wg0-mullvad
///     Visible location: Germany, Frankfurt. IPv4: 1.2.3.4
/// ```
/// Note the visible location is **Country, City** (country first), and there
/// is no "Tunnel:" line — the protocol is inferred from the interface / relay
/// id (`wg` → WireGuard, `ovpn` → OpenVPN).
pub fn parse(s: &str) -> Status {
    let mut st = Status {
        state: "Disconnected".into(),
        ..Default::default()
    };
    for (i, line) in s.lines().enumerate() {
        let t = line.trim();
        if i == 0 {
            st.state = t.to_string();
            let lc = t.to_lowercase();
            st.connected = lc.starts_with("connected");
            st.connecting = lc.starts_with("connecting");
        }
        if let Some(r) = t.strip_prefix("Relay:") {
            // Strip any trailing " (address:port/proto)" → bare relay id.
            st.relay = r.split_whitespace().next().unwrap_or("").to_string();
        }
        if let Some(iface) = t.strip_prefix("Tunnel interface:") {
            let iface = iface.trim().to_lowercase();
            if iface.starts_with("wg") {
                st.tunnel_type = "WireGuard".into();
            } else if !iface.is_empty() {
                st.tunnel_type = "OpenVPN".into();
            }
        }
        if let Some(l) = t.strip_prefix("Visible location:") {
            st.location = l.trim().to_string();
            // "Country, City. IPv4: 1.2.3.4"
            if let Some((before_ip, after_ip)) = l.split_once("IPv4:") {
                st.ipv4 = after_ip.trim().to_string();
                let trimmed = before_ip.trim().trim_end_matches('.').trim().to_string();
                if let Some((country, city)) = trimmed.split_once(',') {
                    st.country = country.trim().to_string();
                    st.city = city.trim().to_string();
                } else {
                    st.country = trimmed;
                }
            }
        }
    }
    // Fall back to the relay id for the protocol when no interface line.
    if st.tunnel_type.is_empty() && !st.relay.is_empty() {
        if st.relay.contains("-wg-") {
            st.tunnel_type = "WireGuard".into();
        } else if st.relay.contains("-ovpn-") {
            st.tunnel_type = "OpenVPN".into();
        }
    }
    st
}

/// Turns `mullvad status listen` output into one [`Status`] per change.
///
/// The daemon prints the current state as a block (an unindented state line
/// plus indented detail lines), then a new block on every change. A block
/// is complete when the next one starts; one that stays the last for a
/// moment is reported through [`Follower::on_quiet`], so a change shows up
/// without waiting for the next one. Repeats are dropped.
#[derive(Default)]
pub struct Follower {
    block: String,
    last: Option<Status>,
}

impl Follower {
    /// One output line; the status to report, if a block just completed and
    /// differs from the last one reported.
    pub fn on_line(&mut self, line: &str) -> Option<Status> {
        if line.trim().is_empty() {
            return None;
        }
        let mut done = None;
        if !line.starts_with(char::is_whitespace) && !self.block.is_empty() {
            done = self.report(parse(&self.block));
            self.block.clear();
        }
        self.block.push_str(line);
        self.block.push('\n');
        done
    }

    /// The stream went quiet: report the block being read.
    pub fn on_quiet(&mut self) -> Option<Status> {
        if self.block.is_empty() {
            return None;
        }
        self.report(parse(&self.block))
    }

    fn report(&mut self, st: Status) -> Option<Status> {
        if self.last.as_ref() == Some(&st) {
            return None;
        }
        self.last = Some(st.clone());
        Some(st)
    }
}

/// How long `mullvad status listen` must be quiet before the block being
/// read counts as complete.
const FOLLOW_QUIET: std::time::Duration = std::time::Duration::from_millis(150);

/// Stream status changes from `mullvad status listen` to `emit` until the
/// daemon stream ends (returns `false`), `emit` fails (the reader is gone),
/// or stdin closes — the caller's cue to stop, which also stops the
/// `mullvad` child so it doesn't outlive us.
pub fn follow(mut emit: impl FnMut(&Status) -> std::io::Result<()>) -> bool {
    use std::io::{BufRead as _, Read as _};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex, mpsc};

    let mut child = match Command::new("mullvad")
        .args(["status", "listen"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            eprintln!("mvpn: cannot run `mullvad status listen`: {err}");
            return false;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    let child = Arc::new(Mutex::new(child));
    let stop = |child: &Mutex<std::process::Child>| {
        if let Ok(mut c) = child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    };

    {
        let child = child.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
            stop(&child);
            std::process::exit(0);
        });
    }

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut follower = Follower::default();
    loop {
        let st = match rx.recv_timeout(FOLLOW_QUIET) {
            Ok(line) => follower.on_line(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => follower.on_quiet(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(st) = st
            && emit(&st).is_err()
        {
            break;
        }
    }
    stop(&child);
    false
}

/// A boolean `mullvad <sub> get` setting whose output ends in "on" / "off".
pub fn setting_on(sub: &str) -> bool {
    sys::mullvad(&[sub, "get"])
        .to_lowercase()
        .trim()
        .ends_with("on")
}

/// Account expiry date (YYYY-MM-DD), or "—".
pub fn account_expiry() -> String {
    parse_expiry(&sys::mullvad(&["account", "get"]))
}

pub fn parse_expiry(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if let Some(e) = t.strip_prefix("Expires at") {
            return e.trim_start_matches(':').trim().chars().take(10).collect();
        }
        if let Some(e) = t.strip_prefix("Expires:") {
            return e.trim().chars().take(10).collect();
        }
    }
    "—".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connected_real_format() {
        // Real `mullvad status`: Country, City; relay may carry an address;
        // protocol from the interface line.
        let s = "Connected\n\
                 Relay: de-fra-wg-002 (185.213.155.74:34609/UDP)\n\
                 Tunnel interface: wg0-mullvad\n\
                 Visible location: Germany, Frankfurt. IPv4: 1.2.3.4";
        let st = parse(s);
        assert!(st.connected);
        assert_eq!(st.relay, "de-fra-wg-002");
        assert_eq!(st.country, "Germany");
        assert_eq!(st.city, "Frankfurt");
        assert_eq!(st.ipv4, "1.2.3.4");
        assert_eq!(st.tunnel_type, "WireGuard");
    }

    #[test]
    fn protocol_falls_back_to_relay_id() {
        let st = parse("Connected\nRelay: us-nyc-ovpn-101\n");
        assert_eq!(st.tunnel_type, "OpenVPN");
    }

    #[test]
    fn parses_disconnected() {
        let st = parse("Disconnected");
        assert!(!st.connected);
        assert!(!st.connecting);
        assert_eq!(st.state, "Disconnected");
    }

    #[test]
    fn parses_connecting() {
        let st = parse("Connecting to de-ber-wg-006...");
        assert!(st.connecting);
        assert!(!st.connected);
    }

    const CONNECTED: &str = "Connected\n    Relay:                  de-fra-wg-002\n    Features:               LAN Sharing, LWO\n    Visible location:       Germany, Frankfurt. IPv4: 1.2.3.4";

    fn feed(f: &mut Follower, block: &str) -> Vec<Status> {
        block.lines().filter_map(|l| f.on_line(l)).collect()
    }

    #[test]
    fn follower_reports_each_listen_block_once() {
        let mut f = Follower::default();
        // The initial state: nothing reported until the stream pauses.
        assert!(feed(&mut f, CONNECTED).is_empty());
        let st = f.on_quiet().expect("initial state after the pause");
        assert!(st.connected);
        assert_eq!(st.relay, "de-fra-wg-002");
        assert_eq!(st.city, "Frankfurt");
        // Quiet again with nothing new: no repeat.
        assert_eq!(f.on_quiet(), None);

        // The next block completes the previous one (same, not repeated) and
        // is itself reported on the pause.
        assert!(feed(&mut f, "Disconnecting...").is_empty());
        let st = f.on_quiet().expect("disconnecting");
        assert!(!st.connected);
        assert_eq!(st.state, "Disconnecting...");
        // Back-to-back blocks without a pause: the first is reported when the
        // second starts.
        let got = feed(&mut f, "Disconnected\nConnecting\n    Relay: se-sto-wg-001");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, "Disconnected");
        let st = f.on_quiet().expect("connecting");
        assert!(st.connecting);
        assert_eq!(st.relay, "se-sto-wg-001");
    }

    #[test]
    fn follower_completes_a_block_reported_early() {
        let mut f = Follower::default();
        // The pause hit between the state line and its details.
        assert!(f.on_line("Connected").is_none());
        let early = f.on_quiet().expect("partial block");
        assert!(early.relay.is_empty());
        assert!(f.on_line("    Relay: de-fra-wg-002").is_none());
        let full = f.on_quiet().expect("the details follow");
        assert_eq!(full.relay, "de-fra-wg-002");
        assert!(f.on_line("").is_none());
    }

    #[test]
    fn expiry_both_forms() {
        assert_eq!(
            parse_expiry("Expires at: 2027-01-02 12:00:00 UTC"),
            "2027-01-02"
        );
        assert_eq!(parse_expiry("Expires: 2027-01-02"), "2027-01-02");
        assert_eq!(parse_expiry("nope"), "—");
    }
}
