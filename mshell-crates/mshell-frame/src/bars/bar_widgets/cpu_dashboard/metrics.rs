//! The CPU Dashboard pill's metrics, GTK-free: which metrics show, their
//! readouts, the per-metric calm / warn / danger ladder (DESIGN.md §2),
//! "unavailable" handling, the disk-for-path and network-total picks, and
//! the hover table rows.

use mshell_config::schema::config::CpuDashboardBarWidget;
use std::path::Path;

/// One metric the capsule can show, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Metric {
    Cpu,
    Temp,
    Gpu,
    Ram,
    Swap,
    Disk,
    Net,
}

impl Metric {
    pub(crate) const ALL: [Metric; 7] = [
        Metric::Cpu,
        Metric::Temp,
        Metric::Gpu,
        Metric::Ram,
        Metric::Swap,
        Metric::Disk,
        Metric::Net,
    ];

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Metric::Cpu => "cpu-symbolic",
            Metric::Temp => "temperature-symbolic",
            Metric::Gpu => "video-display-symbolic",
            Metric::Ram => "memory-symbolic",
            Metric::Swap => "drive-multidisk-symbolic",
            Metric::Disk => "drive-harddisk-symbolic",
            Metric::Net => "network-transmit-receive-symbolic",
        }
    }

    /// `(warn, danger)` thresholds; `None` = never tinted (rates).
    fn thresholds(self) -> Option<(f32, f32)> {
        match self {
            Metric::Cpu => Some((70.0, 90.0)),
            Metric::Temp => Some((80.0, 90.0)),
            Metric::Gpu => Some((70.0, 90.0)),
            Metric::Ram => Some((80.0, 95.0)),
            Metric::Swap => Some((50.0, 90.0)),
            Metric::Disk => Some((85.0, 95.0)),
            Metric::Net => None,
        }
    }
}

/// The latest value of every metric; `None` = no sensor / no data.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Readings {
    pub(crate) cpu: Option<f32>,
    pub(crate) temp: Option<f32>,
    pub(crate) gpu: Option<f32>,
    pub(crate) ram: Option<f32>,
    /// `None` when the machine has no swap.
    pub(crate) swap: Option<f32>,
    pub(crate) disk: Option<f32>,
    /// Bytes/s `(down, up)` over the physical interfaces.
    pub(crate) net: Option<(u64, u64)>,
}

impl Readings {
    fn value(&self, m: Metric) -> Option<f32> {
        match m {
            Metric::Cpu => self.cpu,
            Metric::Temp => self.temp,
            Metric::Gpu => self.gpu,
            Metric::Ram => self.ram,
            Metric::Swap => self.swap,
            Metric::Disk => self.disk,
            Metric::Net => self.net.map(|_| 0.0),
        }
    }
}

/// The metrics the config turns on, in display order. `ram_override` is
/// the session-only right-click toggle (`None` = follow the config).
pub(crate) fn enabled(cfg: &CpuDashboardBarWidget, ram_override: Option<bool>) -> Vec<Metric> {
    Metric::ALL
        .into_iter()
        .filter(|m| match m {
            Metric::Cpu => cfg.show_cpu,
            Metric::Temp => cfg.show_temperature,
            Metric::Gpu => cfg.show_gpu,
            Metric::Ram => ram_override.unwrap_or(cfg.show_ram),
            Metric::Swap => cfg.show_swap,
            Metric::Disk => cfg.show_disk,
            Metric::Net => cfg.show_network,
        })
        .collect()
}

/// `calm` / `warn` / `danger` for a reading (unavailable is always calm).
pub(crate) fn severity(m: Metric, value: Option<f32>) -> &'static str {
    match (m.thresholds(), value) {
        (Some((_, danger)), Some(v)) if v >= danger => "danger",
        (Some((warn, _)), Some(v)) if v >= warn => "warn",
        _ => "calm",
    }
}

