//! CPU Dashboard menu widget — the right-pane content for
//! `MenuType::CpuDashboard`. Mirrors the popover content that
//! used to live inside the `cpu_dashboard` bar pill, ported into
//! the layer-shell menu surface so it opens contiguous with the
//! bar like ufw/ndns instead of as a standalone xdg_popup.
//!
//! Renders four sections, all driven by a 2 s self-cancelling
//! poll over `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, and
//! the CPU package temperature sensor discovered on first poll:
//!   - Hero card: aggregate CPU% + package temp
//!   - Per-core load bars
//!   - RAM bar + percent
//!   - Load average (1m / 5m / 15m)
//!
//! Severity classes (calm / warn / danger) come from the same
//! thresholds the bar pill uses so the menu's tint reads the same
//! signal at the same time.

use crate::bars::bar_widgets::sysstat::{
    find_cpu_temp_sensor_pub, read_cpu_stat_pub, read_temp_millideg_pub,
};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::path::PathBuf;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

// Severity thresholds kept in sync with the bar pill copy so a
// glance at the cluster and a glance at the menu read the same
// state ladder.
const CPU_WARN_PERCENT: u32 = 70;
const CPU_DANGER_PERCENT: u32 = 90;
const TEMP_WARN_CELSIUS: i32 = 80;
const TEMP_DANGER_CELSIUS: i32 = 90;

fn severity_class(cpu: u32, temp: i32) -> &'static str {
    let cpu_state = if cpu >= CPU_DANGER_PERCENT {
        2
    } else if cpu >= CPU_WARN_PERCENT {
        1
    } else {
        0
    };
    let temp_state = if temp >= TEMP_DANGER_CELSIUS {
        2
    } else if temp >= TEMP_WARN_CELSIUS {
        1
    } else {
        0
    };
    match cpu_state.max(temp_state) {
        2 => "danger",
        1 => "warn",
        _ => "calm",
    }
}

fn cpu_only_severity(cpu: u32) -> &'static str {
    if cpu >= CPU_DANGER_PERCENT {
        "danger"
    } else if cpu >= CPU_WARN_PERCENT {
        "warn"
    } else {
        "calm"
    }
}

#[derive(Default, Clone)]
struct CoreDeltas {
    prev_total: Vec<u64>,
    prev_idle: Vec<u64>,
    percent: Vec<u32>,
}

#[derive(Clone)]
struct CoreRow {
    container: gtk::Box,
    bar: gtk::ProgressBar,
    pct_label: gtk::Label,
}

pub(crate) struct CpuDashboardMenuWidgetModel {
    cpu_percent: u32,
    temp_celsius: i32,
    ram_percent: u32,
    load_1m: f32,
    load_5m: f32,
    load_15m: f32,
    prev_total: u64,
    prev_idle: u64,
    cores: CoreDeltas,
    sensor_path: Option<PathBuf>,
    core_rows: Vec<CoreRow>,
}

impl std::fmt::Debug for CpuDashboardMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuDashboardMenuWidgetModel")
            .field("cpu_percent", &self.cpu_percent)
            .field("temp_celsius", &self.temp_celsius)
            .field("ram_percent", &self.ram_percent)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum CpuDashboardMenuWidgetInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardMenuWidgetOutput {}

pub(crate) struct CpuDashboardMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for CpuDashboardMenuWidgetModel {
    type CommandOutput = ();
    type Input = CpuDashboardMenuWidgetInput;
    type Output = CpuDashboardMenuWidgetOutput;
    type Init = CpuDashboardMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "cpu-dashboard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 14,
            set_margin_all: 16,

