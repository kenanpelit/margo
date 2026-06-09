//! ICMP latency measurement via the system `ping`.
//!
//! Relays are pinged on their public IPv4 directly (no need to connect first),
//! which makes "fastest" measurable + parallelisable.

use std::process::Command;
use std::sync::mpsc;
use std::thread;

/// Average RTT in ms for `ip`, or `None` on failure / no reply.
pub fn ping_avg(ip: &str, count: u32, timeout: u32) -> Option<f64> {
    let out = Command::new("ping")
        .args(["-c", &count.to_string(), "-W", &timeout.to_string(), ip])
        .output()
        .ok()?;
    parse_avg(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the `rtt min/avg/max/mdev = a/b/c/d ms` summary → avg (b).
pub fn parse_avg(s: &str) -> Option<f64> {
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("rtt") || t.starts_with("round-trip") {
            // "... = 0.296/0.314/0.339/0.000 ms" → split on '/', avg is [4].
            let parts: Vec<&str> = t.split('/').collect();
            if parts.len() >= 5 {
                return parts[4].trim().parse::<f64>().ok();
            }
        }
    }
    None
}

/// Ping many (label, ip) pairs in parallel, returning (label, avg_ms) for the
/// ones that responded. Bounded fan-out keeps it from spawning hundreds of
/// threads on a big relay set.
pub fn ping_many(targets: &[(String, String)], count: u32, timeout: u32) -> Vec<(String, f64)> {
    const MAX_INFLIGHT: usize = 16;
    let mut results = Vec::new();
    for chunk in targets.chunks(MAX_INFLIGHT) {
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();
        for (label, ip) in chunk {
            let (label, ip, tx) = (label.clone(), ip.clone(), tx.clone());
            handles.push(thread::spawn(move || {
                if let Some(avg) = ping_avg(&ip, count, timeout) {
                    let _ = tx.send((label, avg));
                }
            }));
        }
        drop(tx);
        while let Ok(r) = rx.recv() {
            results.push(r);
        }
        for h in handles {
            let _ = h.join();
        }
    }
    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_rtt() {
        let s = "PING 1.2.3.4 ...\n\
                 rtt min/avg/max/mdev = 0.296/0.314/0.339/0.000 ms";
        assert_eq!(parse_avg(s), Some(0.314));
    }

    #[test]
    fn parses_bsd_roundtrip() {
        let s = "round-trip min/avg/max/stddev = 10.1/12.5/15.0/1.0 ms";
        assert_eq!(parse_avg(s), Some(12.5));
    }

    #[test]
    fn none_on_no_summary() {
        assert_eq!(parse_avg("100% packet loss"), None);
    }
}
