//! System-stat readers shared by the CPU dashboard.
//!
//! The standalone CPU-load / RAM / CPU-temp bar pills were removed
//! in favour of the combined `CpuDashboard` pill + menu. What stays
//! here is the procfs/sysfs parsing — a single source of truth for
//! `/proc/stat` and the hwmon temperature probe — re-exported with
//! the `*_pub` wrappers that `cpu_dashboard.rs` and the CPU
//! dashboard menu widget consume.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Public re-export of the cpu-stat reader for `cpu_dashboard.rs`
/// — keeps a single source of truth for `/proc/stat` parsing
/// instead of duplicating the column layout.
pub(crate) fn read_cpu_stat_pub() -> (u64, u64) {
    read_cpu_stat()
}

/// Public re-export of the temperature-millideg reader.
pub(crate) fn read_temp_millideg_pub(path: &std::path::PathBuf) -> Option<i32> {
    read_temp_millideg(path)
}

/// Public re-export of the hwmon sensor probe.
pub(crate) fn find_cpu_temp_sensor_pub() -> Option<std::path::PathBuf> {
    find_cpu_temp_sensor()
}

fn read_cpu_stat() -> (u64, u64) {
    let Ok(s) = std::fs::read_to_string("/proc/stat") else {
        return (0, 0);
    };
    let Some(first) = s.lines().next() else {
        return (0, 0);
    };
    // Format: "cpu user nice system idle iowait irq softirq steal guest guest_nice"
    let parts: Vec<u64> = first
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() < 4 {
        return (0, 0);
    }
    let total: u64 = parts.iter().sum();
    // `idle` (col 3) + `iowait` (col 4) both count as not-busy.
    let idle = parts[3] + parts.get(4).copied().unwrap_or(0);
    (total, idle)
}

/// Locate a CPU package temperature sensor under
/// `/sys/class/hwmon`. Preferred drivers in order: `coretemp`
/// (Intel), `k10temp` (AMD), `zenpower` (newer AMD third-party),
/// `acpitz` (generic ACPI thermal zone — used by ThinkPads, etc).
/// Caches `temp1_input` once found; the chosen device's path
/// won't move at runtime.
fn find_cpu_temp_sensor() -> Option<PathBuf> {
    // Logged once on cold-start so a missing sensor traces to a
    // recognisable place.
    static LOGGED: AtomicBool = AtomicBool::new(false);

    let preferred = ["coretemp", "k10temp", "zenpower", "acpitz"];
    for want in preferred {
        let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
            return None;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
                continue;
            };
            if name.trim() == want {
                let p = dir.join("temp1_input");
                if p.exists() {
                    if !LOGGED.swap(true, Ordering::Relaxed) {
                        tracing::info!(
                            sensor = %name.trim(),
                            path = %p.display(),
                            "sysstat: cpu temperature sensor selected"
                        );
                    }
                    return Some(p);
                }
            }
        }
    }
    None
}

fn read_temp_millideg(path: &PathBuf) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
