//! Combined CPU dashboard bar pill — single chip showing live CPU
//! load + package temperature with threshold-driven colour states.
//!
//! Click opens the standalone `MenuType::CpuDashboard` layer-shell
//! menu (per-core bars, RAM, load-avg). Right-click toggles RAM%
//! visibility in the bar cluster — off by default so the row reads
//! CPU% · Temp°C only.
//!
//! **Data source.** CPU% and RAM% are read from the shared
//! `sys_info_service()` (`wayle-sysinfo`) reactive store rather than a
//! per-pill `/proc` poll loop: one poll pass in the service feeds this
//! pill, the dashboard menu, and any other consumer, so they never skew
//! and there's a single cadence. Temperature prefers the service's
//! `temperature_celsius`, falling back to margo's own robust hwmon
//! sensor discovery (`sysstat`) when the service can't supply one.
//!
//! Threshold semantics (the higher of the two values wins —
//! mirrors how the user actually feels load: an idle CPU running
//! hot is still "warm-looking", and a busy CPU at moderate temp
//! is still "busy-looking"):
//! - **calm**:   CPU < 50 % AND temp < 60 °C
//! - **warn**:   one of CPU 50–80 %, temp 60–80 °C
//! - **danger**: CPU ≥ 80 % OR temp ≥ 80 °C

use crate::bars::bar_widgets::sysstat::{find_cpu_temp_sensor_pub, read_temp_millideg_pub};
use futures::StreamExt;
use mshell_services::sys_info_service;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

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
    /// hwmon sensor path used only as a fallback when the service can't
    /// supply a CPU temperature (discovered once on init).
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

/// Reactive updates pushed in from the `sys_info_service()` watch
/// streams. Converted to primitives in the watcher so this crate
/// doesn't need the `wayle-sysinfo` types directly.
#[derive(Debug)]
pub(crate) enum CpuDashboardCmd {
    /// CPU usage % + the service's temperature (`None` → use the
    /// hwmon fallback).
    Cpu { percent: u32, temp: Option<i32> },
    /// RAM usage %.
    Mem { percent: u32 },
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
        let svc = sys_info_service();
        let cpu0 = svc.cpu.get();
        let mem0 = svc.memory.get();
        let sensor_path = find_cpu_temp_sensor_pub();
        let temp_celsius = cpu0
            .temperature_celsius
            .filter(|t| *t > 0.0)
            .map(|t| t.round() as i32)
            .or_else(|| {
                sensor_path
                    .as_ref()
                    .and_then(read_temp_millideg_pub)
                    .map(|t| t / 1000)
            })
            .unwrap_or(0);

        // Subscribe to the shared service. `watch()` yields the current
        // value first, then on every change — one poll pass feeds the
        // pill, so there is no per-pill `/proc` loop any more.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = sys_info_service().cpu.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(d) => {
                            let _ = out.send(CpuDashboardCmd::Cpu {
                                percent: d.usage_percent.round() as u32,
                                temp: d
                                    .temperature_celsius
                                    .filter(|t| *t > 0.0)
                                    .map(|t| t.round() as i32),
                            });
                        }
                        None => break,
                    },
                }
            }
        });
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = sys_info_service().memory.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(d) => {
                            let _ = out.send(CpuDashboardCmd::Mem {
                                percent: d.usage_percent.round() as u32,
                            });
                        }
                        None => break,
                    },
                }
            }
        });

        let model = CpuDashboardModel {
            cpu_percent: cpu0.usage_percent.round() as u32,
            temp_celsius,
            ram_percent: mem0.usage_percent.round() as u32,
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
            CpuDashboardInput::Clicked => {
                let _ = sender.output(CpuDashboardOutput::Clicked);
            }
            CpuDashboardInput::ToggleRamInBar => {
                self.show_ram_in_bar = !self.show_ram_in_bar;
            }
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            CpuDashboardCmd::Cpu { percent, temp } => {
                self.cpu_percent = percent;
                if let Some(t) = temp {
                    self.temp_celsius = t;
                } else if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg_pub(p)
                {
                    self.temp_celsius = t / 1000;
                }
            }
            CpuDashboardCmd::Mem { percent } => {
                self.ram_percent = percent;
            }
        }
        root.set_tooltip_text(Some(&format!(
            "CPU {}%  ·  Temp {}°C  ·  RAM {}%\nClick: open dashboard\nRight-click: toggle RAM in bar",
            self.cpu_percent, self.temp_celsius, self.ram_percent,
        )));
        self.update_view(widgets, sender);
    }
}
