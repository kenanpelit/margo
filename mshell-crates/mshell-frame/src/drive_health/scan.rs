//! Which internal drives are attached, from sysfs (no root, no smartctl):
//! NVMe controllers (`/sys/class/nvme`) and non-removable SATA/SAS disks
//! (`/sys/block/sd*`, not behind USB). Temperatures come from the
//! controller's hwmon (NVMe) or the `drivetemp` hwmon (SATA, when that
//! module is loaded).

use std::path::Path;

use crate::menus::menu_widgets::cpu_dashboard::drives::read_sys_drives;

/// One drive seen in a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    /// Stable id: the serial number, else the device path.
    pub id: String,
    /// `/dev/nvme0`, `/dev/sda`.
    pub dev: String,
    pub model: String,
    pub temp_c: Option<i64>,
}

/// The id for a drive: its serial, else its device path.
pub(crate) fn drive_id(serial: &str, dev: &str) -> String {
    let serial = serial.trim();
    if serial.is_empty() {
        dev.to_string()
    } else {
        serial.to_string()
    }
}

/// First `temp1_input` (millidegrees) under `<dir>/hwmon*`, in °C.
fn hwmon_temp(dir: &Path) -> Option<i64> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|h| {
        if !h.file_name().to_string_lossy().starts_with("hwmon") {
            return None;
        }
        std::fs::read_to_string(h.path().join("temp1_input"))
            .ok()?
            .trim()
            .parse::<i64>()
            .ok()
            .map(|m| m / 1000)
    })
}

/// The serial in a SCSI VPD page 0x80 (`vpd_pg80`): a 4-byte header, then
/// the serial as ASCII (libata fills it for SATA disks).
pub(crate) fn vpd_serial(raw: &[u8]) -> String {
    raw.get(4..)
        .map(|b| {
            String::from_utf8_lossy(b)
                .trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .to_string()
        })
        .unwrap_or_default()
}

/// Scan `sys` (normally `/sys`).
pub(crate) fn scan(sys: &Path) -> Vec<Seen> {
    let mut out: Vec<Seen> = read_sys_drives(&sys.join("class/nvme"))
        .into_iter()
        .map(|d| Seen {
            id: drive_id(&d.serial, &d.dev),
            dev: d.dev,
            model: d.model,
            temp_c: d.temp_c.map(i64::from),
        })
        .collect();
    let Ok(blocks) = std::fs::read_dir(sys.join("block")) else {
        return out;
    };
    let mut sata: Vec<Seen> = blocks
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.starts_with("sd") || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return None;
            }
            let p = e.path();
            let read = |rel: &str| {
                std::fs::read_to_string(p.join(rel))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            };
            if read("removable") != "0" {
                return None;
            }
            // USB sticks and enclosures come and go; not "internal".
            let real = std::fs::canonicalize(&p).ok()?;
            if real.to_string_lossy().contains("/usb") {
                return None;
            }
            let dev = format!("/dev/{name}");
            let serial = std::fs::read(p.join("device/vpd_pg80"))
                .map(|raw| vpd_serial(&raw))
                .unwrap_or_default();
            Some(Seen {
                id: drive_id(&serial, &dev),
                model: read("device/model"),
                // drivetemp registers under device/hwmon/hwmonN.
                temp_c: hwmon_temp(&p.join("device/hwmon")),
                dev,
            })
        })
        .collect();
    sata.sort_by(|a, b| a.dev.cmp(&b.dev));
    out.extend(sata);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(p: &Path, rel: &str, body: &[u8]) {
        let f = p.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, body).unwrap();
    }

    #[test]
    fn scans_nvme_and_internal_sata_but_not_usb() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path();
        // NVMe controller (as on this machine).
        w(sys, "class/nvme/nvme1/model", b"TS2TMTE400S          \n");
        w(sys, "class/nvme/nvme1/serial", b"I729380087    \n");
        w(sys, "class/nvme/nvme1/hwmon3/temp1_input", b"41850\n");
        // Internal SATA disk with drivetemp.
        let real = sys.join("devices/pci0000:00/ata1/host0/target0:0:0/0:0:0:0/block/sda");
        w(&real, "removable", b"0\n");
        w(&real, "device/model", b"WDC WD40EFRX-68N\n");
        w(
            &real,
            "device/vpd_pg80",
            b"\x00\x80\x00\x14     WD-WCC7K1234567",
        );
        w(&real, "device/hwmon/hwmon5/temp1_input", b"33000\n");
        // A USB stick claiming to be non-removable.
        let usb = sys.join("devices/pci0000:00/usb1/1-1/host6/block/sdb");
        w(&usb, "removable", b"0\n");
        std::fs::create_dir_all(sys.join("block")).unwrap();
        std::os::unix::fs::symlink(&real, sys.join("block/sda")).unwrap();
        std::os::unix::fs::symlink(&usb, sys.join("block/sdb")).unwrap();
        let seen = scan(sys);
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(
            seen[0],
            Seen {
                id: "I729380087".into(),
                dev: "/dev/nvme1".into(),
                model: "TS2TMTE400S".into(),
                temp_c: Some(41),
            }
        );
        assert_eq!(seen[1].id, "WD-WCC7K1234567");
        assert_eq!(seen[1].dev, "/dev/sda");
        assert_eq!(seen[1].model, "WDC WD40EFRX-68N");
        assert_eq!(seen[1].temp_c, Some(33));
        assert!(scan(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn ids_fall_back_to_the_device() {
        assert_eq!(drive_id("  ", "/dev/sda"), "/dev/sda");
        assert_eq!(drive_id(" X1 ", "/dev/sda"), "X1");
        assert_eq!(vpd_serial(b"\x00\x80"), "");
    }
}
