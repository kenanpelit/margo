//! Combined CPU dashboard bar pill — single chip showing live CPU
//! load + package temperature with threshold-driven colour states,
//! plus a click-to-open Popover that surfaces per-core load bars,
//! memory pressure, and load averages.
//!
//! Replaces the standalone `CpuMonitor` + `CpuTemp` bar pills for
//! users who want the consolidated chip; the original pills stay
//! available for users who prefer them separate.
//!
//! Threshold semantics (the higher of the two values wins —
//! mirrors how the user actually feels load: an idle CPU running
//! hot is still "warm-looking", and a busy CPU at moderate temp
//! is still "busy-looking"):
//! - **calm**:   CPU < 50 % AND temp < 60 °C
//! - **warn**:   one of CPU 50–80 %, temp 60–80 °C
//! - **danger**: CPU ≥ 80 % OR temp ≥ 80 °C
//!
//! The state name becomes a CSS class on the root pill *and* on
//! the hero card inside the popover, so SCSS can retint
//! background / outline / label tone per state without the Rust
//! side knowing about colours. Per-core bars inside the popover
//! inherit the same calm/warn/danger ladder individually so a
//! single pinned thread shows up at a glance.

use crate::bars::bar_widgets::sysstat::{
    find_cpu_temp_sensor_pub, read_cpu_stat_pub, read_temp_millideg_pub,
};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureSingleExt, OrientableExt, PopoverExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::path::PathBuf;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

// Threshold ceilings for the three visual states. Tunable here
// only — the rest of the code reads them by name.
const CPU_WARN_PERCENT: u32 = 50;
const CPU_DANGER_PERCENT: u32 = 80;
const TEMP_WARN_CELSIUS: i32 = 60;
const TEMP_DANGER_CELSIUS: i32 = 80;

/// Per-core delta cache. Indexed by the core id reported in
/// `/proc/stat` (`cpu0`, `cpu1`, …). Resized as needed so we
/// stay correct on hot-plug / SMT toggles.
#[derive(Default, Clone)]
struct CoreDeltas {
    prev_total: Vec<u64>,
    prev_idle: Vec<u64>,
    /// Latest computed busy-% per core, length-aligned with the
    /// `prev_*` vectors after each poll.
    percent: Vec<u32>,
}

pub(crate) struct CpuDashboardModel {
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
    _orientation: Orientation,
    /// Per-core widgets, lazily grown to match the cpu count
    /// detected on first poll. Each entry owns the row's bar +
    /// percent label so we can refresh both without walking the
    /// widget tree.
    core_rows: Vec<CoreRow>,
}

#[derive(Clone)]
struct CoreRow {
    container: gtk::Box,
    bar: gtk::ProgressBar,
    pct_label: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardInput {
    Poll,
    ToggleMenu,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardOutput {}

pub(crate) struct CpuDashboardInit {
    pub(crate) orientation: Orientation,
}

/// Pick the calm/warn/danger CSS class from the two top-level
/// metrics. Public so the menu hero can reuse the same ladder.
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

/// Single-metric severity for per-core bars — they don't have a
/// temp reading of their own.
fn cpu_only_severity(cpu: u32) -> &'static str {
    if cpu >= CPU_DANGER_PERCENT {
        "danger"
    } else if cpu >= CPU_WARN_PERCENT {
        "warn"
    } else {
        "calm"
    }
}

#[relm4::component(pub)]
impl Component for CpuDashboardModel {
    type CommandOutput = ();
    type Input = CpuDashboardInput;
    type Output = CpuDashboardOutput;
    type Init = CpuDashboardInit;

