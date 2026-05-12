//! Battery state via sysfs. Picks the first non-peripheral
//! `/sys/class/power_supply/*` entry whose `type` is `Battery`.
//!
//! No upower D-Bus dance yet; this is the minimum to drive the bar
//! ring + percentage. A future stage can swap the implementation
//! behind `Snapshot::current` for upower without changing the
//! callers (e.g. once we want time-to-full / time-to-empty for the
//! system popup).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Charging,
    Discharging,
    Full,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// 0..=100. `None` when no battery is present (desktop systems).
    pub capacity: u8,
    pub status: Status,
}

impl Snapshot {
    pub fn current() -> Option<Self> {
        let bat = find_battery()?;
        let capacity = read_u8(&bat.join("capacity"))?;
        let status = match read_string(&bat.join("status")).as_deref() {
            Some("Charging") => Status::Charging,
            Some("Discharging") => Status::Discharging,
            Some("Full" | "Not charging") => Status::Full,
            _ => Status::Unknown,
        };
        Some(Self { capacity, status })
    }
}

fn find_battery() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/class/power_supply");
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let path = entry.path();
        let kind = read_string(&path.join("type"));
        if kind.as_deref() == Some("Battery")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !is_peripheral(n))
        {
            return Some(path);
        }
    }
    None
}

/// Mouse / keyboard / headphone batteries also expose
/// `type = Battery`. Filter them out by the standard upower naming
/// convention (`hid-…`, `wireless-…`, etc.).
fn is_peripheral(name: &str) -> bool {
    name.starts_with("hid-")
        || name.starts_with("wireless-")
        || name.contains("mouse")
        || name.contains("keyboard")
        || name.contains("headset")
        || name.contains("headphone")
}

fn read_string(p: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn read_u8(p: &std::path::Path) -> Option<u8> {
    read_string(p).and_then(|s| s.parse().ok())
}
