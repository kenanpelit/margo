//! Parse `mullvad status` / `mullvad status -v` into a structured [`Status`].

use super::sys;

pub use mvpn_proto::Status;

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
/// The value of a `Key: value` detail line. `mullvad status listen` marks
/// the details that changed since its previous block as `Key (new): value`.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.strip_prefix(" (new)").unwrap_or(rest);
    rest.strip_prefix(':')
}

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
        if let Some(r) = field(t, "Relay") {
            // Strip any trailing " (address:port/proto)" → bare relay id.
            st.relay = r.split_whitespace().next().unwrap_or("").to_string();
        }
        if let Some(iface) = field(t, "Tunnel interface") {
            let iface = iface.trim().to_lowercase();
            if iface.starts_with("wg") {
                st.tunnel_type = "WireGuard".into();
            } else if !iface.is_empty() {
                st.tunnel_type = "OpenVPN".into();
            }
        }
        if let Some(l) = field(t, "Visible location") {
            // "Country, City. IPv4: 1.2.3.4" — split by the rule mshell reads.
            st.set_location(l);
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
    /// The last connected status that had a location: the daemon reports a
    /// connect before its visible-location lookup lands and does not report
    /// again when it does, so a reconnect to the same relay borrows it.
    known: Option<Status>,
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

    fn report(&mut self, mut st: Status) -> Option<Status> {
        // A block repeats only the details that changed: a "Connected" that
        // follows "Connecting" carries just `Visible location (new)`, so the
        // relay and the rest come from the block before.
        if let Some(prev) = &self.last {
            carry_over(&mut st, prev);
        }
        if let Some(known) = &self.known {
            st = with_known_location(st, known);
        }
        self.publish(st)
    }

    /// The last reported status still lacks its relay or location (see
    /// [`needs_location`]) — the caller should ask for a full status.
    pub fn wants_full_status(&self) -> bool {
        self.last.as_ref().is_some_and(needs_location)
    }

    /// Fill the last reported status from a full `mullvad status` answer;
    /// the filled status to report, if anything changed.
    pub fn on_full_status(&mut self, full: Status) -> Option<Status> {
        let last = self.last.clone()?;
        self.publish(fill_from(last, full))
    }

    fn publish(&mut self, st: Status) -> Option<Status> {
        if st.connected && !st.location.is_empty() && !st.relay.is_empty() {
            self.known = Some(st.clone());
        }
        if self.last.as_ref() == Some(&st) {
            return None;
        }
        self.last = Some(st.clone());
        Some(st)
    }
}

/// A connected status that misses its relay or location.
pub fn needs_location(st: &Status) -> bool {
    st.connected && (st.location.is_empty() || st.relay.is_empty())
}

/// Take the missing relay / location from `full` when it describes the
/// same tunnel (an empty relay matches any).
fn fill_from(mut st: Status, full: Status) -> Status {
    if full.connected && (st.relay.is_empty() || full.relay == st.relay) {
        if st.relay.is_empty() {
            st.relay = full.relay;
        }
        if st.location.is_empty() {
            st.location = full.location;
            st.country = full.country;
            st.city = full.city;
            st.ipv4 = full.ipv4;
        }
        if st.tunnel_type.is_empty() {
            st.tunnel_type = full.tunnel_type;
        }
    }
    st
}

/// Fill `st`'s missing location from `known` when it is the same relay.
fn with_known_location(mut st: Status, known: &Status) -> Status {
    if needs_location(&st) && !st.relay.is_empty() && st.relay == known.relay {
        st.location = known.location.clone();
        st.country = known.country.clone();
        st.city = known.city.clone();
        st.ipv4 = known.ipv4.clone();
    }
    st
}

/// Fill the details `st` left out from `prev`, while both are the same
/// tunnel (connecting or connected).
fn carry_over(st: &mut Status, prev: &Status) {
    let tunnel = |s: &Status| s.connected || s.connecting;
    if !tunnel(st) || !tunnel(prev) {
        return;
    }
    if st.relay.is_empty() {
        st.relay = prev.relay.clone();
    }
    if st.tunnel_type.is_empty() {
        st.tunnel_type = prev.tunnel_type.clone();
    }
    if st.location.is_empty() {
        st.location = prev.location.clone();
        st.ipv4 = prev.ipv4.clone();
        st.country = prev.country.clone();
        st.city = prev.city.clone();
    }
}

/// How long `mullvad status listen` must be quiet before the block being
/// read counts as complete.
const FOLLOW_QUIET: std::time::Duration = std::time::Duration::from_millis(150);