/// The capsule text for one metric; `None` when unavailable.
pub(crate) fn text(m: Metric, r: &Readings) -> Option<String> {
    let pct = |v: f32| format!("{}%", v.round() as i64);
    match m {
        Metric::Temp => r.temp.map(|t| format!("{}°C", t.round() as i64)),
        Metric::Net => r.net.map(|(down, up)| {
            format!(
                "{}↓ {}↑",
                mshell_utils::units::rate_compact(down),
                mshell_utils::units::rate_compact(up)
            )
        }),
        _ => r.value(m).map(pct),
    }
}

/// What one capsule cell shows: `None` = hide the cell (unavailable and
/// `hide_unavailable`); otherwise its text (an em dash when unavailable)
/// and its severity class (`calm` when not highlighting).
pub(crate) fn cell(
    m: Metric,
    r: &Readings,
    cfg: &CpuDashboardBarWidget,
) -> Option<(String, &'static str)> {
    match text(m, r) {
        Some(t) => {
            let class = if cfg.highlight {
                severity(m, r.value(m))
            } else {
                "calm"
            };
            Some((t, class))
        }
        None if cfg.hide_unavailable => None,
        None => Some(("—".to_string(), "calm")),
    }
}

/// Index of the filesystem holding `path`: the longest mount point that is
/// a prefix of it (`/home/me` → `/home` over `/`).
pub(crate) fn disk_for_path(mounts: &[&Path], path: &Path) -> Option<usize> {
    mounts
        .iter()
        .enumerate()
        .filter(|(_, m)| path.starts_with(m))
        .max_by_key(|(_, m)| m.components().count())
        .map(|(i, _)| i)
}

/// Total `(down, up)` bytes/s over physical interfaces, each byte once
/// (tunnels / bridges / loopback ride on a physical link already).
pub(crate) fn physical_rates(
    ifaces: &[(String, u64, u64)],
    is_virtual: impl Fn(&str) -> bool,
) -> Option<(u64, u64)> {
    let phys: Vec<_> = ifaces.iter().filter(|(n, _, _)| !is_virtual(n)).collect();
    if phys.is_empty() {
        return None;
    }
    Some(
        phys.iter()
            .fold((0, 0), |(d, u), (_, rx, tx)| (d + rx, u + tx)),
    )
}

