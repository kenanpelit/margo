//! Hardware probe for the setup wizard — decides which optional steps
//! are even relevant (no Touchpad step on a desktop, no Wi-Fi step
//! without a wireless card, no Power step on AC-only machines, no
//! Display step on a single monitor). Each field has a pure parser
//! (unit-tested) + a thin `probe()` that reads the real system.

#[derive(Debug, Clone, Copy)]
pub(crate) struct HwInfo {
    pub has_touchpad: bool,
    pub has_wifi: bool,
    pub has_battery: bool,
    pub monitor_count: usize,
}

pub(crate) fn parse_touchpad(proc_devices: &str) -> bool {
    proc_devices
        .lines()
        .any(|l| l.to_ascii_lowercase().contains("touchpad"))
}

pub(crate) fn parse_wifi(nmcli_device: &str) -> bool {
    // `nmcli -t -f DEVICE,TYPE,STATE device` → `name:type:state` per line.
    nmcli_device
        .lines()
        .any(|l| l.split(':').nth(1) == Some("wifi"))
}

pub(crate) fn parse_battery(supply_names: &[String]) -> bool {
    supply_names
        .iter()
        .any(|n| n.to_ascii_uppercase().starts_with("BAT"))
}

pub(crate) fn parse_monitor_count(drm_statuses: &[String]) -> usize {
    drm_statuses
        .iter()
        .filter(|s| s.trim() == "connected")
        .count()
}

impl HwInfo {
    pub(crate) fn probe() -> Self {
        let proc_devices = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
        let nmcli = std::process::Command::new("nmcli")
            .args(["-t", "-f", "DEVICE,TYPE,STATE", "device"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let supply: Vec<String> = std::fs::read_dir("/sys/class/power_supply")
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let drm: Vec<String> = std::fs::read_dir("/sys/class/drm")
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        std::fs::read_to_string(e.path().join("status"))
                            .ok()
                            .map(|s| s.trim().to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let monitor_count = parse_monitor_count(&drm).max(1);
        HwInfo {
            has_touchpad: parse_touchpad(&proc_devices),
            has_wifi: parse_wifi(&nmcli),
            has_battery: parse_battery(&supply),
            monitor_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touchpad_detected_from_proc_devices() {
        let s = "N: Name=\"SynPS/2 Synaptics TouchPad\"\nN: Name=\"AT Keyboard\"\n";
        assert!(parse_touchpad(s));
        assert!(!parse_touchpad("N: Name=\"Logitech Mouse\"\n"));
    }

    #[test]
    fn wifi_detected_from_nmcli_device() {
        assert!(parse_wifi(
            "wlan0:wifi:connected\neth0:ethernet:connected\n"
        ));
        assert!(!parse_wifi("eth0:ethernet:connected\n"));
    }

    #[test]
    fn battery_detected_from_supply_names() {
        assert!(parse_battery(&["AC".into(), "BAT0".into()]));
        assert!(!parse_battery(&["AC".into()]));
    }

    #[test]
    fn monitor_count_from_drm_status() {
        assert_eq!(
            parse_monitor_count(&["connected".into(), "disconnected".into()]),
            1
        );
        assert_eq!(
            parse_monitor_count(&["connected".into(), "connected".into()]),
            2
        );
    }
}