/// While the reported status still lacks its relay / location, ask for a
/// full `mullvad status` at most this often.
const FULL_STATUS_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

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
    let mut last_full: Option<std::time::Instant> = None;
    loop {
        let st = match rx.recv_timeout(FOLLOW_QUIET) {
            Ok(line) => follower.on_line(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => match follower.on_quiet() {
                Some(st) => Some(st),
                None if follower.wants_full_status()
                    && last_full.is_none_or(|t| t.elapsed() >= FULL_STATUS_RETRY) =>
                {
                    last_full = Some(std::time::Instant::now());
                    follower.on_full_status(query())
                }
                None => None,
            },
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

    /// Captured from `mullvad status listen` across `mvpn reconnect`: the
    /// final "Connected" block lists only what changed.
    #[test]
    fn follower_keeps_the_details_a_changed_block_leaves_out() {
        let mut f = Follower::default();
        let got = feed(
            &mut f,
            "Connected\n    Relay:                  de-fra-wg-002\n    Features:               LAN Sharing\n    Visible location:       Germany, Frankfurt. IPv4: 185.213.155.228\n\
             Connecting\n    Relay:                  de-fra-wg-002\n    Features:               LAN Sharing\n    Visible location:       Germany, Frankfurt\n\
             Connected\n",
        );
        assert_eq!(got.len(), 2);
        assert!(
            f.on_line("    Visible location (new): Germany, Frankfurt. IPv4: 185.213.155.228")
                .is_none()
        );
        let st = f.on_quiet().expect("the reconnected state");
        assert!(st.connected);
        assert_eq!(st.relay, "de-fra-wg-002");
        assert_eq!(st.location, "Germany, Frankfurt. IPv4: 185.213.155.228");
        assert_eq!(st.city, "Frankfurt");
        assert_eq!(st.ipv4, "185.213.155.228");
        assert_eq!(st.tunnel_type, "WireGuard");
        // Disconnecting starts a new story: nothing is carried into it.
        assert!(feed(&mut f, "Disconnected").is_empty());
        let st = f.on_quiet().expect("disconnected");
        assert!(st.relay.is_empty() && st.location.is_empty());
    }

    #[test]
    fn parses_changed_detail_lines() {
        let st = parse(
            "Connected\n    Relay (new): se-sto-wg-001\n    Visible location (new): Sweden, Stockholm. IPv4: 5.6.7.8",
        );
        assert_eq!(st.relay, "se-sto-wg-001");
        assert_eq!(st.city, "Stockholm");
    }

    /// The daemon's change stream reports a connect before the location is
    /// known and never again for the lookup: the last known location for
    /// the same relay fills the gap, a different relay's never does.
    #[test]
    fn a_reconnect_without_location_takes_the_relays_known_location() {
        let mut f = Follower::default();
        feed(&mut f, CONNECTED);
        let first = f.on_quiet().expect("connected");
        assert!(!f.wants_full_status());
        feed(&mut f, "Disconnected\n");
        f.on_quiet();
        feed(&mut f, "Connected\n    Relay: de-fra-wg-002\n");
        let again = f.on_quiet().expect("reconnected");
        assert_eq!(again.location, first.location);
        assert_eq!(again.city, "Frankfurt");
        // Another relay never borrows it: the caller is asked to query.
        feed(&mut f, "Disconnected\n");
        f.on_quiet();
        feed(&mut f, "Connected\n    Relay: se-sto-wg-001\n");
        let other = f.on_quiet().expect("other relay");
        assert!(other.location.is_empty());
        assert!(f.wants_full_status());
    }

    /// A reconnect's final block can carry neither relay nor location; the
    /// full status fills both, once, and only for the same tunnel.
    #[test]
    fn a_status_without_relay_is_filled_from_the_full_status() {
        let mut f = Follower::default();
        feed(&mut f, "Connected\n");
        assert!(f.on_quiet().is_some());
        assert!(f.wants_full_status());
        let full = parse(CONNECTED);
        let filled = f.on_full_status(full.clone()).expect("filled");
        assert_eq!(filled.relay, "de-fra-wg-002");
        assert_eq!(filled.city, "Frankfurt");
        assert!(!f.wants_full_status());
        assert!(f.on_full_status(full.clone()).is_none(), "no repeat");
        // A full status for a different relay is never borrowed.
        let mut g = Follower::default();
        feed(&mut g, "Connected\n    Relay: se-sto-wg-001\n");
        g.on_quiet();
        assert!(g.on_full_status(full).is_none());
        // Disconnected never asks.
        let mut h = Follower::default();
        feed(&mut h, "Disconnected\n");
        h.on_quiet();
        assert!(!h.wants_full_status());
    }

    /// The bug: country/city were only split out when an `IPv4:` part
    /// followed, so a "Connecting" block (`Visible location: Germany,
    /// Frankfurt`) and an IPv6-only exit had no city — and mshell re-parsed
    /// the raw location itself to make up for it.
    #[test]
    fn city_and_country_without_an_ipv4_part() {
        let st =
            parse("Connecting\n    Relay: de-fra-wg-002\n    Visible location: Germany, Frankfurt");
        assert_eq!(
            (st.country.as_str(), st.city.as_str()),
            ("Germany", "Frankfurt")
        );
        assert!(st.ipv4.is_empty());
        let st = parse("Connected\n    Visible location: USA, New York, NY. IPv6: 2001:db8::1");
        assert_eq!(
            (st.country.as_str(), st.city.as_str()),
            ("USA", "New York, NY")
        );
        let st = parse(
            "Connected\n    Visible location: Germany, Frankfurt. IPv4: 1.2.3.4, IPv6: 2001:db8::1",
        );
        assert_eq!(st.ipv4, "1.2.3.4");
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
