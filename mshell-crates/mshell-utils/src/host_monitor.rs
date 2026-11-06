//! Host reachability for the Host Monitor pill and menu
//! (`network.host_monitor`): each configured host is pinged on an
//! interval, its state is up (with the round-trip time) or down (with why),
//! and an up → down / down → up change becomes a notification.
//!
//! A check is one `ping -n -q -c 3 -i 0.2 -W <timeout> -- <host>` (iputils):
//! three probes 200 ms apart, so one lost packet on a busy link is not an
//! outage. Up = any reply (exit 0), with the average RTT from the summary;
//! exit 1 = no reply; exit 2 = an error worded from stderr (unknown host,
//! no route). Output is read with `LC_ALL=C`. Hosts that drop ICMP read
//! as down — the same limit as any ping monitor.
//!
//! Pure parsers and the transition logic are unit-tested against `ping`
//! output captured on the owner's machine.

use mshell_config::schema::config::{HostMonitorConfig, MonitoredHost};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

/// Bounds for the configured interval / timeout.
pub const INTERVAL_RANGE: (u32, u32) = (5, 3600);
pub const TIMEOUT_RANGE: (u32, u32) = (1, 10);

/// What the last check found.
#[derive(Debug, Clone, PartialEq)]
pub enum HostState {
    /// Not checked yet.
    Checking,
    /// Answered; the average round-trip time when ping printed one.
    Up { latency_ms: Option<f64> },
    /// No answer, or ping could not even try; the reason for the UI.
    Down { reason: String },
}

impl HostState {
    pub fn is_down(&self) -> bool {
        matches!(self, HostState::Down { .. })
    }

    /// `Some(up?)` once checked.
    fn up(&self) -> Option<bool> {
        match self {
            HostState::Checking => None,
            HostState::Up { .. } => Some(true),
            HostState::Down { .. } => Some(false),
        }
    }
}

/// One row: the configured host and its state.
#[derive(Debug, Clone, PartialEq)]
pub struct HostStatus {
    pub host: String,
    pub label: String,
    pub state: HostState,
}

impl HostStatus {
    /// The label, or the host when none was set.
    pub fn display(&self) -> &str {
        display_name(&self.host, &self.label)
    }
}

pub fn display_name<'a>(host: &'a str, label: &'a str) -> &'a str {
    if label.trim().is_empty() {
        host
    } else {
        label.trim()
    }
}

/// The colour tier of a round-trip time (the plugin's 100 / 300 ms steps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyTier {
    Good,
    Slow,
    Bad,
}

pub fn latency_tier(ms: f64) -> LatencyTier {
    if ms < 100.0 {
        LatencyTier::Good
    } else if ms < 300.0 {
        LatencyTier::Slow
    } else {
        LatencyTier::Bad
    }
}