    view! {
        #[root]
        gtk::Box {
            #[watch]
            set_css_classes: &[
                "cpu-dashboard-bar-widget",
                severity_class(model.cpu_percent, model.temp_celsius),
            ],
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                connect_clicked[sender] => move |_| {
                    sender.input(CpuDashboardInput::ToggleMenu);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,

                    gtk::Image {
                        set_icon_name: Some("computer-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-bar-label",
                        #[watch]
                        set_label: &format!("{}%", model.cpu_percent),
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-bar-sep",
                        set_label: "·",
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-bar-label",
                        #[watch]
                        set_label: &format!("{}°C", model.temp_celsius),
                    },
                },
            },

            // ── Popover menu surface ─────────────────────────
            //
            // Anchored on the bar button so it pops directly
            // below the pill. `autohide=true` closes it on any
            // click outside the popover area, matching the
            // mshell menu conventions.
            #[name = "popover"]
            gtk::Popover {
                set_position: gtk::PositionType::Bottom,
                set_has_arrow: false,
                set_autohide: true,
                add_css_class: "cpu-dashboard-menu",

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 14,
                    set_margin_all: 16,
                    set_width_request: 360,

                    // Hero card — big CPU% + temp side by side.
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
                        #[name = "ram_bar"]
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
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Prime the running CPU delta so the first tick produces
        // a meaningful percentage instead of "100 % since boot".
        let (prev_total, prev_idle) = read_cpu_stat_pub();
        let sensor_path = find_cpu_temp_sensor_pub();
        let temp_celsius = sensor_path
            .as_ref()
            .and_then(read_temp_millideg_pub)
            .map(|t| t / 1000)
            .unwrap_or(0);

        // Self-cancelling timer — see sysstat.rs schedule_poll
        // for the panic-on-closed-channel rationale.
        let sender_clone = sender.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if sender_clone
                .input_sender()
                .send(CpuDashboardInput::Poll)
                .is_err()
            {
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });

        let model = CpuDashboardModel {
            cpu_percent: 0,
            temp_celsius,
            ram_percent: 0,
            load_1m: 0.0,
            load_5m: 0.0,
            load_15m: 0.0,
            prev_total,
            prev_idle,
            cores: CoreDeltas::default(),
            sensor_path,
            _orientation: params.orientation,
            core_rows: Vec::new(),
        };

        let widgets = view_output!();

        // Right-click on the pill is reserved for future use
        // (e.g. cycle compact / verbose label modes). Wire an
        // empty SECONDARY gesture now so the click doesn't fall
        // through to the parent bar's drag-handle area.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        widgets.button.add_controller(gesture);

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
            CpuDashboardInput::Poll => {
                // ── Aggregate CPU ─────────────────────────────
                let (total, idle) = read_cpu_stat_pub();
                let delta_total = total.saturating_sub(self.prev_total);
                let delta_idle = idle.saturating_sub(self.prev_idle);
                if delta_total > 0 {
                    let busy = delta_total.saturating_sub(delta_idle);
                    self.cpu_percent = ((busy * 100) / delta_total) as u32;
                }
                self.prev_total = total;
                self.prev_idle = idle;

                // ── Per-core ──────────────────────────────────
                let per_core = read_per_core_cpu_stat();
                if self.cores.prev_total.len() != per_core.len() {
                    // Hot-plug / first poll — reset.
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

                // Grow / shrink the rendered bars to match the
                // current core count.
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

                // ── Temperature ───────────────────────────────
                if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg_pub(p)
                {
                    self.temp_celsius = t / 1000;
                }

                // ── Memory + load avg ─────────────────────────
                self.ram_percent = read_ram_used_percent_local().unwrap_or(self.ram_percent);
                if let Some((a, b, c)) = read_loadavg() {
                    self.load_1m = a;
                    self.load_5m = b;
                    self.load_15m = c;
                }
            }
            CpuDashboardInput::ToggleMenu => {
                if widgets.popover.is_visible() {
                    widgets.popover.popdown();
                } else {
                    widgets.popover.popup();
                    // Force a fresh sample so the popover never
                    // opens with stale text in the gap between
                    // the last tick and the click.
                    sender.input(CpuDashboardInput::Poll);
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Per-core analogue of `read_cpu_stat`. Returns `(total, idle)`
/// pairs in core-id order. Skips the aggregate `cpu` line —
/// callers use `read_cpu_stat_pub` for the aggregate.
fn read_per_core_cpu_stat() -> Vec<(u64, u64)> {
    let Ok(s) = std::fs::read_to_string("/proc/stat") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in s.lines() {
        // `cpu0 …`, `cpu1 …`, … — but not the aggregate `cpu …`.
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
