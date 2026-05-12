//! Memory usage from `/proc/meminfo`. Returns the % of physical RAM
//! currently in use, computed the same way `free -h` does
//! (`(MemTotal - MemAvailable) / MemTotal`).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// 0..=100
    pub used_percent: u8,
    /// Used / total in KiB. Handy for the popup detail card later.
    pub used_kib: u64,
    pub total_kib: u64,
}

impl Snapshot {
    pub fn current() -> Option<Self> {
        let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
        let mut total: Option<u64> = None;
        let mut available: Option<u64> = None;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                total = parse_kib(rest);
            } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                available = parse_kib(rest);
            }
            if total.is_some() && available.is_some() {
                break;
            }
        }
        let total_kib = total?;
        let available_kib = available?;
        let used_kib = total_kib.saturating_sub(available_kib);
        let used_percent = if total_kib > 0 {
            ((used_kib as f64 / total_kib as f64) * 100.0).round() as u8
        } else {
            0
        };
        Some(Self {
            used_percent: used_percent.min(100),
            used_kib,
            total_kib,
        })
    }
}

/// `/proc/meminfo` rows look like `MemTotal:       16308228 kB`.
/// Strip whitespace and the trailing `kB`, parse the integer.
fn parse_kib(rest: &str) -> Option<u64> {
    rest.trim_end_matches("kB").trim().parse().ok()
}
