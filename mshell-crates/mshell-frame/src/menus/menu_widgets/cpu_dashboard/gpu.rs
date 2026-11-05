//! Intel iGPU load for the CPU dashboard: busy % + frequency from sysfs
//! (`xe` and `i915`) and per-process GPU usage from DRM fdinfo
//! (`/proc/<pid>/fdinfo/<fd>`, the kernel's drm-usage-stats format —
//! what nvtop / intel_gpu_top read). Pure parsers + rate maths, tested
//! against real samples from the owner's Meteor Lake (`xe`) laptop.
//!
//! Busy % is "time the GT was out of its idle (RC6 / C6) state" over the
//! poll window — the same signal intel_gpu_top shows as RC6 inverse. It
//! needs no root, unlike the PMU engine counters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Driver {
    Xe,
    I915,
}

/// One GT (graphics tile) whose idle residency we can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GtPaths {
    /// `render` / `media` (xe `gt0-rc` / `gt1-mc`), or `gt0` (i915).
    pub label: String,
    pub idle_ms: PathBuf,
    pub act_freq: PathBuf,
    pub max_freq: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Igpu {
    pub driver: Driver,
    /// `card0`.
    pub card: String,
    pub gts: Vec<GtPaths>,
}

fn driver_of(card_dir: &Path) -> Option<Driver> {
    let link = std::fs::read_link(card_dir.join("device/driver")).ok()?;
    match link.file_name()?.to_str()? {
        "xe" => Some(Driver::Xe),
        "i915" => Some(Driver::I915),
        _ => None,
    }
}

/// xe `gtidle/name` (`gt0-rc`, `gt1-mc`) → a human label.
pub(crate) fn xe_gt_label(name: &str) -> String {
    if name.ends_with("-rc") {
        "render".to_string()
    } else if name.ends_with("-mc") {
        "media".to_string()
    } else {
        name.to_string()
    }
}

/// Find the first Intel GPU under `drm_root` (`/sys/class/drm`).
pub(crate) fn find_igpu(drm_root: &Path) -> Option<Igpu> {
    let mut cards: Vec<_> = std::fs::read_dir(drm_root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.to_string();
            (n.starts_with("card") && n[4..].bytes().all(|b| b.is_ascii_digit()) && n.len() > 4)
                .then_some(n)
        })
        .collect();
    cards.sort();
    for card in cards {
        let dir = drm_root.join(&card);
        let Some(driver) = driver_of(&dir) else {
            continue;
        };
        let gts = match driver {
            Driver::Xe => xe_gts(&dir.join("device")),
            Driver::I915 => i915_gts(&dir),
        };
        if !gts.is_empty() {
            return Some(Igpu { driver, card, gts });
        }
    }
    None
}

fn sorted_dirs(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn xe_gts(device: &Path) -> Vec<GtPaths> {
    let mut out = Vec::new();
    for tile in sorted_dirs(device, "tile") {
        for gt in sorted_dirs(&tile, "gt") {
            let idle = gt.join("gtidle/idle_residency_ms");
            if !idle.exists() {
                continue;
            }
            let name = std::fs::read_to_string(gt.join("gtidle/name")).unwrap_or_default();
            out.push(GtPaths {
                label: xe_gt_label(name.trim()),
                idle_ms: idle,
                act_freq: gt.join("freq0/act_freq"),
                max_freq: gt.join("freq0/max_freq"),
            });
        }
    }
    out
}

fn i915_gts(card: &Path) -> Vec<GtPaths> {
    let mut out = Vec::new();
    for gt in sorted_dirs(&card.join("gt"), "gt") {
        let idle = gt.join("rc6_residency_ms");
        if idle.exists() {
            out.push(GtPaths {
                label: gt
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                idle_ms: idle,
                act_freq: gt.join("rps_act_freq_mhz"),
                max_freq: gt.join("rps_max_freq_mhz"),
            });
        }
    }
    if out.is_empty() && card.join("power/rc6_residency_ms").exists() {
        // Older kernels: card-level files only.
        out.push(GtPaths {
            label: "gt0".to_string(),
            idle_ms: card.join("power/rc6_residency_ms"),
            act_freq: card.join("gt_act_freq_mhz"),
            max_freq: card.join("gt_max_freq_mhz"),
        });
    }
    out
}

fn read_u64(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// One sysfs read of a GT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct GtSample {
    pub idle_ms: u64,
    pub act_mhz: u64,
    pub max_mhz: u64,
}

pub(crate) fn sample_gt(gt: &GtPaths) -> Option<GtSample> {
    Some(GtSample {
        idle_ms: read_u64(&gt.idle_ms)?,
        act_mhz: read_u64(&gt.act_freq).unwrap_or(0),
        max_mhz: read_u64(&gt.max_freq).unwrap_or(0),
    })
}

/// Busy % over a window: wall time not spent in the idle state.
pub(crate) fn busy_pct(prev_idle_ms: u64, idle_ms: u64, wall_ms: u64) -> f32 {
    if wall_ms == 0 {
        return 0.0;
    }
    let idle = idle_ms.saturating_sub(prev_idle_ms).min(wall_ms);
    100.0 - idle as f32 * 100.0 / wall_ms as f32
}

// ── Per-process usage from DRM fdinfo ───────────────────────────────

/// One engine counter from fdinfo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EngineCounter {
    /// xe: `drm-cycles-<e>`; i915 & others: `drm-engine-<e>` (ns).
    pub busy: u64,
    /// xe only: `drm-total-cycles-<e>` (the GPU timestamp the busy cycles
    /// are measured against). 0 for ns-based drivers.
    pub total: u64,
    pub capacity: u32,
}

