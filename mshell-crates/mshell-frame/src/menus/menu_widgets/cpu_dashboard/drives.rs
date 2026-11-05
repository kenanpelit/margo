//! Drive health for the CPU dashboard's Drives tab.
//!
//! Two tiers, like the drive-health plugin:
//! - **Unprivileged** (always): NVMe controllers from
//!   `/sys/class/nvme/*` (model, serial, firmware) + the controller's
//!   hwmon "Composite" temperature, and the device list from
//!   `smartctl -j --scan` (which needs no root).
//! - **Full SMART** (on request): `smartctl -j -a` needs root for NVMe
//!   (`/dev/nvme0` is `root:root 0600`), so it runs once, for every
//!   device, behind ONE `pkexec` prompt. A non-root attempt is detected
//!   from smartctl's own JSON (`exit_status` bit 1 + "Permission denied")
//!   and reported as "needs authorization", never as an error.

use serde_json::Value;
use std::path::Path;

/// One NVMe controller as seen from sysfs (no root needed).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SysDrive {
    /// `/dev/nvme0`.
    pub dev: String,
    pub model: String,
    pub serial: String,
    pub firmware: String,
    /// hwmon `temp1_input` (Composite) in °C.
    pub temp_c: Option<i32>,
}

/// Enumerate `<root>/nvme*` (`root` = `/sys/class/nvme`).
pub(crate) fn read_sys_drives(root: &Path) -> Vec<SysDrive> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let read = |p: &Path| {
        std::fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut out: Vec<SysDrive> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.starts_with("nvme") {
                return None;
            }
            let p = e.path();
            let temp_c = std::fs::read_dir(&p).ok().and_then(|d| {
                d.flatten()
                    .filter(|h| h.file_name().to_string_lossy().starts_with("hwmon"))
                    .find_map(|h| {
                        std::fs::read_to_string(h.path().join("temp1_input"))
                            .ok()
                            .and_then(|s| s.trim().parse::<i32>().ok())
                            .map(|m| m / 1000)
                    })
            });
            Some(SysDrive {
                dev: format!("/dev/{name}"),
                model: read(&p.join("model")),
                serial: read(&p.join("serial")),
                firmware: read(&p.join("firmware_rev")),
                temp_c,
            })
        })
        .collect();
    out.sort_by(|a, b| a.dev.cmp(&b.dev));
    out
}

