//! USB-C Power Delivery status for the power menu — ports the useful
//! half of the noctalia `usbeehive` plugin natively, reading sysfs
//! directly instead of shelling out to the `usbeehive` binary.
//!
//! Sources, per USB-C port:
//! * `/sys/class/typec/portN/power_role` — `source [sink]`: the bracketed
//!   role is the live one (sink = this machine is being charged, source =
//!   it is powering the partner).
//! * `/sys/class/typec/portN-partner` — present while something is
//!   plugged in.
//! * `/sys/class/typec/portN/power_operation_mode` — `usb_power_delivery`
//!   / `3.0A` / `1.5A` / `default`.
//! * The port's power supply (`ucsi-source-psy-<con>:<NNN>`, NNN = 1-based
//!   connector; `tcpm-source-psy-…` on TCPM machines) for `online`,
//!   `voltage_now` and `current_now`/`current_max` (µV / µA).
//! * The partner's `usb_power_delivery/source-capabilities/*` PDOs when
//!   the kernel exposes them, for the charger's maximum wattage.
//!
//! Many UCSI firmwares (the owner's ThinkPad included) report the
//! negotiated current but `voltage_now = 0` and no PDOs. Then the
//! wattage is unknown and we show just the current — no guessing.

use std::fs;
use std::path::{Path, PathBuf};

const TYPEC_ROOT: &str = "/sys/class/typec";
const PSY_ROOT: &str = "/sys/class/power_supply";

/// Live power role of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerRole {
    /// This machine is receiving power (charger plugged in).
    Sink,
    /// This machine is supplying power to the partner.
    Source,
    Unknown,
}

/// One USB-C port with something plugged in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UsbCPort {
    /// 1-based port number for display (`port0` → 1).
    pub(crate) number: u32,
    pub(crate) role: PowerRole,
    /// Power Delivery negotiated (vs. Type-C current / USB default).
    pub(crate) pd: bool,
    /// The port's power supply says power is flowing (`online = 1`).
    pub(crate) online: bool,
    /// Live voltage in volts, when the firmware reports it.
    pub(crate) volts: Option<f64>,
    /// Negotiated current in amperes, when reported.
    pub(crate) amps: Option<f64>,
    /// Charger maximum from its PD source PDOs, in watts.
    pub(crate) max_watts: Option<f64>,
}

impl UsbCPort {
    /// Live wattage: voltage × current when both are known.
    pub(crate) fn watts(&self) -> Option<f64> {
        match (self.volts, self.amps) {
            (Some(v), Some(a)) => Some(v * a),
            _ => None,
        }
    }

    /// `USB-C 1` title.
    pub(crate) fn title(&self) -> String {
        format!("USB-C {}", self.number)
    }

    /// One-line status: `Charging · 65 W · charger up to 65 W · PD`.
    pub(crate) fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(
            match (self.role, self.online) {
                (PowerRole::Sink, true) => "Charging",
                (PowerRole::Sink, false) => "Not charging",
                (PowerRole::Source, _) => "Powering a device",
                (PowerRole::Unknown, _) => "Connected",
            }
            .to_string(),
        );
        if let Some(w) = self.watts() {
            parts.push(format!("{w:.0} W"));
        } else if let Some(a) = self.amps
            && self.online
        {
            parts.push(format!("{a:.2} A"));
        }
        if let Some(max) = self.max_watts {
            parts.push(format!("charger up to {max:.0} W"));
        }
        if self.pd {
            parts.push("PD".to_string());
        }
        parts.join("  ·  ")
    }
}

/// Parse a sysfs role line: the bracketed word is the active one.
pub(crate) fn parse_power_role(s: &str) -> PowerRole {
    let active = s
        .split_whitespace()
        .find(|w| w.starts_with('[') && w.ends_with(']'))
        .map(|w| w.trim_matches(|c| c == '[' || c == ']'));
    match active {
        Some("sink") => PowerRole::Sink,
        Some("source") => PowerRole::Source,
        _ => PowerRole::Unknown,
    }
}

/// Map a power-supply name to its 0-based typec port index:
/// `ucsi-source-psy-USBC000:001` → 0, `tcpm-source-psy-…` has no index.
pub(crate) fn psy_port_index(name: &str) -> Option<u32> {
    if !name.starts_with("ucsi-source-psy-") {
        return None;
    }
    let n: u32 = name.rsplit(':').next()?.parse().ok()?;
    n.checked_sub(1)
}