/// The DRM usage stats of one client (one open DRM file).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DrmClient {
    pub driver: String,
    pub pdev: String,
    pub client_id: u64,
    pub engines: HashMap<String, EngineCounter>,
    /// Sum of `drm-resident-<region>` in KiB.
    pub resident_kb: u64,
}

fn kib(v: &str) -> u64 {
    let mut it = v.split_whitespace();
    let n: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    match it.next() {
        Some("MiB") => n * 1024,
        Some("GiB") => n * 1024 * 1024,
        Some("KiB") => n,
        _ => n / 1024, // plain bytes
    }
}

/// Parse a `/proc/<pid>/fdinfo/<fd>` file; `None` if it isn't a DRM
/// client with usage stats.
pub(crate) fn parse_fdinfo(s: &str) -> Option<DrmClient> {
    let mut c = DrmClient::default();
    let mut has_id = false;
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        let num = || {
            v.split_whitespace()
                .next()
                .and_then(|x| x.parse::<u64>().ok())
        };
        if k == "drm-driver" {
            c.driver = v.to_string();
        } else if k == "drm-pdev" {
            c.pdev = v.to_string();
        } else if k == "drm-client-id" {
            c.client_id = num()?;
            has_id = true;
        } else if let Some(e) = k.strip_prefix("drm-total-cycles-") {
            c.engines.entry(e.to_string()).or_default().total = num().unwrap_or(0);
        } else if let Some(e) = k.strip_prefix("drm-cycles-") {
            c.engines.entry(e.to_string()).or_default().busy = num().unwrap_or(0);
        } else if let Some(e) = k.strip_prefix("drm-engine-capacity-") {
            c.engines.entry(e.to_string()).or_default().capacity = num().unwrap_or(1) as u32;
        } else if let Some(e) = k.strip_prefix("drm-engine-") {
            c.engines.entry(e.to_string()).or_default().busy = num().unwrap_or(0);
        } else if k.starts_with("drm-resident-") {
            c.resident_kb += kib(v);
        }
    }
    (has_id && !c.driver.is_empty()).then_some(c)
}

/// Busiest engine of a client between two samples: `(engine, percent)`.
/// xe compares busy cycles to the GPU timestamp delta; ns drivers compare
/// busy ns to the wall-clock window. Divided by engine capacity (e.g. two
/// `vcs` instances).
pub(crate) fn client_busy(prev: &DrmClient, now: &DrmClient, wall_ns: u64) -> (String, f32) {
    let mut best = (String::new(), 0.0f32);
    for (name, e) in &now.engines {
        let Some(p) = prev.engines.get(name) else {
            continue;
        };
        let d_busy = e.busy.saturating_sub(p.busy) as f64;
        let denom = if e.total > 0 {
            e.total.saturating_sub(p.total) as f64
        } else {
            wall_ns as f64
        };
        if denom <= 0.0 {
            continue;
        }
        let pct = (d_busy / denom * 100.0 / e.capacity.max(1) as f64).min(100.0) as f32;
        if pct > best.1 || best.0.is_empty() {
            best = (name.clone(), pct);
        }
    }
    best
}