/// `12 ms`, `0.4 ms`, `1.2 s`.
pub fn format_latency(ms: f64) -> String {
    if ms < 1.0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{} ms", ms.round() as u32)
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The ping target in a typed or pasted entry: an IP, a host name, or a
/// URL / `host:port` / `user@host` reduced to its host. `None` when what
/// is left is not a valid IP or host name (so nothing `ping` could read as
/// an option ever reaches it).
pub fn normalize_target(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if let Some((scheme, rest)) = s.split_once("://")
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    {
        s = rest;
    }
    // Path, query, fragment.
    s = s.split(['/', '?', '#']).next().unwrap_or("");
    // Userinfo.
    if let Some((_, host)) = s.rsplit_once('@') {
        s = host;
    }
    // `[v6]` / `[v6]:port`.
    if let Some(rest) = s.strip_prefix('[') {
        let inner = rest.split_once(']')?.0;
        return inner.parse::<Ipv6Addr>().ok().map(|a| a.to_string());
    }
    // A bare IPv6 address has several colons; `host:port` has one.
    if s.matches(':').count() > 1 {
        return s.parse::<Ipv6Addr>().ok().map(|a| a.to_string());
    }
    let host = s.split(':').next().unwrap_or("").trim_end_matches('.');
    if host.parse::<IpAddr>().is_ok() {
        return Some(host.to_string());
    }
    hostname_ok(host).then(|| host.to_ascii_lowercase())
}

/// RFC 1123 labels: letters, digits, inner hyphens; 1–63 each, 253 total;
/// not all-numeric (that would be a malformed IPv4 address).
fn hostname_ok(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let labels_ok = host.split('.').all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    labels_ok && !host.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Why a host can't be added (worded for the UI), else its normal form.
pub fn validate_new(raw: &str, existing: &[MonitoredHost]) -> Result<String, String> {
    let host = normalize_target(raw)
        .ok_or_else(|| "Enter an IP address, a host name or a link".to_string())?;
    if existing.iter().any(|h| h.host.eq_ignore_ascii_case(&host)) {
        return Err(format!("{host} is already monitored"));
    }
    Ok(host)
}

pub fn interval(cfg: &HostMonitorConfig) -> Duration {
    Duration::from_secs(u64::from(
        cfg.interval_seconds
            .clamp(INTERVAL_RANGE.0, INTERVAL_RANGE.1),
    ))
}

fn timeout_secs(cfg: &HostMonitorConfig) -> u32 {
    cfg.timeout_seconds.clamp(TIMEOUT_RANGE.0, TIMEOUT_RANGE.1)
}

/// The `ping` argv for one check (the host is already normalised).
pub fn ping_args(host: &str, timeout_secs: u32) -> Vec<String> {
    [
        "-n",
        "-q",
        "-c",
        "3",
        "-i",
        "0.2",
        "-W",
        &timeout_secs.to_string(),
        "--",
        host,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Read one finished `ping`: its exit code, stdout and stderr.
pub fn parse_ping(code: Option<i32>, stdout: &str, stderr: &str) -> HostState {
    match code {
        Some(0) => HostState::Up {
            latency_ms: avg_rtt(stdout),
        },
        Some(1) => HostState::Down {
            reason: "No reply".to_string(),
        },
        _ => HostState::Down {
            reason: error_reason(stderr),
        },
    }
}

/// `rtt min/avg/max/mdev = 0.084/0.095/0.109/0.010 ms` → 0.095.
fn avg_rtt(stdout: &str) -> Option<f64> {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("rtt ") || l.starts_with("round-trip "))?;
    let values = line.split_once('=')?.1.trim();
    values.split('/').nth(1)?.trim().parse().ok()
}

fn error_reason(stderr: &str) -> String {
    let s = stderr.to_ascii_lowercase();
    if s.contains("name or service not known")
        || s.contains("temporary failure in name resolution")
        || s.contains("no address associated")
    {
        "Unknown host".to_string()
    } else if s.contains("network is unreachable") {
        "Network unreachable".to_string()
    } else if s.contains("no route to host") {
        "No route to host".to_string()
    } else {
        match stderr.lines().next().map(str::trim) {
            Some(line) if !line.is_empty() => line.trim_start_matches("ping: ").to_string(),
            _ => "ping failed".to_string(),
        }
    }
}

/// A change worth a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// Stopped answering (also on the first check of a host that is down).
    Down { name: String, reason: String },
    /// Answers again after being down.
    Recovered { name: String },
}

/// Compare `results` with the last known up/down per host (`last`, updated
/// in place). Hosts no longer listed are forgotten, so one re-added later
/// starts fresh; a host still `Checking` changes nothing.
pub fn transitions(last: &mut HashMap<String, bool>, results: &[HostStatus]) -> Vec<Transition> {
    last.retain(|host, _| results.iter().any(|r| &r.host == host));
    let mut out = Vec::new();
    for r in results {
        let Some(up) = r.state.up() else { continue };
        let before = last.insert(r.host.clone(), up);
        match (before, up, &r.state) {
            (Some(false), true, _) => out.push(Transition::Recovered {
                name: r.display().to_string(),
            }),
            (None | Some(true), false, HostState::Down { reason }) => out.push(Transition::Down {
                name: r.display().to_string(),
                reason: reason.clone(),
            }),
            _ => {}
        }
    }
    out
}

/// The rows to show: every configured host in its configured order, with
/// its last checked state (`Checking` for one added since the last check).
pub fn rows(hosts: &[MonitoredHost], checked: &[HostStatus]) -> Vec<HostStatus> {
    hosts
        .iter()
        .map(|h| HostStatus {
            host: h.host.clone(),
            label: h.label.clone(),
            state: checked
                .iter()
                .find(|c| c.host == h.host)
                .map_or(HostState::Checking, |c| c.state.clone()),
        })
        .collect()
}

pub fn down_count(results: &[HostStatus]) -> usize {
    results.iter().filter(|r| r.state.is_down()).count()
}

// ── Running ping ───────────────────────────────────────────────────────────

/// Why checks can't run here, if they can't.
pub fn unavailable() -> Option<&'static str> {
    (!margo_fs::path::on_path("ping")).then_some("ping is not installed (package iputils)")
}

async fn check(host: String, timeout_secs: u32) -> HostState {
    // 3 probes 0.2 s apart + the reply wait, plus slack for DNS.
    let limit = Duration::from_secs(u64::from(timeout_secs) + 5);
    let run = tokio::process::Command::new("ping")
        .args(ping_args(&host, timeout_secs))
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(limit, run).await {
        Err(_) => HostState::Down {
            reason: "Timed out".to_string(),
        },
        Ok(Err(e)) => HostState::Down {
            reason: format!("ping: {e}"),
        },
        Ok(Ok(out)) => parse_ping(
            out.status.code(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
    }
}

/// Check every configured host at once.
pub async fn check_all(cfg: &HostMonitorConfig) -> Vec<HostStatus> {
    let timeout = timeout_secs(cfg);
    let states =
        futures::future::join_all(cfg.hosts.iter().map(|h| check(h.host.clone(), timeout))).await;
    cfg.hosts
        .iter()
        .zip(states)
        .map(|(h, state)| HostStatus {
            host: h.host.clone(),
            label: h.label.clone(),
            state,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // `LC_ALL=C ping -n -q -c 3 -i 0.2 -W 1 -- <host>` on the owner's
    // machine (iputils 20250605).
    const UP_V4: &str = "PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.\n\n--- 1.1.1.1 ping statistics ---\n3 packets transmitted, 3 received, 0% packet loss, time 402ms\nrtt min/avg/max/mdev = 46.066/47.937/51.403/2.452 ms\n";
    const UP_V6: &str = "PING ::1 (::1) 56 data bytes\n\n--- ::1 ping statistics ---\n3 packets transmitted, 3 received, 0% packet loss, time 406ms\nrtt min/avg/max/mdev = 0.261/0.284/0.305/0.018 ms\n";
    const NO_REPLY: &str = "PING 192.0.2.1 (192.0.2.1) 56(84) bytes of data.\n\n--- 192.0.2.1 ping statistics ---\n3 packets transmitted, 0 received, 100% packet loss, time 410ms\n\n";
    const UNKNOWN: &str = "ping: nonexistent.invalid: Name or service not known\n";

    fn status(host: &str, state: HostState) -> HostStatus {
        HostStatus {
            host: host.into(),
            label: String::new(),
            state,
        }
    }

    #[test]
    fn ping_results_from_captured_output() {
        assert_eq!(
            parse_ping(Some(0), UP_V4, ""),
            HostState::Up {
                latency_ms: Some(47.937)
            }
        );
        assert_eq!(
            parse_ping(Some(0), UP_V6, ""),
            HostState::Up {
                latency_ms: Some(0.284)
            }
        );
        assert_eq!(
            parse_ping(Some(1), NO_REPLY, ""),
            HostState::Down {
                reason: "No reply".into()
            }
        );
        assert_eq!(
            parse_ping(Some(2), "", UNKNOWN),
            HostState::Down {
                reason: "Unknown host".into()
            }
        );
        // iputils' wording when there is no default route (not captured:
        // that would mean taking the owner's network down).
        assert_eq!(
            parse_ping(Some(2), "", "ping: connect: Network is unreachable\n"),
            HostState::Down {
                reason: "Network unreachable".into()
            }
        );
        assert_eq!(
            parse_ping(None, "", ""),
            HostState::Down {
                reason: "ping failed".into()
            }
        );
        // Exit 0 without a summary (never seen, but not a failure).
        assert_eq!(
            parse_ping(Some(0), "", ""),
            HostState::Up { latency_ms: None }
        );
    }

    #[test]
    fn targets_are_reduced_to_a_host() {
        let n = |s: &str| normalize_target(s);
        assert_eq!(n("8.8.8.8").as_deref(), Some("8.8.8.8"));
        assert_eq!(n(" Example.COM ").as_deref(), Some("example.com"));
        assert_eq!(
            n("https://grafana.lan:3000/d/abc?x=1#y").as_deref(),
            Some("grafana.lan")
        );
        assert_eq!(n("ssh://kenan@nas.local:22").as_deref(), Some("nas.local"));
        assert_eq!(n("192.168.1.10:8080").as_deref(), Some("192.168.1.10"));
        assert_eq!(n("[2001:db8::1]:443").as_deref(), Some("2001:db8::1"));
        assert_eq!(n("http://[fd00::5]/").as_deref(), Some("fd00::5"));
        assert_eq!(n("fe80::1").as_deref(), Some("fe80::1"));
        assert_eq!(n("router.").as_deref(), Some("router"));
        for bad in [
            "",
            "   ",
            "-c 1000",
            "-fhost",
            "a b",
            "host;rm",
            "999.1.1.1",
            "1.2.3",
            "-bad.example",
            "bad-.example",
            "under_score.lan",
            "http://",
            "[::1",
        ] {
            assert_eq!(n(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn new_hosts_are_unique() {
        let existing = vec![MonitoredHost {
            host: "nas.lan".into(),
            label: "NAS".into(),
        }];
        assert_eq!(
            validate_new("https://router.lan/", &existing),
            Ok("router.lan".into())
        );
        assert!(validate_new("NAS.lan", &existing).is_err());
        assert!(validate_new("not a host", &existing).is_err());
    }

    #[test]
    fn the_ping_argv_is_exact() {
        assert_eq!(
            ping_args("nas.lan", 2),
            [
                "-n", "-q", "-c", "3", "-i", "0.2", "-W", "2", "--", "nas.lan"
            ]
        );
        let cfg = HostMonitorConfig {
            interval_seconds: 1,
            timeout_seconds: 99,
            ..Default::default()
        };
        assert_eq!(interval(&cfg), Duration::from_secs(5));
        assert_eq!(timeout_secs(&cfg), 10);
        assert_eq!(
            interval(&HostMonitorConfig::default()),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn notifications_on_down_and_recovery_only() {
        let mut last = HashMap::new();
        let up = || HostState::Up {
            latency_ms: Some(3.0),
        };
        let down = || HostState::Down {
            reason: "No reply".into(),
        };
        // Not checked yet: nothing.
        assert!(transitions(&mut last, &[status("a", HostState::Checking)]).is_empty());
        // First result: up is quiet, down is reported.
        let t = transitions(&mut last, &[status("a", up()), status("b", down())]);
        assert_eq!(
            t,
            [Transition::Down {
                name: "b".into(),
                reason: "No reply".into()
            }]
        );
        // Still down: quiet. a goes down (with its label), b recovers.
        assert!(transitions(&mut last, &[status("a", up()), status("b", down())]).is_empty());
        let mut a = status("a", down());
        a.label = "Router".into();
        let t = transitions(&mut last, &[a, status("b", up())]);
        assert_eq!(
            t,
            [
                Transition::Down {
                    name: "Router".into(),
                    reason: "No reply".into()
                },
                Transition::Recovered { name: "b".into() }
            ]
        );
        // A removed host is forgotten: re-added and down, it reports again.
        assert!(transitions(&mut last, &[status("a", down())]).is_empty());
        assert_eq!(last.len(), 1);
        assert_eq!(
            transitions(&mut last, &[status("a", down()), status("b", down())]).len(),
            1
        );
        assert_eq!(down_count(&[status("a", down()), status("b", up())]), 1);
    }

    #[test]
    fn rows_follow_the_config() {
        let hosts = vec![
            MonitoredHost {
                host: "b".into(),
                label: "B".into(),
            },
            MonitoredHost {
                host: "new".into(),
                label: String::new(),
            },
        ];
        let checked = vec![
            status("gone", HostState::Up { latency_ms: None }),
            status("b", HostState::Up { latency_ms: None }),
        ];
        let r = rows(&hosts, &checked);
        assert_eq!(r.len(), 2);
        assert_eq!(
            (r[0].display(), &r[0].state),
            ("B", &HostState::Up { latency_ms: None })
        );
        assert_eq!((r[1].display(), &r[1].state), ("new", &HostState::Checking));
    }

    #[test]
    fn latency_wording_and_tiers() {
        assert_eq!(format_latency(0.284), "0.3 ms");
        assert_eq!(format_latency(47.937), "48 ms");
        assert_eq!(format_latency(1530.0), "1.5 s");
        assert_eq!(latency_tier(99.9), LatencyTier::Good);
        assert_eq!(latency_tier(100.0), LatencyTier::Slow);
        assert_eq!(latency_tier(300.0), LatencyTier::Bad);
        assert_eq!(display_name("nas.lan", "  "), "nas.lan");
        assert_eq!(display_name("nas.lan", " NAS "), "NAS");
    }
}
