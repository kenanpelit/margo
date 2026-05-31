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

/// Public re-export: every temperature sensor under `/sys/class/hwmon`
/// as `(label, °C)`, sorted with CPU/GPU first. Generalises the
/// single-CPU probe so the dashboard can show GPU / NVMe / chipset
/// temps alongside the package temp.
pub(crate) fn read_all_temp_sensors_pub() -> Vec<(String, i32)> {
    read_all_temp_sensors()
}

/// Public re-export: every fan under `/sys/class/hwmon` as
/// `(label, rpm)`.
pub(crate) fn read_all_fans_pub() -> Vec<(String, u32)> {
    read_all_fans()
}

/// Map a raw hwmon chip `name` to a friendly category prefix. Keeps
/// the sensor list readable ("CPU", "GPU", "NVMe") instead of driver
/// names ("k10temp", "amdgpu", "nvme").
fn friendly_chip(name: &str) -> &str {
    match name {
        "coretemp" | "k10temp" | "zenpower" => "CPU",
        "amdgpu" | "nouveau" | "nvidia" | "radeon" => "GPU",
        "nvme" => "NVMe",
        "drivetemp" => "Disk",
        "acpitz" => "ACPI",
        "iwlwifi" | "iwlwifi_1" => "WiFi",
        other => other,
    }
}

/// Rank for sort order — CPU first, then GPU, NVMe/Disk, the rest.
fn chip_rank(friendly: &str) -> u8 {
    match friendly {
        "CPU" => 0,
        "GPU" => 1,
        "NVMe" | "Disk" => 2,
        _ => 3,
    }
}

/// Walk `/sys/class/hwmon`, reading every `tempN_input`. Labels come
/// from the matching `tempN_label` when present, else `<chip> N`.
/// Implausible readings (≤ 0 or > 150 °C) are dropped — some chips
/// expose unwired channels that read garbage.
fn read_all_temp_sensors() -> Vec<(String, i32)> {
    let mut out: Vec<(u8, String, i32)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let chip = std::fs::read_to_string(dir.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let friendly = friendly_chip(&chip).to_string();
        let rank = chip_rank(&friendly);

        for n in 1..=16 {
            let input = dir.join(format!("temp{n}_input"));
            let Some(milli) = read_temp_millideg(&input) else {
                continue;
            };
            let celsius = milli / 1000;
            if celsius <= 0 || celsius > 150 {
                continue;
            }
            let label = std::fs::read_to_string(dir.join(format!("temp{n}_label")))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|l| format!("{friendly}: {l}"))
                .unwrap_or_else(|| {
                    if friendly == chip {
                        format!("{friendly} {n}")
                    } else {
                        friendly.clone()
                    }
                });
            out.push((rank, label, celsius));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, l, c)| (l, c)).collect()
}

/// Walk `/sys/class/hwmon`, reading every `fanN_input` (RPM). Labels
/// from `fanN_label` when present, else `<chip> Fan N`.
fn read_all_fans() -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let chip = std::fs::read_to_string(dir.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let friendly = friendly_chip(&chip).to_string();

        for n in 1..=8 {
            let input = dir.join(format!("fan{n}_input"));
            let Ok(raw) = std::fs::read_to_string(&input) else {
                continue;
            };
            let Ok(rpm) = raw.trim().parse::<u32>() else {
                continue;
            };
            let label = std::fs::read_to_string(dir.join(format!("fan{n}_label")))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if friendly == chip && chip.is_empty() {
                        format!("Fan {n}")
                    } else {
                        format!("{friendly} Fan {n}")
                    }
                });
            out.push((label, rpm));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
