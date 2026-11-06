//! CPU Dashboard bar pill — the system-monitor capsule.
//!
//! One chip with up to seven metrics, in this order: CPU %, CPU
//! temperature, Intel iGPU busy %, RAM %, swap %, disk use of a path and
//! network down / up (`bars.widgets.cpu_dashboard`; the default is the
//! classic CPU % + temperature). Each metric carries its own calm / warn /
//! danger class (DESIGN.md §2) on its icon + value group — a diverging
//! choice from the single cluster tint of §4.4, so the hot metric is the
//! one that lights up. A sensor without a reading shows an em dash (or is
//! dropped with `hide_unavailable`), never a misleading 0.
//!
//! Click opens the standalone `MenuType::CpuDashboard` menu. Right-click
//! toggles RAM in the capsule for the session (ephemeral, in memory).
//!
//! **Data.** CPU, memory, disks and network come from the shared
//! `sys_info_service()` (`wayle-sysinfo`) watch streams — one poll pass
//! feeds every consumer. Temperature falls back to margo's hwmon discovery
//! (`sysstat`) when the service has none. The iGPU sampler (the CPU
//! dashboard's sysfs reader) runs only while that metric is on. The hover
//! table's extras (frequency, load, top processes) are sampled only while
//! the pointer is on the pill.

mod metrics;

use crate::bars::bar_widgets::sysstat::{find_cpu_temp_sensor_pub, read_temp_millideg_pub};
use crate::menus::menu_widgets::cpu_dashboard::{gpu, procs};
use futures::StreamExt;
use metrics::{Details, Metric, Readings};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, CpuDashboardBarWidget,
};
use mshell_services::sys_info_service;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, GridExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// iGPU and hover-detail sampling cadence.
const SAMPLE_EVERY: Duration = Duration::from_secs(2);

/// The per-metric severity classes a cell can carry.
const SEVERITY: &[&str] = &["calm", "warn", "danger"];

pub(crate) struct CpuDashboardModel {
    cfg: CpuDashboardBarWidget,
    readings: Readings,
    details: Details,
    /// Right-click RAM toggle for this session (`None` = follow config).
    ram_override: Option<bool>,
    /// hwmon fallback when the service has no CPU temperature.
    sensor_path: Option<PathBuf>,
    cluster: gtk::Box,
    cells: HashMap<Metric, (gtk::Box, gtk::Label)>,
    tooltip_grid: gtk::Grid,
    /// Tells the iGPU sampler whether to run.
    gpu_on: watch::Sender<bool>,
    /// Tells the hover sampler whether the pointer is on the pill (and the
    /// detailed table is on).
    hovering: watch::Sender<bool>,
    /// Pointer on the pill: the hover table is only rebuilt while it is.
    pointer_in: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardInput {
    Clicked,
    ToggleRamInBar,
    Config(CpuDashboardBarWidget),
    Hover(bool),
}

#[derive(Debug)]
pub(crate) enum CpuDashboardOutput {
    Clicked,
}

pub(crate) struct CpuDashboardInit {
    pub(crate) orientation: Orientation,
}

/// Pushed in from the watch streams and samplers (primitives only, so the
/// pill needn't know `wayle-sysinfo` types).
#[derive(Debug)]
pub(crate) enum CpuDashboardCmd {
    Cpu {
        percent: f32,
        temp: Option<f32>,
        freq: (u64, u64),
    },
    Mem {
        ram: f32,
        ram_bytes: (u64, u64),
        swap: Option<(f32, u64, u64)>,
    },
    Disks(Vec<(PathBuf, u64, u64, f32)>),
    Net(Option<(u64, u64)>),
    Gpu(Option<f32>),
    Hover {
        load: Option<(f32, f32, f32)>,
        top_cpu: Option<(String, f32)>,
        top_ram: Option<(String, u64)>,
    },
}

fn pill_config() -> CpuDashboardBarWidget {
    config_manager()
        .config()
        .bars()
        .widgets()
        .cpu_dashboard()
        .get_untracked()
}

#[relm4::component(pub)]
impl Component for CpuDashboardModel {
    type CommandOutput = CpuDashboardCmd;
    type Input = CpuDashboardInput;
    type Output = CpuDashboardOutput;
    type Init = CpuDashboardInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &[
                "ok-button-surface",
                "ok-bar-widget",
                "cpu-dashboard-bar-widget",
            ],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(CpuDashboardInput::Clicked);
                },