/// A per-process GPU row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GpuProc {
    pub pid: i32,
    pub name: String,
    pub busy: f32,
    pub engine: String,
    pub resident_kb: u64,
}

/// Clients keyed by `(pid, pdev, client-id)`: several fds of one process
/// can share a client (dup'd / inherited fds) and must count once.
pub(crate) type ClientKey = (i32, String, u64);

/// Fold two client sweeps into per-process rows (sum of a process's
/// clients, capped at 100%), busiest first; idle processes that still
/// hold GPU memory stay listed below the busy ones.
pub(crate) fn gpu_procs(
    prev: &HashMap<ClientKey, DrmClient>,
    now: &HashMap<ClientKey, DrmClient>,
    names: &HashMap<i32, String>,
    wall_ns: u64,
) -> Vec<GpuProc> {
    let mut by_pid: HashMap<i32, GpuProc> = HashMap::new();
    for (key, c) in now {
        let (engine, busy) = prev
            .get(key)
            .map(|p| client_busy(p, c, wall_ns))
            .unwrap_or_default();
        let e = by_pid.entry(key.0).or_insert_with(|| GpuProc {
            pid: key.0,
            name: names.get(&key.0).cloned().unwrap_or_default(),
            busy: 0.0,
            engine: String::new(),
            resident_kb: 0,
        });
        if busy > 0.0 && (e.engine.is_empty() || busy > e.busy) {
            e.engine = engine;
        }
        e.busy = (e.busy + busy).min(100.0);
        e.resident_kb += c.resident_kb;
    }
    let mut out: Vec<GpuProc> = by_pid
        .into_values()
        .filter(|p| p.busy > 0.0 || p.resident_kb > 0)
        .collect();
    out.sort_by(|a, b| {
        b.busy
            .total_cmp(&a.busy)
            .then(b.resident_kb.cmp(&a.resident_kb))
            .then(a.pid.cmp(&b.pid))
    });
    out
}

/// A DRM fd found by `find_drm_fds`: `(pid, fd number)`.
pub(crate) type DrmFd = (i32, String);

/// Sweep `<proc_root>/*/fd/*` for fds pointing at `/dev/dri/*`, in our
/// own processes only: [`read_clients`] can read nobody else's fdinfo, so
/// readlinking another user's fds (about half of the ~11k open fds here)
/// was wasted work. The caller still repeats this only every few polls
/// and re-reads the known fds in between. Blocking: call off the GTK
/// thread.
pub(crate) fn find_drm_fds(proc_root: &Path) -> Vec<DrmFd> {
    use std::os::unix::fs::MetadataExt;
    let own = std::fs::metadata(proc_root.join("self"))
        .ok()
        .map(|m| m.uid());
    find_drm_fds_of(proc_root, own)
}

/// [`find_drm_fds`] for the processes of `uid` (every process when
/// `None`: the owner is unknown).
fn find_drm_fds_of(proc_root: &Path, uid: Option<u32>) -> Vec<DrmFd> {
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(proc_root) else {
        return out;
    };
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        // `/proc/<pid>` is owned by the process's (effective) uid.
        if let Some(uid) = uid
            && entry.metadata().map(|m| m.uid()).ok() != Some(uid)
        {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let is_drm = std::fs::read_link(fd.path())
                .map(|t| t.starts_with("/dev/dri/"))
                .unwrap_or(false);
            if is_drm {
                out.push((pid, fd.file_name().to_string_lossy().into_owned()));
            }
        }
    }
    out
}