/// Fixed-supply PDO wattage from a `source-capabilities/<n>:fixed_supply`
/// dir's `voltage` (`20000mV`) and `maximum_current` (`3250mA`).
pub(crate) fn pdo_watts(voltage: &str, current: &str) -> Option<f64> {
    let mv: f64 = voltage.trim().trim_end_matches("mV").parse().ok()?;
    let ma: f64 = current.trim().trim_end_matches("mA").parse().ok()?;
    (mv > 0.0 && ma > 0.0).then_some(mv * ma / 1_000_000.0)
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Micro-unit sysfs integer → unit, `None` when missing or zero.
fn read_micro(p: &Path) -> Option<f64> {
    let v: f64 = read(p)?.parse().ok()?;
    (v > 0.0).then_some(v / 1_000_000.0)
}

/// Max fixed-supply wattage the partner advertises, if exposed.
fn partner_max_watts(partner: &Path) -> Option<f64> {
    let caps = partner.join("usb_power_delivery/source-capabilities");
    let mut best: Option<f64> = None;
    for e in fs::read_dir(caps).ok()?.flatten() {
        let d = e.path();
        if let (Some(v), Some(c)) = (read(&d.join("voltage")), read(&d.join("maximum_current")))
            && let Some(w) = pdo_watts(&v, &c)
        {
            best = Some(best.map_or(w, |b: f64| b.max(w)));
        }
    }
    best
}

/// Scan the USB-C ports under the given sysfs roots (seams for tests).
pub(crate) fn scan_at(typec_root: &Path, psy_root: &Path) -> Vec<UsbCPort> {
    // Power supplies keyed by port index.
    let mut psys: Vec<(u32, PathBuf)> = Vec::new();
    if let Ok(rd) = fs::read_dir(psy_root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(i) = psy_port_index(&name) {
                psys.push((i, e.path()));
            }
        }
    }
    let mut ports = Vec::new();
    let Ok(rd) = fs::read_dir(typec_root) else {
        return ports;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(idx) = name
            .strip_prefix("port")
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue; // partners / cables / plugs
        };
        let port = e.path();
        let partner = typec_root.join(format!("{name}-partner"));
        if !partner.exists() {
            continue; // nothing plugged in
        }
        let role = read(&port.join("power_role"))
            .map(|s| parse_power_role(&s))
            .unwrap_or(PowerRole::Unknown);
        let pd = read(&port.join("power_operation_mode")).as_deref() == Some("usb_power_delivery");
        let psy = psys.iter().find(|(i, _)| *i == idx).map(|(_, p)| p);
        let online = psy
            .and_then(|p| read(&p.join("online")))
            .is_some_and(|s| s == "1");
        let volts = psy.and_then(|p| read_micro(&p.join("voltage_now")));
        let amps = psy.and_then(|p| {
            read_micro(&p.join("current_now")).or_else(|| read_micro(&p.join("current_max")))
        });
        ports.push(UsbCPort {
            number: idx + 1,
            role,
            pd,
            online,
            volts,
            amps,
            max_watts: partner_max_watts(&partner),
        });
    }
    ports.sort_by_key(|p| p.number);
    ports
}