                // One tight icon + value group per metric; the icons are
                // the separators. Each group carries its own severity.
                #[local_ref]
                cluster_widget -> gtk::Box {
                    add_css_class: "cpu-dashboard-bar-cluster",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 9,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = params.orientation;
        let cfg = pill_config();
        let svc = sys_info_service();
        let cpu0 = svc.cpu.get();
        let mem0 = svc.memory.get();
        let sensor_path = find_cpu_temp_sensor_pub();
        let temp0 = cpu_temp(cpu0.temperature_celsius, sensor_path.as_deref());

        let cluster_widget = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let tooltip_grid = gtk::Grid::new();
        tooltip_grid.add_css_class("cpu-dashboard-tooltip");
        tooltip_grid.set_column_spacing(16);
        tooltip_grid.set_row_spacing(2);

        let (gpu_on, gpu_rx) = watch::channel(cfg.show_gpu);
        let (hovering, hover_rx) = watch::channel(false);
        spawn_streams(&sender);
        spawn_gpu_sampler(&sender, gpu_rx);
        spawn_hover_sampler(&sender, hover_rx);

        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            let cfg = config_manager()
                .config()
                .bars()
                .widgets()
                .cpu_dashboard()
                .get();
            s.input(CpuDashboardInput::Config(cfg));
        });

        let mut model = CpuDashboardModel {
            cfg,
            readings: Readings {
                cpu: Some(cpu0.usage_percent),
                temp: temp0,
                ram: Some(mem0.usage_percent),
                swap: (mem0.swap_total_bytes > 0).then_some(mem0.swap_percent),
                ..Readings::default()
            },
            details: Details {
                freq_mhz: Some((cpu0.avg_frequency_mhz, cpu0.max_frequency_mhz)),
                ram_bytes: Some((mem0.used_bytes, mem0.total_bytes)),
                swap_bytes: (mem0.swap_total_bytes > 0)
                    .then_some((mem0.swap_used_bytes, mem0.swap_total_bytes)),
                ..Details::default()
            },
            ram_override: None,
            sensor_path,
            cluster: cluster_widget.clone(),
            cells: HashMap::new(),
            tooltip_grid,
            gpu_on,
            hovering,
            pointer_in: false,
            _effects: effects,
        };

        let widgets = view_output!();

        // Right-click toggles RAM in the capsule for this session.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let s = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| s.input(CpuDashboardInput::ToggleRamInBar));
        widgets.button.add_controller(gesture);

        // Hover drives the detail sampler (nothing is read when nobody looks).
        let motion = gtk::EventControllerMotion::new();
        let s = sender.clone();
        motion.connect_enter(move |_, _, _| s.input(CpuDashboardInput::Hover(true)));
        let s = sender.clone();
        motion.connect_leave(move |_| s.input(CpuDashboardInput::Hover(false)));
        root.add_controller(motion);

        let grid = model.tooltip_grid.clone();
        root.connect_query_tooltip(move |_, _, _, _, tooltip| {
            tooltip.set_custom(Some(&grid));
            true
        });

        rebuild_cells(&mut model);
        fill_tooltip(&model, &shown(&model));
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CpuDashboardInput::Clicked => {
                let _ = sender.output(CpuDashboardOutput::Clicked);
            }
            CpuDashboardInput::ToggleRamInBar => {
                let shown = self.ram_override.unwrap_or(self.cfg.show_ram);
                self.ram_override = Some(!shown);
                rebuild_cells(self);
            }
            CpuDashboardInput::Config(cfg) => {
                self.gpu_on.send_replace(cfg.show_gpu);
                self.hovering
                    .send_replace(self.pointer_in && cfg.detailed_tooltip);
                if !cfg.show_gpu {
                    self.readings.gpu = None;
                }
                self.cfg = cfg;
                rebuild_cells(self);
            }
            CpuDashboardInput::Hover(on) => {
                self.pointer_in = on;
                self.hovering.send_replace(on && self.cfg.detailed_tooltip);
                if on {
                    fill_tooltip(self, &shown(self));
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CpuDashboardCmd::Cpu {
                percent,
                temp,
                freq,
            } => {
                self.readings.cpu = Some(percent);
                self.readings.temp = cpu_temp(temp, self.sensor_path.as_deref());
                self.details.freq_mhz = (freq.0 > 0).then_some(freq);
            }
            CpuDashboardCmd::Mem {
                ram,
                ram_bytes,
                swap,
            } => {
                self.readings.ram = Some(ram);
                self.details.ram_bytes = Some(ram_bytes);
                self.readings.swap = swap.map(|(p, _, _)| p);
                self.details.swap_bytes = swap.map(|(_, used, total)| (used, total));
            }
            CpuDashboardCmd::Disks(disks) => {
                let mounts: Vec<&Path> = disks.iter().map(|(m, ..)| m.as_path()).collect();
                let pick = metrics::disk_for_path(&mounts, Path::new(self.cfg.disk_path.trim()));
                let disk = pick.map(|i| &disks[i]);
                self.readings.disk = disk.map(|(_, _, _, pct)| *pct);
                self.details.disk_bytes = disk.map(|(_, used, total, _)| (*used, *total));
                self.details.disk_mount = disk.map(|(m, ..)| m.display().to_string());
            }
            CpuDashboardCmd::Net(rates) => self.readings.net = rates,
            CpuDashboardCmd::Gpu(busy) => self.readings.gpu = busy,
            CpuDashboardCmd::Hover {
                load,
                top_cpu,
                top_ram,
            } => {
                self.details.load = load;
                self.details.top_cpu = top_cpu;
                self.details.top_ram = top_ram;
            }
        }
        refresh_cells(self);
    }
}

