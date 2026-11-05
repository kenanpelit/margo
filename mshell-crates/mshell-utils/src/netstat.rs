//! Network-interface byte counters — the one `/proc/net/dev` parser and the
//! one "is this a real link?" rule, shared by the bar's network speed, the
//! network menu graph, the speed-test gauge, the VPN pill and the VPN menu
//! graph.
//!
//! *Physical* means the kernel has a backing device for the interface
//! (`/sys/class/net/<iface>/device` exists): Wi-Fi, Ethernet, USB
//! tethering. Loopback, VPN tunnels (`wg0-mullvad`, `tun0`, `proton0`,
//! `tailscale0`), bridges, veths, bonds and VLANs are virtual — their
//! traffic is also counted on a physical link, so summing them would count
//! the same bytes twice.

use std::collections::BTreeMap;
use std::path::Path;

/// Per-interface cumulative `(rx_bytes, tx_bytes)`, by interface name.
pub type Counters = BTreeMap<String, (u64, u64)>;

const SYS_CLASS_NET: &str = "/sys/class/net";

/// Parse `/proc/net/dev` into per-interface `(rx, tx)` byte counters
/// (`lo` excluded). Rx bytes is the first column, tx bytes the ninth.
pub fn parse_proc_net_dev(s: &str) -> Counters {
    let mut map = Counters::new();
    for line in s.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" || name.contains('|') {
            continue;
        }
        let cols: Vec<u64> = rest
            .split_whitespace()
            .map_while(|x| x.parse().ok())
            .collect();
        if cols.len() >= 9 {
            map.insert(name.to_string(), (cols[0], cols[8]));
        }
    }
    map
}

/// The live `/proc/net/dev` counters (empty when unreadable). A procfs read
/// — no I/O wait — so it is fine on any thread.
pub fn read_proc_net_dev() -> Counters {
    std::fs::read_to_string("/proc/net/dev")
        .map(|s| parse_proc_net_dev(&s))
        .unwrap_or_default()
}

/// Summed `(rx, tx)` over the live physical interfaces — the machine's
/// real traffic, each byte counted once.
pub fn read_physical_totals() -> (u64, u64) {
    physical_totals(&read_proc_net_dev(), is_virtual)
}

/// `iface` has no backing hardware device (see the module docs).
pub fn is_virtual(iface: &str) -> bool {
    is_virtual_in(Path::new(SYS_CLASS_NET), iface)
}

/// [`is_virtual`] against an explicit `/sys/class/net` root (testable).
/// Without sysfs for the interface (a sandbox, or it just vanished) the
/// usual virtual name prefixes decide.
pub fn is_virtual_in(sys_class_net: &Path, iface: &str) -> bool {
    let dir = sys_class_net.join(iface);
    if dir.exists() {
        return !dir.join("device").exists();
    }
    const VIRTUAL_PREFIXES: [&str; 9] = [
        "lo",
        "veth",
        "br-",
        "docker",
        "virbr",
        "wg",
        "tun",
        "tap",
        "tailscale",
    ];
    VIRTUAL_PREFIXES.iter().any(|p| iface.starts_with(p))
}

/// Summed `(rx, tx)` over the physical interfaces in `counters`.
pub fn physical_totals(counters: &Counters, is_virtual: impl Fn(&str) -> bool) -> (u64, u64) {
    counters
        .iter()
        .filter(|(name, _)| !is_virtual(name))
        .fold((0u64, 0u64), |(rx, tx), (_, &(r, t))| {
            (rx.saturating_add(r), tx.saturating_add(t))
        })
}

/// Live `(rx, tx)` bytes/s between two snapshots: the busiest interface per
/// direction, not the sum — with a VPN up the same bytes cross both the
/// tunnel and the physical link, so summing would double the reading.
pub fn busiest_rates(prev: &Counters, cur: &Counters, secs: f64) -> (f64, f64) {
    if secs <= 0.0 {
        return (0.0, 0.0);
    }
    let mut rx: f64 = 0.0;
    let mut tx: f64 = 0.0;
    for (name, &(r, t)) in cur {
        if let Some(&(pr, pt)) = prev.get(name) {
            rx = rx.max(r.saturating_sub(pr) as f64 / secs);
            tx = tx.max(t.saturating_sub(pt) as f64 / secs);
        }
    }
    (rx, tx)
}

/// Bytes/s between two counter readings. A counter that went backwards
/// (interface re-created between samples) reads as 0, not a huge spike.
pub fn counter_rate(prev: u64, cur: u64, secs: f64) -> u64 {
    if secs <= 0.0 || cur < prev {
        return 0;
    }
    ((cur - prev) as f64 / secs).round() as u64
}