/// `/proc/loadavg` → the 1 / 5 / 15 minute load averages.
pub(crate) fn parse_loadavg(s: &str) -> Option<(f32, f32, f32)> {
    let mut it = s.split_whitespace().map(|v| v.parse::<f32>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

/// Extra hover-only details, sampled while the pointer is on the pill.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Details {
    pub(crate) freq_mhz: Option<(u64, u64)>,
    pub(crate) load: Option<(f32, f32, f32)>,
    pub(crate) ram_bytes: Option<(u64, u64)>,
    pub(crate) swap_bytes: Option<(u64, u64)>,
    pub(crate) disk_bytes: Option<(u64, u64)>,
    pub(crate) disk_mount: Option<String>,
    /// `(name, % of one CPU)`.
    pub(crate) top_cpu: Option<(String, f32)>,
    /// `(name, RSS KiB)`.
    pub(crate) top_ram: Option<(String, u64)>,
}

/// The two-column hover table: `(label, value)` rows for the shown
/// metrics plus CPU frequency / load and the top processes.
pub(crate) fn tooltip_rows(shown: &[Metric], r: &Readings, d: &Details) -> Vec<(String, String)> {
    let dash = || "—".to_string();
    let used = |b: Option<(u64, u64)>, pct: Option<f32>| match (b, pct) {
        (Some((used, total)), Some(p)) => format!(
            "{}% · {} / {}",
            p.round() as i64,
            mshell_utils::units::bytes(used),
            mshell_utils::units::bytes(total)
        ),
        (None, Some(p)) => format!("{}%", p.round() as i64),
        _ => dash(),
    };
    let mut rows = Vec::new();
    for m in shown {
        let row = match m {
            Metric::Cpu => ("CPU", text(*m, r).unwrap_or_else(dash)),
            Metric::Temp => ("Temperature", text(*m, r).unwrap_or_else(dash)),
            Metric::Gpu => ("iGPU busy", text(*m, r).unwrap_or_else(dash)),
            Metric::Ram => ("Memory", used(d.ram_bytes, r.ram)),
            Metric::Swap => ("Swap", used(d.swap_bytes, r.swap)),
            Metric::Disk => (
                "Disk",
                match &d.disk_mount {
                    Some(mount) => format!("{} ({mount})", used(d.disk_bytes, r.disk)),
                    None => used(d.disk_bytes, r.disk),
                },
            ),
            Metric::Net => (
                "Network",
                r.net
                    .map(|(down, up)| {
                        format!(
                            "↓ {}  ↑ {}",
                            mshell_utils::units::rate(down as f64),
                            mshell_utils::units::rate(up as f64)
                        )
                    })
                    .unwrap_or_else(dash),
            ),
        };
        rows.push((row.0.to_string(), row.1));
    }
    if let Some((avg, max)) = d.freq_mhz {
        rows.push((
            "Frequency".to_string(),
            format!(
                "{:.1} / {:.1} GHz",
                avg as f64 / 1000.0,
                max as f64 / 1000.0
            ),
        ));
    }
    if let Some((one, five, fifteen)) = d.load {
        rows.push((
            "Load".to_string(),
            format!("{one:.2}  {five:.2}  {fifteen:.2}"),
        ));
    }
    if let Some((name, cpu)) = &d.top_cpu {
        rows.push(("Top CPU".to_string(), format!("{name} · {cpu:.0}%")));
    }
    if let Some((name, kib)) = &d.top_ram {
        rows.push((
            "Top memory".to_string(),
            format!("{name} · {}", mshell_utils::units::kib(*kib)),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cfg() -> CpuDashboardBarWidget {
        CpuDashboardBarWidget::default()
    }

    #[test]
    fn the_default_capsule_is_todays_cpu_and_temperature() {
        assert_eq!(enabled(&cfg(), None), [Metric::Cpu, Metric::Temp]);
        assert_eq!(
            enabled(&cfg(), Some(true)),
            [Metric::Cpu, Metric::Temp, Metric::Ram],
            "right-click still adds RAM for the session"
        );
        let all = CpuDashboardBarWidget {
            show_gpu: true,
            show_ram: true,
            show_swap: true,
            show_disk: true,
            show_network: true,
            ..cfg()
        };
        assert_eq!(enabled(&all, None), Metric::ALL);
        assert_eq!(enabled(&all, Some(false)).len(), 6);
    }

    #[test]
    fn each_metric_climbs_its_own_ladder() {
        assert_eq!(severity(Metric::Cpu, Some(69.0)), "calm");
        assert_eq!(severity(Metric::Cpu, Some(70.0)), "warn");
        assert_eq!(severity(Metric::Cpu, Some(95.0)), "danger");
        assert_eq!(severity(Metric::Temp, Some(85.0)), "warn");
        assert_eq!(severity(Metric::Ram, Some(85.0)), "warn");
        assert_eq!(severity(Metric::Disk, Some(85.0)), "warn");
        assert_eq!(severity(Metric::Swap, Some(60.0)), "warn");
        assert_eq!(
            severity(Metric::Net, Some(1e9)),
            "calm",
            "rates never alarm"
        );
        assert_eq!(
            severity(Metric::Cpu, None),
            "calm",
            "unavailable never tints"
        );
    }

    #[test]
    fn unavailable_is_an_em_dash_or_hidden_never_zero() {
        let r = Readings {
            cpu: Some(12.4),
            temp: None,
            ..Readings::default()
        };
        assert_eq!(
            cell(Metric::Cpu, &r, &cfg()),
            Some(("12%".to_string(), "calm"))
        );
        assert_eq!(
            cell(Metric::Temp, &r, &cfg()),
            Some(("—".to_string(), "calm"))
        );
        let hide = CpuDashboardBarWidget {
            hide_unavailable: true,
            ..cfg()
        };
        assert_eq!(cell(Metric::Temp, &r, &hide), None);
        assert_eq!(cell(Metric::Swap, &r, &hide), None, "no swap device");
    }

    #[test]
    fn highlighting_can_be_turned_off() {
        let r = Readings {
            cpu: Some(99.0),
            ..Readings::default()
        };
        assert_eq!(cell(Metric::Cpu, &r, &cfg()).unwrap().1, "danger");
        let plain = CpuDashboardBarWidget {
            highlight: false,
            ..cfg()
        };
        assert_eq!(cell(Metric::Cpu, &r, &plain).unwrap().1, "calm");
    }

    #[test]
    fn readouts_use_the_shared_units() {
        let r = Readings {
            temp: Some(61.6),
            net: Some((4_200, 1_100_000)),
            ..Readings::default()
        };
        assert_eq!(text(Metric::Temp, &r).as_deref(), Some("62°C"));
        let net = text(Metric::Net, &r).unwrap();
        assert!(net.contains('↓') && net.contains('↑'), "{net}");
        assert_eq!(
            net,
            format!(
                "{}↓ {}↑",
                mshell_utils::units::rate_compact(4_200),
                mshell_utils::units::rate_compact(1_100_000)
            )
        );
    }

    #[test]
    fn the_disk_is_the_deepest_mount_holding_the_path() {
        let root = PathBuf::from("/");
        let home = PathBuf::from("/home");
        let boot = PathBuf::from("/boot");
        let mounts = [root.as_path(), home.as_path(), boot.as_path()];
        assert_eq!(disk_for_path(&mounts, Path::new("/home/kenan")), Some(1));
        assert_eq!(disk_for_path(&mounts, Path::new("/")), Some(0));
        assert_eq!(
            disk_for_path(&mounts, Path::new("/homework")),
            Some(0),
            "not /home"
        );
        assert_eq!(disk_for_path(&[home.as_path()], Path::new("/var")), None);
    }

    #[test]
    fn network_totals_skip_virtual_links_and_lo() {
        let ifaces = vec![
            ("lo".to_string(), 999, 999),
            ("wlan0".to_string(), 1000, 200),
            ("wg0".to_string(), 900, 150),
            ("enp0s31f6".to_string(), 10, 5),
        ];
        let virt = |n: &str| matches!(n, "lo" | "wg0");
        assert_eq!(physical_rates(&ifaces, virt), Some((1010, 205)));
        assert_eq!(physical_rates(&ifaces[..1], virt), None, "no physical link");
    }

    #[test]
    fn loadavg_parses_real_proc_output() {
        // Captured from /proc/loadavg on the owner's laptop.
        assert_eq!(
            parse_loadavg("1.18 0.97 0.88 2/1534 812345\n"),
            Some((1.18, 0.97, 0.88))
        );
        assert_eq!(parse_loadavg("garbage"), None);
    }

    #[test]
    fn the_hover_table_follows_the_capsule_then_adds_details() {
        let r = Readings {
            cpu: Some(7.0),
            ram: Some(50.0),
            ..Readings::default()
        };
        let d = Details {
            freq_mhz: Some((1400, 4800)),
            load: Some((1.0, 0.5, 0.25)),
            ram_bytes: Some((8 << 30, 16 << 30)),
            top_cpu: Some(("firefox".to_string(), 23.4)),
            top_ram: Some(("firefox".to_string(), 2 << 20)),
            ..Details::default()
        };
        let rows = tooltip_rows(&[Metric::Cpu, Metric::Ram, Metric::Temp], &r, &d);
        let labels: Vec<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            [
                "CPU",
                "Memory",
                "Temperature",
                "Frequency",
                "Load",
                "Top CPU",
                "Top memory"
            ]
        );
        assert_eq!(rows[1].1, "50% · 8.0 GiB / 16.0 GiB");
        assert_eq!(rows[2].1, "—");
        assert_eq!(rows[3].1, "1.4 / 4.8 GHz");
        assert_eq!(rows[5].1, "firefox · 23%");
        assert_eq!(rows[6].1, "firefox · 2.0 GiB");
    }
}