/// Service temperature, else the hwmon fallback; `None` without a sensor.
fn cpu_temp(service: Option<f32>, sensor: Option<&Path>) -> Option<f32> {
    service.filter(|t| *t > 0.0).or_else(|| {
        sensor
            .and_then(|p| read_temp_millideg_pub(&p.to_path_buf()))
            .map(|m| m as f32 / 1000.0)
    })
}

fn shown(model: &CpuDashboardModel) -> Vec<Metric> {
    metrics::enabled(&model.cfg, model.ram_override)
}

/// Recreate one icon + value group per enabled metric.
fn rebuild_cells(model: &mut CpuDashboardModel) {
    while let Some(child) = model.cluster.first_child() {
        model.cluster.remove(&child);
    }
    model.cells.clear();
    for m in shown(model) {
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        group.add_css_class("cpu-dashboard-bar-metric");
        let icon = gtk::Image::from_icon_name(m.icon());
        icon.add_css_class("cpu-dashboard-bar-icon");
        let label = gtk::Label::new(None);
        label.add_css_class("cpu-dashboard-bar-label");
        group.append(&icon);
        group.append(&label);
        model.cluster.append(&group);
        model.cells.insert(m, (group, label));
    }
    refresh_cells(model);
}

/// Repaint every cell and the hover table from the latest readings.
fn refresh_cells(model: &CpuDashboardModel) {
    let shown = shown(model);
    for m in &shown {
        let Some((group, label)) = model.cells.get(m) else {
            continue;
        };
        match metrics::cell(*m, &model.readings, &model.cfg) {
            Some((text, class)) => {
                group.set_visible(true);
                label.set_label(&text);
                for c in SEVERITY {
                    if *c == class {
                        group.add_css_class(c);
                    } else {
                        group.remove_css_class(c);
                    }
                }
            }
            None => group.set_visible(false),
        }
    }
    // The table is rebuilt only while someone is looking at it (and once on
    // hover entry), not on every poll tick.
    if model.pointer_in {
        fill_tooltip(model, &shown);
    }
}

fn fill_tooltip(model: &CpuDashboardModel, shown: &[Metric]) {
    let grid = &model.tooltip_grid;
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
    let mut rows = if model.cfg.detailed_tooltip {
        metrics::tooltip_rows(shown, &model.readings, &model.details)
    } else {
        metrics::tooltip_rows(shown, &model.readings, &Details::default())
    };
    rows.push(("Click".to_string(), "Open the dashboard".to_string()));
    rows.push(("Right-click".to_string(), "Show or hide RAM".to_string()));
    for (i, (key, value)) in rows.iter().enumerate() {
        let k = gtk::Label::new(Some(key));
        k.add_css_class("cpu-dashboard-tooltip-key");
        k.set_halign(gtk::Align::Start);
        let v = gtk::Label::new(Some(value));
        v.add_css_class("cpu-dashboard-tooltip-value");
        v.set_halign(gtk::Align::Start);
        grid.attach(&k, 0, i as i32, 1, 1);
        grid.attach(&v, 1, i as i32, 1, 1);
    }
}