/// Read the fdinfo of `fds`. `/proc/<pid>/fdinfo` is only readable for
/// our own processes, so other users' clients are silently absent; fds
/// closed since the sweep just drop out. Blocking.
pub(crate) fn read_clients(
    proc_root: &Path,
    fds: &[DrmFd],
) -> (HashMap<ClientKey, DrmClient>, HashMap<i32, String>) {
    let mut clients = HashMap::new();
    let mut names = HashMap::new();
    for (pid, fd) in fds {
        let pdir = proc_root.join(pid.to_string());
        let Some(c) = std::fs::read_to_string(pdir.join("fdinfo").join(fd))
            .ok()
            .and_then(|s| parse_fdinfo(&s))
        else {
            continue;
        };
        names.entry(*pid).or_insert_with(|| {
            std::fs::read_to_string(pdir.join("comm"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        });
        clients.insert((*pid, c.pdev.clone(), c.client_id), c);
    }
    (clients, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XE: &str = include_str!("testdata/fdinfo_xe.txt");

    #[test]
    fn parses_real_xe_fdinfo() {
        let c = parse_fdinfo(XE).expect("drm client");
        assert_eq!(c.driver, "xe");
        assert_eq!(c.pdev, "0000:00:02.0");
        assert_eq!(c.client_id, 15);
        assert_eq!(c.resident_kb, 209_380);
        let rcs = c.engines["rcs"];
        assert_eq!(rcs.busy, 45_880_549);
        assert_eq!(rcs.total, 33_328_881_722);
        assert_eq!(c.engines["vcs"].capacity, 2);
        assert_eq!(c.engines.len(), 5);
        assert!(parse_fdinfo("pos:\t0\nflags:\t02\n").is_none(), "not DRM");
    }

    #[test]
    fn xe_busy_uses_gpu_timestamp_and_capacity() {
        let prev = parse_fdinfo(XE).expect("client");
        let mut now = prev.clone();
        // rcs busy for a quarter of the timestamp window.
        if let Some(e) = now.engines.get_mut("rcs") {
            e.busy += 250;
            e.total += 1000;
        }
        // vcs: 2 instances, busy cycles = whole window → 50%.
        if let Some(e) = now.engines.get_mut("vcs") {
            e.busy += 1000;
            e.total += 1000;
        }
        let (engine, pct) = client_busy(&prev, &now, 0);
        assert_eq!(engine, "vcs");
        assert!((pct - 50.0).abs() < 0.01, "{pct}");
    }

    #[test]
    fn i915_style_ns_counters() {
        // drm-usage-stats ns format (Documentation/gpu/drm-usage-stats.rst).
        let a = "drm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t7\n\
                 drm-engine-render:\t1000000000 ns\ndrm-engine-video:\t0 ns\n\
                 drm-engine-capacity-video:\t2\ndrm-resident-system0:\t8 MiB\n";
        let b = a.replace("1000000000 ns", "1500000000 ns");
        let pa = parse_fdinfo(a).expect("a");
        let pb = parse_fdinfo(&b).expect("b");
        assert_eq!(pa.resident_kb, 8192);
        let (engine, pct) = client_busy(&pa, &pb, 2_000_000_000);
        assert_eq!(engine, "render");
        assert!((pct - 25.0).abs() < 0.01);
    }

    #[test]
    fn per_process_fold() {
        let base = parse_fdinfo(XE).expect("client");
        let mut busy = base.clone();
        if let Some(e) = busy.engines.get_mut("rcs") {
            e.busy += 100;
            e.total += 1000;
        }
        let mut prev = HashMap::new();
        let mut now = HashMap::new();
        prev.insert((10, base.pdev.clone(), 1), base.clone());
        now.insert((10, base.pdev.clone(), 1), busy.clone());
        prev.insert((10, base.pdev.clone(), 2), base.clone());
        now.insert((10, base.pdev.clone(), 2), busy);
        // New client this sweep (no baseline): memory only.
        now.insert((20, base.pdev.clone(), 3), base.clone());
        let mut idle = base.clone();
        idle.resident_kb = 0;
        now.insert((30, base.pdev.clone(), 4), idle);
        let names = HashMap::from([(10, "mshell".to_string()), (20, "margo".to_string())]);
        let rows = gpu_procs(&prev, &now, &names, 0);
        assert_eq!(rows.len(), 2, "pid 30 has no load and no memory");
        assert_eq!(rows[0].pid, 10);
        assert_eq!(rows[0].name, "mshell");
        assert!((rows[0].busy - 20.0).abs() < 0.01, "two clients × 10%");
        assert_eq!(rows[0].engine, "rcs");
        assert_eq!(rows[0].resident_kb, 2 * 209_380);
        assert_eq!(rows[1].pid, 20);
        assert_eq!(rows[1].busy, 0.0);
    }

    #[test]
    fn finds_and_reads_drm_fds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let p = root.join("2199");
        std::fs::create_dir_all(p.join("fd")).expect("mkdir");
        std::fs::create_dir_all(p.join("fdinfo")).expect("mkdir");
        std::fs::write(p.join("comm"), "mshell\n").expect("w");
        std::os::unix::fs::symlink("/dev/dri/renderD128", p.join("fd/63")).expect("ln");
        std::os::unix::fs::symlink("/dev/null", p.join("fd/0")).expect("ln");
        std::fs::write(p.join("fdinfo/63"), XE).expect("w");
        std::fs::create_dir_all(root.join("self")).expect("mkdir");
        let fds = find_drm_fds(root);
        assert_eq!(fds, [(2199, "63".to_string())]);
        let (clients, names) = read_clients(root, &fds);
        assert_eq!(clients.len(), 1);
        assert!(clients.contains_key(&(2199, "0000:00:02.0".to_string(), 15)));
        assert_eq!(names[&2199], "mshell");
        // A closed fd simply drops out.
        let (gone, _) = read_clients(root, &[(2199, "99".to_string())]);
        assert!(gone.is_empty());
    }

    #[test]
    fn drm_sweep_skips_other_users_processes() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let p = root.join("4242");
        std::fs::create_dir_all(p.join("fd")).expect("mkdir");
        std::os::unix::fs::symlink("/dev/dri/card0", p.join("fd/7")).expect("ln");
        let me = std::fs::metadata(&p).expect("stat").uid();
        assert_eq!(find_drm_fds_of(root, Some(me)), [(4242, "7".to_string())]);
        // Another user's process: its fds are never readlinked.
        assert!(find_drm_fds_of(root, Some(me.wrapping_add(1))).is_empty());
        assert_eq!(
            find_drm_fds_of(root, None).len(),
            1,
            "owner unknown: sweep all"
        );
    }

    #[test]
    fn busy_from_idle_residency() {
        assert_eq!(busy_pct(1000, 2000, 1000), 0.0);
        assert_eq!(busy_pct(1000, 1500, 2000), 75.0);
        assert_eq!(busy_pct(1000, 1000, 2000), 100.0);
        assert_eq!(busy_pct(1000, 9000, 2000), 0.0, "clamped");
        assert_eq!(busy_pct(0, 0, 0), 0.0);
        assert_eq!(xe_gt_label("gt0-rc"), "render");
        assert_eq!(xe_gt_label("gt1-mc"), "media");
    }

    #[test]
    fn finds_xe_gpu_in_a_sysfs_tree() {
        // Mirrors /sys/class/drm/card0 on the owner's Meteor Lake (xe).
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let drivers = root.join("bus/xe");
        std::fs::create_dir_all(&drivers).expect("mkdir");
        let dev = root.join("card0/device");
        for (gt, name) in [("gt0", "gt0-rc"), ("gt1", "gt1-mc")] {
            let g = dev.join("tile0").join(gt);
            std::fs::create_dir_all(g.join("gtidle")).expect("mkdir");
            std::fs::create_dir_all(g.join("freq0")).expect("mkdir");
            std::fs::write(g.join("gtidle/name"), format!("{name}\n")).expect("w");
            std::fs::write(g.join("gtidle/idle_residency_ms"), "1624271\n").expect("w");
            std::fs::write(g.join("freq0/act_freq"), "0\n").expect("w");
            std::fs::write(g.join("freq0/max_freq"), "2250\n").expect("w");
        }
        std::os::unix::fs::symlink(&drivers, dev.join("driver")).expect("symlink");
        std::fs::create_dir_all(root.join("card0-eDP-1")).expect("mkdir");
        let gpu = find_igpu(root).expect("igpu");
        assert_eq!(gpu.driver, Driver::Xe);
        assert_eq!(gpu.card, "card0");
        let labels: Vec<_> = gpu.gts.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["render", "media"]);
        assert_eq!(
            sample_gt(&gpu.gts[0]),
            Some(GtSample {
                idle_ms: 1_624_271,
                act_mhz: 0,
                max_mhz: 2250
            })
        );
        assert!(find_igpu(Path::new("/nonexistent")).is_none());
    }
}
