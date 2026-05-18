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

// Threshold ceilings for the three visual states. Tuned high
// enough that an idle desktop sits in "calm" — the previous
// 50/60 floor was tripping on a typical idle temp (65–70 °C on
// most laptops) and made the pill read as a permanent warn.
//
// Now:
//   warn  → real load you'd notice (~70 % CPU or hot package)
//   danger → sustained pegging (>90 % CPU or thermal throttle
//            territory)
const CPU_WARN_PERCENT: u32 = 70;
const CPU_DANGER_PERCENT: u32 = 90;
const TEMP_WARN_CELSIUS: i32 = 80;
const TEMP_DANGER_CELSIUS: i32 = 90;

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
    /// Whether the bar pill cluster also surfaces the RAM
    /// percentage. Off by default — the bar reads CPU% · Temp°C
    /// only; right-click flips this on so users who watch memory
    /// can opt in. Toggles via the pill's secondary-click
    /// gesture. Ephemeral (in-memory only).
    show_ram_in_bar: bool,
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
    Clicked,
    ToggleRamInBar,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardOutput {
    Clicked,
}

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

                // Single cluster carries the severity class so we
                // tint label + icon together while the outer pill
                // chrome (`ok-button-surface`) stays exactly like
                // npodman / nnetwork / ndns.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_css_classes: &[
                        "cpu-dashboard-bar-cluster",
                        severity_class(model.cpu_percent, model.temp_celsius),
                    ],

                    gtk::Image {
                        set_icon_name: Some("computer-symbolic"),
                        add_css_class: "cpu-dashboard-bar-icon",
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
                    // RAM slot — hidden by default. Wrap separator
                    // + label in one Box so they show/hide as a
                    // unit (don't want a dangling ` · ` glyph).
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        #[watch]
                        set_visible: model.show_ram_in_bar,

                        gtk::Label {
                            add_css_class: "cpu-dashboard-bar-sep",
                            set_label: "·",
                        },
                        gtk::Label {
                            add_css_class: "cpu-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}%", model.ram_percent),
                        },
                    },
                },
            },

            // Popover dropped — bar pill now emits Clicked and
            // the layer-shell menu (MenuType::CpuDashboard) renders
            // the rich content via cpu_dashboard_menu_widget.
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
        // Prime RAM up front so the popover shows a real value
        // the first time it opens instead of "0%" until the
        // 2 s poll fires.
        let ram_percent = read_ram_used_percent_local().unwrap_or(0);
        // Prime load avg too for the same reason.
        let (load_1m, load_5m, load_15m) =
            read_loadavg().unwrap_or((0.0, 0.0, 0.0));

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
            ram_percent,
            load_1m,
            load_5m,
            load_15m,
            prev_total,
            prev_idle,
            cores: CoreDeltas::default(),
            sensor_path,
            _orientation: params.orientation,
            show_ram_in_bar: false,
            core_rows: Vec::new(),
        };

        let widgets = view_output!();

        // Right-click toggles RAM% visibility in the bar cluster.
        // Default is hidden (CPU% · Temp°C); right-click adds the
        // RAM slot to the row, right-click again removes it.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(CpuDashboardInput::ToggleRamInBar);
        });
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
                // Aggregate CPU only — per-core / RAM bar / load
                // avg moved into the menu widget. The bar pill
                // surfaces just the chip metrics (CPU% · Temp · RAM%).
                let (total, idle) = read_cpu_stat_pub();
                let delta_total = total.saturating_sub(self.prev_total);
                let delta_idle = idle.saturating_sub(self.prev_idle);
                if delta_total > 0 {
                    let busy = delta_total.saturating_sub(delta_idle);
                    self.cpu_percent = ((busy * 100) / delta_total) as u32;
                }
                self.prev_total = total;
                self.prev_idle = idle;
                if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg_pub(p)
                {
                    self.temp_celsius = t / 1000;
                }
                self.ram_percent = read_ram_used_percent_local().unwrap_or(self.ram_percent);

                _root.set_tooltip_text(Some(&format!(
                    "CPU {}%  ·  Temp {}°C  ·  RAM {}%",
                    self.cpu_percent, self.temp_celsius, self.ram_percent,
                )));
            }
            CpuDashboardInput::Clicked => {
                let _ = sender.output(CpuDashboardOutput::Clicked);
            }
            CpuDashboardInput::ToggleRamInBar => {
                self.show_ram_in_bar = !self.show_ram_in_bar;
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