/// Device paths from `smartctl -j --scan`.
pub(crate) fn parse_scan(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    v["devices"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|d| d["name"].as_str())
                .filter(|n| valid_device(n))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Only plain `/dev/<alnum>` paths reach the privileged command line.
pub(crate) fn valid_device(dev: &str) -> bool {
    dev.strip_prefix("/dev/").is_some_and(|n| {
        !n.is_empty() && n.len() <= 32 && n.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// The SMART fields the panel shows. `None` = not reported by this drive.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct SmartReport {
    pub dev: String,
    pub model: String,
    pub passed: Option<bool>,
    pub temp_c: Option<i64>,
    /// NVMe "Percentage Used" — vendor estimate of rated endurance spent.
    pub wear_pct: Option<u64>,
    pub spare_pct: Option<u64>,
    pub spare_threshold_pct: Option<u64>,
    pub critical_warning: Option<u64>,
    pub media_errors: Option<u64>,
    pub error_log_entries: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub power_on_hours: Option<u64>,
    /// Host writes in TB (NVMe data units are 512 000 bytes).
    pub written_tb: Option<f64>,
    /// ATA: reallocated + pending + offline-uncorrectable sectors.
    pub bad_sectors: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SmartRead {
    Report(SmartReport),
    /// smartctl could not open the device without root.
    NeedsRoot(String),
    Error(String),
}

/// Parse one `smartctl -j -a <dev>` document.
pub(crate) fn parse_smart_value(v: &Value) -> SmartRead {
    let dev = v["device"]["name"]
        .as_str()
        .or_else(|| {
            v["smartctl"]["argv"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|x| x.as_str())
        })
        .unwrap_or("?")
        .to_string();
    let exit = v["smartctl"]["exit_status"].as_u64().unwrap_or(0);
    let messages: Vec<&str> = v["smartctl"]["messages"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["string"].as_str()).collect())
        .unwrap_or_default();
    // Bits 0-1 are fatal (bad command line / device open failed); bit 2 is
    // "some command failed" but the document may still carry data.
    if exit & 0b11 != 0 {
        if messages.iter().any(|m| m.contains("Permission denied")) {
            return SmartRead::NeedsRoot(dev);
        }
        let msg = messages.first().copied().unwrap_or("smartctl failed");
        return SmartRead::Error(if msg.contains(&dev) {
            msg.to_string()
        } else {
            format!("{dev}: {msg}")
        });
    }
    let nv = &v["nvme_smart_health_information_log"];
    let u = |x: &Value| x.as_u64();
    let ata_raw = |id: u64| -> Option<u64> {
        v["ata_smart_attributes"]["table"]
            .as_array()?
            .iter()
            .find(|a| a["id"].as_u64() == Some(id))
            .and_then(|a| a["raw"]["value"].as_u64())
    };
    let bad: Vec<u64> = [5, 197, 198].iter().filter_map(|id| ata_raw(*id)).collect();
    SmartRead::Report(SmartReport {
        dev,
        model: v["model_name"].as_str().unwrap_or_default().to_string(),
        passed: v["smart_status"]["passed"].as_bool(),
        temp_c: v["temperature"]["current"]
            .as_i64()
            .or_else(|| nv["temperature"].as_i64()),
        wear_pct: u(&nv["percentage_used"]),
        spare_pct: u(&nv["available_spare"]),
        spare_threshold_pct: u(&nv["available_spare_threshold"]),
        critical_warning: u(&nv["critical_warning"]),
        media_errors: u(&nv["media_errors"]),
        error_log_entries: u(&nv["num_err_log_entries"]),
        unsafe_shutdowns: u(&nv["unsafe_shutdowns"]),
        power_on_hours: u(&v["power_on_time"]["hours"]).or_else(|| u(&nv["power_on_hours"])),
        written_tb: u(&nv["data_units_written"]).map(|d| d as f64 * 512_000.0 / 1e12),
        bad_sectors: (!bad.is_empty()).then(|| bad.iter().sum()),
    })
}

/// Parse the concatenated JSON documents the batched privileged run
/// prints (one per device).
pub(crate) fn parse_smart_stream(out: &str) -> Vec<SmartRead> {
    serde_json::Deserializer::from_str(out)
        .into_iter::<Value>()
        .map_while(Result::ok)
        .map(|v| parse_smart_value(&v))
        .collect()
}

/// Severity of a drive (good / needs attention / failing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Health {
    Good,
    Warn(Vec<String>),
    Bad(Vec<String>),
}

impl Health {
    /// The dashboard's semantic label tiers (amber `--warning`, red
    /// `--error`); healthy stays the plain value colour.
    pub(crate) fn class(&self) -> &'static str {
        match self {
            Health::Good => "",
            Health::Warn(_) => "metric-warning",
            Health::Bad(_) => "metric-critical",
        }
    }
}

/// Wear at or above this is a warning (vendor endurance nearly spent).
const WEAR_WARN_PCT: u64 = 80;
/// NVMe composite temperature warning threshold.
const TEMP_WARN_C: i64 = 70;

pub(crate) fn assess(r: &SmartReport) -> Health {
    let mut bad = Vec::new();
    let mut warn = Vec::new();
    if r.passed == Some(false) {
        bad.push("SMART self-assessment FAILED".to_string());
    }
    if let Some(c) = r.critical_warning.filter(|c| *c != 0) {
        bad.push(format!("critical warning flags 0x{c:02x}"));
    }
    if let Some(m) = r.media_errors.filter(|m| *m > 0) {
        bad.push(format!("{m} media errors"));
    }
    if let (Some(s), Some(t)) = (r.spare_pct, r.spare_threshold_pct)
        && s < t
    {
        bad.push(format!("spare {s}% below threshold {t}%"));
    }
    if let Some(b) = r.bad_sectors.filter(|b| *b > 0) {
        warn.push(format!("{b} bad sectors"));
    }
    if let Some(w) = r.wear_pct.filter(|w| *w >= WEAR_WARN_PCT) {
        warn.push(format!("{w}% of rated endurance used"));
    }
    if let Some(t) = r.temp_c.filter(|t| *t >= TEMP_WARN_C) {
        warn.push(format!("running hot ({t}°C)"));
    }
    if !bad.is_empty() {
        bad.extend(warn);
        Health::Bad(bad)
    } else if !warn.is_empty() {
        Health::Warn(warn)
    } else {
        Health::Good
    }
}

/// The single privileged invocation: `pkexec mshell-privileged
/// smart-read <devs>`. The helper re-validates every device as root
/// (a drive node right under /dev, never a symlink) and runs a fixed
/// `smartctl -j -a` per device; `valid_device` filters them here first.
pub(crate) fn privileged_argv(devs: &[String]) -> Option<Vec<String>> {
    if devs.is_empty() || !devs.iter().all(|d| valid_device(d)) {
        return None;
    }
    let req = mshell_privileged::Request::SmartRead {
        devices: devs.to_vec(),
    }
    .validated()
    .ok()?;
    Some(mshell_privileged::lossy(&req.argv()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONROOT: &str = include_str!("testdata/smartctl_nvme0_nonroot.json");
    const SCAN: &str = include_str!("testdata/smartctl_scan.json");
    /// Schema fixture (smartctl 7.5 NVMe layout). A real root capture
    /// wasn't possible from the agent session (no root), so values are
    /// plausible, not this machine's.
    const FULL: &str = include_str!("testdata/smartctl_nvme_schema.json");

    #[test]
    fn real_nonroot_output_means_needs_root() {
        let v: Value = serde_json::from_str(NONROOT).expect("json");
        assert_eq!(
            parse_smart_value(&v),
            SmartRead::NeedsRoot("/dev/nvme0".to_string())
        );
    }

    #[test]
    fn real_scan_lists_both_nvme_controllers() {
        assert_eq!(parse_scan(SCAN), ["/dev/nvme0", "/dev/nvme1"]);
        assert!(parse_scan("not json").is_empty());
    }

    #[test]
    fn parses_full_nvme_report() {
        let v: Value = serde_json::from_str(FULL).expect("json");
        let SmartRead::Report(r) = parse_smart_value(&v) else {
            panic!("expected a report");
        };
        assert_eq!(r.dev, "/dev/nvme0");
        assert_eq!(r.model, "Timetec 35TT2280GEN4E-2TB");
        assert_eq!(r.passed, Some(true));
        assert_eq!(r.temp_c, Some(47));
        assert_eq!(r.wear_pct, Some(3));
        assert_eq!(r.spare_pct, Some(100));
        assert_eq!(r.media_errors, Some(0));
        assert_eq!(r.error_log_entries, Some(0));
        assert_eq!(r.unsafe_shutdowns, Some(37));
        assert_eq!(r.power_on_hours, Some(2961));
        let tb = r.written_tb.expect("written");
        assert!((tb - 15.647).abs() < 0.01, "{tb}");
        assert_eq!(r.bad_sectors, None);
        assert_eq!(assess(&r), Health::Good);
    }

    #[test]
    fn stream_of_documents() {
        let both = format!("{FULL}\n{NONROOT}");
        let reads = parse_smart_stream(&both);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], SmartRead::Report(_)));
        assert!(matches!(reads[1], SmartRead::NeedsRoot(_)));
        assert!(parse_smart_stream("").is_empty());
    }

    #[test]
    fn other_open_failures_are_errors() {
        let v: Value = serde_json::from_str(
            r#"{"smartctl":{"argv":["smartctl","-j","-a","/dev/sdz"],"exit_status":2,
               "messages":[{"string":"/dev/sdz: No such device","severity":"error"}]}}"#,
        )
        .expect("json");
        assert_eq!(
            parse_smart_value(&v),
            SmartRead::Error("/dev/sdz: No such device".to_string())
        );
    }

    #[test]
    fn ata_bad_sectors_and_assessment() {
        let v: Value = serde_json::from_str(
            r#"{"smartctl":{"exit_status":4},"device":{"name":"/dev/sda"},
               "smart_status":{"passed":true},"temperature":{"current":35},
               "ata_smart_attributes":{"table":[
                 {"id":5,"raw":{"value":8}},{"id":197,"raw":{"value":1}},{"id":9,"raw":{"value":100}}]}}"#,
        )
        .expect("json");
        let SmartRead::Report(r) = parse_smart_value(&v) else {
            panic!("bit 2 alone still yields a report");
        };
        assert_eq!(r.bad_sectors, Some(9));
        assert_eq!(assess(&r), Health::Warn(vec!["9 bad sectors".to_string()]));

        let failing = SmartReport {
            passed: Some(false),
            critical_warning: Some(4),
            spare_pct: Some(5),
            spare_threshold_pct: Some(10),
            wear_pct: Some(91),
            temp_c: Some(75),
            ..SmartReport::default()
        };
        let Health::Bad(why) = assess(&failing) else {
            panic!("failing drive must be Bad");
        };
        assert_eq!(why.len(), 5);
        assert_eq!(assess(&failing).class(), "metric-critical");
        assert_eq!(Health::Good.class(), "");
    }

    #[test]
    fn device_validation_and_argv() {
        assert!(valid_device("/dev/nvme0"));
        assert!(valid_device("/dev/sda"));
        assert!(!valid_device("/dev/../etc/shadow"));
        assert!(!valid_device("/dev/nvme0; rm -rf ~"));
        assert!(!valid_device("/dev/"));
        assert!(!valid_device("nvme0"));
        assert_eq!(privileged_argv(&[]), None);
        assert_eq!(privileged_argv(&["/dev/x y".to_string()]), None);
        let argv =
            privileged_argv(&["/dev/nvme0".to_string(), "/dev/nvme1".to_string()]).expect("argv");
        assert_eq!(
            argv,
            [
                mshell_privileged::HELPER,
                "smart-read",
                "/dev/nvme0",
                "/dev/nvme1"
            ]
        );
    }

    #[test]
    fn reads_sysfs_nvme_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let c = dir.path().join("nvme1");
        std::fs::create_dir_all(c.join("hwmon4")).expect("mkdir");
        std::fs::write(
            c.join("model"),
            "TS2TMTE400S                             \n",
        )
        .expect("w");
        std::fs::write(c.join("serial"), "I729380087          \n").expect("w");
        std::fs::write(c.join("firmware_rev"), "X\n").expect("w");
        std::fs::write(c.join("hwmon4/temp1_input"), "55850\n").expect("w");
        let d = read_sys_drives(dir.path());
        assert_eq!(
            d,
            [SysDrive {
                dev: "/dev/nvme1".into(),
                model: "TS2TMTE400S".into(),
                serial: "I729380087".into(),
                firmware: "X".into(),
                temp_c: Some(55),
            }]
        );
        assert!(read_sys_drives(Path::new("/nonexistent")).is_empty());
    }
}