/// Subscribe to the shared sysinfo service (current value first, then on
/// every poll).
fn spawn_streams(sender: &ComponentSender<CpuDashboardModel>) {
    sender.command(|out, shutdown| async move {
        let mut cpu = sys_info_service().cpu.watch();
        let mut mem = sys_info_service().memory.watch();
        let mut disks = sys_info_service().disks.watch();
        let mut net = sys_info_service().network.watch();
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            let msg = tokio::select! {
                () = &mut shutdown_fut => break,
                Some(d) = cpu.next() => CpuDashboardCmd::Cpu {
                    percent: d.usage_percent,
                    temp: d.temperature_celsius,
                    freq: (d.avg_frequency_mhz, d.max_frequency_mhz),
                },
                Some(d) = mem.next() => CpuDashboardCmd::Mem {
                    ram: d.usage_percent,
                    ram_bytes: (d.used_bytes, d.total_bytes),
                    swap: (d.swap_total_bytes > 0)
                        .then_some((d.swap_percent, d.swap_used_bytes, d.swap_total_bytes)),
                },
                Some(d) = disks.next() => CpuDashboardCmd::Disks(
                    d.iter()
                        .map(|x| (x.mount_point.clone(), x.used_bytes, x.total_bytes, x.usage_percent))
                        .collect(),
                ),
                Some(d) = net.next() => {
                    let ifaces: Vec<(String, u64, u64)> = d
                        .iter()
                        .map(|n| (n.interface.clone(), n.rx_bytes_per_sec, n.tx_bytes_per_sec))
                        .collect();
                    CpuDashboardCmd::Net(metrics::physical_rates(
                        &ifaces,
                        mshell_utils::netstat::is_virtual,
                    ))
                }
                else => break,
            };
            if out.send(msg).is_err() {
                break;
            }
        }
    });
}

/// Sample the Intel iGPU's busy % (max over its GTs) every
/// [`SAMPLE_EVERY`] while the metric is on; idle otherwise.
fn spawn_gpu_sampler(sender: &ComponentSender<CpuDashboardModel>, mut on: watch::Receiver<bool>) {
    sender.command(|out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut prev: Option<(Instant, Vec<u64>)> = None;
        loop {
            let enabled = *on.borrow_and_update();
            if !enabled {
                prev = None;
            }
            let tick = enabled.then_some(SAMPLE_EVERY);
            tokio::select! {
                () = &mut shutdown_fut => break,
                changed = on.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    continue;
                }
                () = sleep_or_forever(tick) => {}
            }
            let sample = tokio::task::spawn_blocking(|| {
                let igpu = gpu::find_igpu(Path::new("/sys/class/drm"))?;
                Some(
                    igpu.gts
                        .iter()
                        .filter_map(gpu::sample_gt)
                        .map(|s| s.idle_ms)
                        .collect::<Vec<_>>(),
                )
            })
            .await
            .ok()
            .flatten();
            let now = Instant::now();
            let busy = match (&prev, &sample) {
                (Some((t0, idle0)), Some(idle1)) if idle0.len() == idle1.len() => {
                    let wall = now.duration_since(*t0).as_millis() as u64;
                    idle0
                        .iter()
                        .zip(idle1)
                        .map(|(a, b)| gpu::busy_pct(*a, *b, wall))
                        .reduce(f32::max)
                }
                _ => None,
            };
            prev = sample.map(|s| (now, s));
            // First sample has no baseline; only "no iGPU" reports None.
            if busy.is_some() || prev.is_none() {
                let _ = out.send(CpuDashboardCmd::Gpu(busy));
            }
        }
    });
}

/// While the pointer is on the pill (and the detailed table is on), read
/// the load average and the top CPU / memory processes every
/// [`SAMPLE_EVERY`].
fn spawn_hover_sampler(
    sender: &ComponentSender<CpuDashboardModel>,
    mut hovering: watch::Receiver<bool>,
) {
    sender.command(|out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut tracker = procs::ProcTracker::default();
        let mut last = Instant::now();
        loop {
            let on = *hovering.borrow_and_update();
            let tick = on.then_some(SAMPLE_EVERY);
            tokio::select! {
                () = &mut shutdown_fut => break,
                changed = hovering.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    if !*hovering.borrow() {
                        continue;
                    }
                }
                () = sleep_or_forever(tick) => {}
            }
            let snap =
                tokio::task::spawn_blocking(|| procs::read_snapshot(Path::new("/proc"))).await;
            let Ok(snap) = snap else { continue };
            let now = Instant::now();
            let rows = tracker.update(&snap, now.duration_since(last).as_secs_f64());
            last = now;
            let top_cpu = rows
                .iter()
                .filter(|r| r.cpu > 0.0)
                .max_by(|a, b| a.cpu.total_cmp(&b.cpu))
                .map(|r| (r.name.clone(), r.cpu));
            let top_ram = rows
                .iter()
                .max_by_key(|r| r.rss_kb)
                .map(|r| (r.name.clone(), r.rss_kb));
            let load = std::fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|s| metrics::parse_loadavg(&s));
            let _ = out.send(CpuDashboardCmd::Hover {
                load,
                top_cpu,
                top_ram,
            });
        }
    });
}

async fn sleep_or_forever(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}