/// Scan the live sysfs. Cheap (a handful of tiny reads); empty on
/// machines without USB-C / typec class support.
pub(crate) fn scan() -> Vec<UsbCPort> {
    scan_at(Path::new(TYPEC_ROOT), Path::new(PSY_ROOT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    /// The owner's ThinkPad, captured from sysfs: port0 is sinking from a
    /// PD charger (UCSI reports 3.25 A but voltage 0, no PDOs), port1 is
    /// sourcing a PD device; its psy is offline.
    fn thinkpad(root: &Path) -> (PathBuf, PathBuf) {
        let typec = root.join("typec");
        let psy = root.join("power_supply");
        write(&typec.join("port0/power_role"), "source [sink]\n");
        write(
            &typec.join("port0/power_operation_mode"),
            "usb_power_delivery\n",
        );
        write(
            &typec.join("port0-partner/usb_power_delivery/revision"),
            "2.0\n",
        );
        write(&typec.join("port1/power_role"), "[source] sink\n");
        write(
            &typec.join("port1/power_operation_mode"),
            "usb_power_delivery\n",
        );
        fs::create_dir_all(typec.join("port1-partner")).unwrap();
        let p1 = psy.join("ucsi-source-psy-USBC000:001");
        write(&p1.join("online"), "1\n");
        write(&p1.join("voltage_now"), "0\n");
        write(&p1.join("current_now"), "3250000\n");
        write(&p1.join("current_max"), "0\n");
        let p2 = psy.join("ucsi-source-psy-USBC000:002");
        write(&p2.join("online"), "0\n");
        write(&p2.join("voltage_now"), "0\n");
        write(&p2.join("current_now"), "3000000\n");
        fs::create_dir_all(psy.join("BAT0")).unwrap();
        fs::create_dir_all(psy.join("AC")).unwrap();
        (typec, psy)
    }

    #[test]
    fn parses_roles() {
        assert_eq!(parse_power_role("source [sink]"), PowerRole::Sink);
        assert_eq!(parse_power_role("[source] sink"), PowerRole::Source);
        assert_eq!(parse_power_role("source sink"), PowerRole::Unknown);
        assert_eq!(parse_power_role(""), PowerRole::Unknown);
    }

    #[test]
    fn maps_ucsi_psy_names_to_ports() {
        assert_eq!(psy_port_index("ucsi-source-psy-USBC000:001"), Some(0));
        assert_eq!(psy_port_index("ucsi-source-psy-USBC000:002"), Some(1));
        assert_eq!(psy_port_index("ucsi-source-psy-USBC000:000"), None);
        assert_eq!(psy_port_index("BAT0"), None);
        assert_eq!(psy_port_index("tcpm-source-psy-4-0022"), None);
    }

    #[test]
    fn pdo_wattage() {
        assert_eq!(pdo_watts("20000mV", "3250mA"), Some(65.0));
        assert_eq!(pdo_watts("5000mV\n", "3000mA\n"), Some(15.0));
        assert_eq!(pdo_watts("0mV", "3000mA"), None);
        assert_eq!(pdo_watts("x", "3000mA"), None);
    }

    #[test]
    fn thinkpad_ports_without_voltage_show_current_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        let (typec, psy) = thinkpad(dir.path());
        let ports = scan_at(&typec, &psy);
        assert_eq!(ports.len(), 2);
        let charger = &ports[0];
        assert_eq!(charger.number, 1);
        assert_eq!(charger.role, PowerRole::Sink);
        assert!(charger.pd && charger.online);
        assert_eq!(charger.volts, None);
        assert_eq!(charger.amps, Some(3.25));
        assert_eq!(charger.watts(), None);
        assert_eq!(charger.summary(), "Charging  ·  3.25 A  ·  PD");
        let device = &ports[1];
        assert_eq!(device.role, PowerRole::Source);
        assert_eq!(device.summary(), "Powering a device  ·  PD");
    }

    #[test]
    fn full_reporting_firmware_shows_watts_and_charger_max() {
        let dir = tempfile::tempdir().unwrap();
        let (typec, psy) = thinkpad(dir.path());
        let p1 = psy.join("ucsi-source-psy-USBC000:001");
        write(&p1.join("voltage_now"), "20000000\n");
        let caps = typec.join("port0-partner/usb_power_delivery/source-capabilities");
        write(&caps.join("1:fixed_supply/voltage"), "5000mV\n");
        write(&caps.join("1:fixed_supply/maximum_current"), "3000mA\n");
        write(&caps.join("4:fixed_supply/voltage"), "20000mV\n");
        write(&caps.join("4:fixed_supply/maximum_current"), "3250mA\n");
        let ports = scan_at(&typec, &psy);
        assert_eq!(ports[0].watts(), Some(65.0));
        assert_eq!(ports[0].max_watts, Some(65.0));
        assert_eq!(
            ports[0].summary(),
            "Charging  ·  65 W  ·  charger up to 65 W  ·  PD"
        );
    }

    #[test]
    fn empty_ports_and_missing_class_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let typec = dir.path().join("typec");
        write(&typec.join("port0/power_role"), "[source] sink\n");
        assert!(scan_at(&typec, &dir.path().join("none")).is_empty());
        assert!(scan_at(&dir.path().join("missing"), &dir.path().join("none")).is_empty());
    }
}
