//! CPU temperature via `/sys/class/thermal/`.
//!
//! Picks the first `thermal_zone*` whose `type` looks CPU-shaped
//! (x86_pkg_temp / cpu_thermal / coretemp). Returns the temperature
//! in degrees Celsius (sysfs reports milli-Celsius).

use std::path::PathBuf;

pub fn current_celsius() -> Option<u8> {
    let zone = find_zone()?;
    let raw = std::fs::read_to_string(zone.join("temp")).ok()?;
    let mc: i64 = raw.trim().parse().ok()?;
    Some(((mc / 1000).clamp(0, 200)) as u8)
}

fn find_zone() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/class/thermal");
    let mut fallback: Option<PathBuf> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("thermal_zone") {
            continue;
        }
        let kind = std::fs::read_to_string(path.join("type"))
            .ok()
            .map(|s| s.trim().to_string());
        match kind.as_deref() {
            Some("x86_pkg_temp" | "cpu_thermal" | "coretemp") => return Some(path),
            _ => {
                if fallback.is_none() {
                    fallback = Some(path);
                }
            }
        }
    }
    fallback
}