            // Hero — aggregate CPU + temp.
            gtk::Box {
                #[watch]
                set_css_classes: &[
                    "cpu-dashboard-hero",
                    severity_class(model.cpu_percent, model.temp_celsius),
                ],
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 24,
                set_homogeneous: true,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-value",
                        #[watch]
                        set_label: &format!("{}%", model.cpu_percent),
                        set_halign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-caption",
                        set_label: "CPU LOAD",
                        set_halign: gtk::Align::Center,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-value",
                        #[watch]
                        set_label: &format!("{}°C", model.temp_celsius),
                        set_halign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-caption",
                        set_label: "PACKAGE TEMP",
                        set_halign: gtk::Align::Center,
                    },
                },
            },

            // Per-core load bars.
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "PER-CORE LOAD",
                set_halign: gtk::Align::Start,
            },
            #[name = "cores_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
            },

            // Memory.
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "MEMORY",
                set_halign: gtk::Align::Start,
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                gtk::ProgressBar {
                    set_hexpand: true,
                    #[watch]
                    set_fraction: (model.ram_percent as f64) / 100.0,
                    #[watch]
                    set_css_classes: &[
                        "cpu-dashboard-bar",
                        if model.ram_percent >= 90 { "danger" }
                        else if model.ram_percent >= 75 { "warn" }
                        else { "calm" },
                    ],
                },
                gtk::Label {
                    add_css_class: "cpu-dashboard-bar-value",
                    #[watch]
                    set_label: &format!("{}%", model.ram_percent),
                    set_width_chars: 4,
                },
            },

            // Load averages.
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "LOAD AVERAGE (1m · 5m · 15m)",
                set_halign: gtk::Align::Start,
            },
            gtk::Label {
                add_css_class: "cpu-dashboard-loadavg",
                #[watch]
                set_label: &format!(
                    "{:.2}    {:.2}    {:.2}",
                    model.load_1m, model.load_5m, model.load_15m,
                ),
                set_halign: gtk::Align::Start,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (prev_total, prev_idle) = read_cpu_stat_pub();
        let sensor_path = find_cpu_temp_sensor_pub();
        let temp_celsius = sensor_path
            .as_ref()
            .and_then(read_temp_millideg_pub)
            .map(|t| t / 1000)
            .unwrap_or(0);
        let ram_percent = read_ram_used_percent_local().unwrap_or(0);
        let (load_1m, load_5m, load_15m) =
            read_loadavg().unwrap_or((0.0, 0.0, 0.0));

        // Self-cancelling — Break when the receiver hangs up, so
        // the timer dies with the widget instead of running for
        // the rest of the shell session.
        let sender_clone = sender.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if sender_clone
                .input_sender()
                .send(CpuDashboardMenuWidgetInput::Poll)
                .is_err()
            {
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });

        let model = CpuDashboardMenuWidgetModel {
            cpu_percent: 0,
            temp_celsius,
            ram_percent,
            load_1m,
            load_5m,
            load_15m,
            prev_total,
            prev_idle,
            cores: CoreDeltas::default(),
            sensor_path,
            core_rows: Vec::new(),
        };

        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CpuDashboardMenuWidgetInput::Poll => {
                let (total, idle) = read_cpu_stat_pub();
                let delta_total = total.saturating_sub(self.prev_total);
                let delta_idle = idle.saturating_sub(self.prev_idle);
                if delta_total > 0 {
                    let busy = delta_total.saturating_sub(delta_idle);
                    self.cpu_percent = ((busy * 100) / delta_total) as u32;
                }
                self.prev_total = total;
                self.prev_idle = idle;

                let per_core = read_per_core_cpu_stat();
                if self.cores.prev_total.len() != per_core.len() {
                    self.cores.prev_total = per_core.iter().map(|(t, _)| *t).collect();
                    self.cores.prev_idle = per_core.iter().map(|(_, i)| *i).collect();
                    self.cores.percent = vec![0; per_core.len()];
                } else {
                    for (i, (t, idl)) in per_core.iter().enumerate() {
                        let dt = t.saturating_sub(self.cores.prev_total[i]);
                        let di = idl.saturating_sub(self.cores.prev_idle[i]);
                        if dt > 0 {
                            let busy = dt.saturating_sub(di);
                            self.cores.percent[i] = ((busy * 100) / dt) as u32;
                        }
                        self.cores.prev_total[i] = *t;
                        self.cores.prev_idle[i] = *idl;
                    }
                }

                while self.core_rows.len() < self.cores.percent.len() {
                    let i = self.core_rows.len();
                    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let lbl = gtk::Label::new(Some(&format!("c{i}")));
                    lbl.add_css_class("cpu-dashboard-core-label");
                    lbl.set_width_chars(3);
                    lbl.set_xalign(0.0);
                    let bar = gtk::ProgressBar::new();
                    bar.set_hexpand(true);
                    bar.add_css_class("cpu-dashboard-bar");
                    let pct_label = gtk::Label::new(Some("0%"));
                    pct_label.add_css_class("cpu-dashboard-bar-value");
                    pct_label.set_width_chars(4);
                    container.append(&lbl);
                    container.append(&bar);
                    container.append(&pct_label);
                    widgets.cores_box.append(&container);
                    self.core_rows.push(CoreRow { container, bar, pct_label });
                }
                while self.core_rows.len() > self.cores.percent.len() {
                    if let Some(row) = self.core_rows.pop() {
                        widgets.cores_box.remove(&row.container);
                    }
                }

                for (i, p) in self.cores.percent.iter().enumerate() {
                    if let Some(row) = self.core_rows.get(i) {
                        row.bar.set_fraction((*p as f64) / 100.0);
                        row.bar
                            .set_css_classes(&["cpu-dashboard-bar", cpu_only_severity(*p)]);
                        row.pct_label.set_label(&format!("{p}%"));
                    }
                }

                if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg_pub(p)
                {
                    self.temp_celsius = t / 1000;
                }

                self.ram_percent =
                    read_ram_used_percent_local().unwrap_or(self.ram_percent);
                if let Some((a, b, c)) = read_loadavg() {
                    self.load_1m = a;
                    self.load_5m = b;
                    self.load_15m = c;
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

fn read_per_core_cpu_stat() -> Vec<(u64, u64)> {
    let Ok(s) = std::fs::read_to_string("/proc/stat") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in s.lines() {
        if !line.starts_with("cpu") {
            break;
        }
        let head = line.split_whitespace().next().unwrap_or("");
        if head == "cpu" {
            continue;
        }
        if !head.chars().nth(3).is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let parts: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 4 {
            continue;
        }
        let total: u64 = parts.iter().sum();
        let idle = parts[3] + parts.get(4).copied().unwrap_or(0);
        out.push((total, idle));
    }
    out
}

fn read_ram_used_percent_local() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total: Option<u64> = None;
    let mut avail: Option<u64> = None;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = rest.split_whitespace().next()?.parse().ok();
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail = rest.split_whitespace().next()?.parse().ok();
        }
        if total.is_some() && avail.is_some() {
            break;
        }
    }
    let total = total?;
    let avail = avail?;
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(avail);
    Some(((used * 100) / total) as u32)
}

fn read_loadavg() -> Option<(f32, f32, f32)> {
    let s = std::fs::read_to_string("/proc/loadavg").ok()?;
    let mut it = s.split_whitespace();
    let a: f32 = it.next()?.parse().ok()?;
    let b: f32 = it.next()?.parse().ok()?;
    let c: f32 = it.next()?.parse().ok()?;
    Some((a, b, c))
}
