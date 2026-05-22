//! Combined CPU dashboard bar pill — single chip showing live CPU
//! load + package temperature with threshold-driven colour states.
//!
//! Click opens the standalone `MenuType::CpuDashboard` layer-shell
//! menu (per-core bars, RAM, load-avg). Right-click toggles RAM%
//! visibility in the bar cluster — off by default so the row reads
//! CPU% · Temp°C only.
//!
//! Threshold semantics (the higher of the two values wins —
//! mirrors how the user actually feels load: an idle CPU running
//! hot is still "warm-looking", and a busy CPU at moderate temp
//! is still "busy-looking"):
//! - **calm**:   CPU < 50 % AND temp < 60 °C
//! - **warn**:   one of CPU 50–80 %, temp 60–80 °C
//! - **danger**: CPU ≥ 80 % OR temp ≥ 80 °C

use crate::bars::bar_widgets::sysstat::{
    find_cpu_temp_sensor_pub, read_cpu_stat_pub, read_temp_millideg_pub,
};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

// Threshold ceilings for the three visual states. Tuned high
// enough that an idle desktop sits in "calm" — the previous
// 50/60 floor was tripping on a typical idle temp (65–70 °C on
// most laptops) and made the pill read as a permanent warn.
const CPU_WARN_PERCENT: u32 = 70;
const CPU_DANGER_PERCENT: u32 = 90;
const TEMP_WARN_CELSIUS: i32 = 80;
const TEMP_DANGER_CELSIUS: i32 = 90;

pub(crate) struct CpuDashboardModel {
    cpu_percent: u32,
    temp_celsius: i32,
    ram_percent: u32,
    prev_total: u64,
    prev_idle: u64,
    sensor_path: Option<PathBuf>,
    _orientation: Orientation,
    /// Whether the bar pill cluster also surfaces the RAM
    /// percentage. Off by default — the bar reads CPU% · Temp°C
    /// only; right-click flips this on so users who watch memory
    /// can opt in. Ephemeral (in-memory only).
    show_ram_in_bar: bool,
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
                // tint every metric's icon + label together while the
                // outer pill chrome (`ok-button-surface`) stays exactly
                // like podman / network / dns. Each metric is its own
                // tight icon+value group; the per-metric glyphs
                // (chip / thermometer / memory stick) replace the old
                // generic computer icon and the "·" text separators —
                // the icons are the separators now.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 9,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_css_classes: &[
                        "cpu-dashboard-bar-cluster",
                        severity_class(model.cpu_percent, model.temp_celsius),
                    ],

                    // CPU load
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        gtk::Image {
                            set_icon_name: Some("cpu-symbolic"),
                            add_css_class: "cpu-dashboard-bar-icon",
                        },
                        gtk::Label {
                            add_css_class: "cpu-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}%", model.cpu_percent),
                        },
                    },

                    // Package temperature
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        gtk::Image {
                            set_icon_name: Some("temperature-symbolic"),
                            add_css_class: "cpu-dashboard-bar-icon",
                        },
                        gtk::Label {
                            add_css_class: "cpu-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}°C", model.temp_celsius),
                        },
                    },

                    // RAM — hidden by default (right-click opt-in). The
                    // whole icon+value group shows/hides as a unit.
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        #[watch]
                        set_visible: model.show_ram_in_bar,

                        gtk::Image {
                            set_icon_name: Some("memory-symbolic"),
                            add_css_class: "cpu-dashboard-bar-icon",
                        },
                        gtk::Label {
                            add_css_class: "cpu-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}%", model.ram_percent),
                        },
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
        let ram_percent = read_ram_used_percent_local().unwrap_or(0);

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
            prev_total,
            prev_idle,
            sensor_path,
            _orientation: params.orientation,
            show_ram_in_bar: false,
        };

        let widgets = view_output!();

        // Right-click toggles RAM% visibility in the bar cluster.
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

                self.ram_percent =
                    read_ram_used_percent_local().unwrap_or(self.ram_percent);

                _root.set_tooltip_text(Some(&format!(
                    "CPU {}%  ·  Temp {}°C  ·  RAM {}%\nClick: open dashboard\nRight-click: toggle RAM in bar",
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