/// `(rx_bytes, tx_bytes)` of one interface from sysfs, `None` when it is
/// gone.
pub fn iface_counters(iface: &str) -> Option<(u64, u64)> {
    iface_counters_in(Path::new(SYS_CLASS_NET), iface)
}

/// [`iface_counters`] against an explicit `/sys/class/net` root (testable).
pub fn iface_counters_in(sys_class_net: &Path, iface: &str) -> Option<(u64, u64)> {
    let stats = sys_class_net.join(iface).join("statistics");
    let read = |f: &str| -> Option<u64> {
        std::fs::read_to_string(stats.join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

/// Interface names from `/sys/class/net`.
pub fn list_interfaces() -> Vec<String> {
    std::fs::read_dir(SYS_CLASS_NET)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `/proc/net/dev` (header + lo + Wi-Fi + Mullvad tunnel + docker).
    const NET_DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 9000 100 0 0 0 0 0 0 9000 100 0 0 0 0 0 0
 wlan0: 5000000 4000 0 0 0 0 0 0 700000 3000 0 0 0 0 0 0
wg0-mullvad: 4800000 3900 0 0 0 0 0 0 650000 2900 0 0 0 0 0 0
docker0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0
";

    /// A fake `/sys/class/net`: wlan0 has a device, the rest don't.
    fn fake_sysfs() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for iface in ["lo", "wlan0", "wg0-mullvad", "docker0"] {
            std::fs::create_dir_all(root.path().join(iface).join("statistics")).unwrap();
        }
        std::fs::create_dir(root.path().join("wlan0/device")).unwrap();
        std::fs::write(root.path().join("wlan0/statistics/rx_bytes"), "5000000\n").unwrap();
        std::fs::write(root.path().join("wlan0/statistics/tx_bytes"), "700000\n").unwrap();
        root
    }

    #[test]
    fn parses_counters_without_lo_or_the_header() {
        let c = parse_proc_net_dev(NET_DEV);
        assert_eq!(c.len(), 3);
        assert_eq!(c["wlan0"], (5_000_000, 700_000));
        assert_eq!(c["wg0-mullvad"], (4_800_000, 650_000));
        assert!(!c.contains_key("lo"));
    }

    /// The bug: the bar's network speed summed every non-docker interface,
    /// so with the VPN up the tunnel's bytes were counted on top of the
    /// Wi-Fi bytes that carry them — the pill showed about double.
    #[test]
    fn totals_count_only_physical_links() {
        let sys = fake_sysfs();
        let c = parse_proc_net_dev(NET_DEV);
        let totals = physical_totals(&c, |i| is_virtual_in(sys.path(), i));
        assert_eq!(totals, (5_000_000, 700_000));
    }

    #[test]
    fn without_sysfs_the_name_decides() {
        let none = Path::new("/nonexistent-sysfs");
        for v in [
            "lo",
            "veth12ab",
            "br-1234",
            "docker0",
            "virbr0",
            "wg0",
            "tun0",
            "tailscale0",
        ] {
            assert!(is_virtual_in(none, v), "{v}");
        }
        for p in ["wlan0", "enp0s31f6", "eth0"] {
            assert!(!is_virtual_in(none, p), "{p}");
        }
    }

    #[test]
    fn busiest_interface_per_direction() {
        let a = parse_proc_net_dev(NET_DEV);
        let mut b = a.clone();
        b.insert("wlan0".into(), (5_000_000 + 2_000, 700_000 + 400));
        b.insert("wg0-mullvad".into(), (4_800_000 + 1_800, 650_000 + 600));
        assert_eq!(busiest_rates(&a, &b, 2.0), (1_000.0, 300.0));
        assert_eq!(busiest_rates(&a, &b, 0.0), (0.0, 0.0));
    }

    #[test]
    fn counter_rates() {
        assert_eq!(counter_rate(1_000, 3_000, 2.0), 1_000);
        assert_eq!(
            counter_rate(5_000, 1_000, 1.0),
            0,
            "a reset counter is not a spike"
        );
        assert_eq!(counter_rate(0, 100, 0.0), 0);
    }

    #[test]
    fn reads_one_interface_from_sysfs() {
        let sys = fake_sysfs();
        assert_eq!(
            iface_counters_in(sys.path(), "wlan0"),
            Some((5_000_000, 700_000))
        );
        assert_eq!(iface_counters_in(sys.path(), "docker0"), None);
        assert_eq!(iface_counters_in(sys.path(), "gone0"), None);
    }
}
