//! CPU Dashboard menu widget — the right-pane content for
//! `MenuType::CpuDashboard`. Mirrors the popover content that
//! used to live inside the `cpu_dashboard` bar pill, ported into
//! the layer-shell menu surface so it opens contiguous with the
//! bar like ufw/dns instead of as a standalone xdg_popup.
//!
//! Driven by a 2 s self-cancelling poll over `/proc/stat`,
//! `/proc/cpuinfo`, `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`,
//! and the CPU package temperature sensor discovered on first poll:
//!   - CPU identity line (model · cores/threads)
//!   - Hero card: aggregate CPU% + current frequency + package temp
//!   - User vs System split
//!   - CPU-load history sparkline (~2 min window)
//!   - Per-core load bars
//!   - Memory (used / total + swap)
//!   - Load average (1m / 5m / 15m) + uptime
//!
//! Severity classes (calm / warn / danger) come from the same
//! thresholds the bar pill uses so the menu's tint reads the same
//! signal at the same time.

use crate::bars::bar_widgets::sysstat::{
    find_cpu_temp_sensor_pub, read_all_fans_pub, read_all_temp_sensors_pub, read_cpu_stat_pub,
    read_temp_millideg_pub,
};
use futures::StreamExt;
use mshell_services::sys_info_service;
use relm4::gtk::prelude::{
    BoxExt, DrawingAreaExt, DrawingAreaExtManual, GridExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::cell::Cell;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// History samples kept for the sparkline (2 s poll × 60 ≈ 2 min).
const HISTORY_LEN: usize = 60;

// Severity thresholds kept in sync with the bar pill copy so a
// glance at the cluster and a glance at the menu read the same
// state ladder.
const CPU_WARN_PERCENT: u32 = 70;
const CPU_DANGER_PERCENT: u32 = 90;
const TEMP_WARN_CELSIUS: i32 = 80;
const TEMP_DANGER_CELSIUS: i32 = 90;
/// Semantic temperature label: amber warning tier (--warning).
const TEMP_LABEL_WARN_CELSIUS: i32 = 75;
/// Semantic temperature label: red critical tier (--error).
const TEMP_LABEL_CRITICAL_CELSIUS: i32 = 85;

fn temp_label_class(temp: i32) -> &'static str {
    if temp >= TEMP_LABEL_CRITICAL_CELSIUS {
        "metric-critical"
    } else if temp >= TEMP_LABEL_WARN_CELSIUS {
        "metric-warning"
    } else {
        ""
    }
}

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

/// Heat-bucket class for a per-core tile. Low load washes in the matugen
/// accent (heat-1/2); the warn / danger thresholds escalate to the stable
/// amber / red tiers (heat-3/4) — same signal the rest of the dashboard uses,
/// without a rainbow gradient (DESIGN.md §Severity).
fn core_heat_class(cpu: u32) -> &'static str {
    if cpu >= CPU_DANGER_PERCENT {
        "heat-4"
    } else if cpu >= CPU_WARN_PERCENT {
        "heat-3"
    } else if cpu >= 50 {
        "heat-2"
    } else if cpu >= 25 {
        "heat-1"
    } else {
        "heat-0"
    }
}

/// Column count for the per-core heat-grid: a single row for small chips,
/// otherwise a calm 8-wide grid (22 threads → 8×3) that keeps tiles readable
/// at the panel width.
fn core_grid_cols(n: usize) -> i32 {
    if n <= 6 { n.max(1) as i32 } else { 8 }
}

#[derive(Default, Clone)]
struct CoreDeltas {
    prev_total: Vec<u64>,
    prev_idle: Vec<u64>,
    percent: Vec<u32>,
}

/// One per-core heat tile: a small card (`tile`) whose background class is
/// re-set each poll by load, holding a dim core index + the load %.
#[derive(Clone)]
struct CoreRow {
    tile: gtk::Box,
    pct_label: gtk::Label,
}

pub(crate) struct CpuDashboardMenuWidgetModel {
    cpu_percent: u32,
    user_percent: u32,
    system_percent: u32,
    freq_ghz: f32,
    freq_min_ghz: f32,
    freq_max_ghz: f32,
    temp_celsius: i32,
    /// All hwmon temperature sensors (CPU/GPU/NVMe/…) as `(label, °C)`.
    temps: Vec<(String, i32)>,
    /// All hwmon fans as `(label, rpm)`.
    fans: Vec<(String, u32)>,
    ram_percent: u32,
    mem_used_kb: u64,
    mem_total_kb: u64,
    swap_used_kb: u64,
    swap_total_kb: u64,
    load_1m: f32,
    load_5m: f32,
    load_15m: f32,
    uptime: String,
    cpu_model: String,
    cpu_cores: usize,
    cpu_threads: usize,
    prev_total: u64,
    prev_idle: u64,
    prev_user: u64,
    prev_system: u64,
    cores: CoreDeltas,
    sensor_path: Option<PathBuf>,
    core_rows: Vec<CoreRow>,
    /// CPU-load samples for the sparkline; shared with the draw func.
    history: Rc<RefCell<Vec<u32>>>,
    /// Reveal state, shared with the poll timer. The 2 s `Poll` does ~9
    /// /proc reads plus two full `/sys/class/hwmon` walks; gating it on
    /// reveal means a closed dashboard does none of that (the timer still
    /// wakes, but the work is skipped). Set by `ParentRevealChanged` from
    /// the host menu. Starts `true` since the widget is built on first
    /// reveal.
    revealed: Rc<Cell<bool>>,
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
    /// Host menu show/hide. Gates the poll so a closed dashboard does no
    /// /proc / hwmon work.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum CpuDashboardMenuWidgetOutput {}

pub(crate) struct CpuDashboardMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for CpuDashboardMenuWidgetModel {
    /// A storage snapshot pushed in from the `sys_info_service().disks`
    /// watch stream: `(mount, used_kb, total_kb, percent)` per filesystem.
    type CommandOutput = Vec<(String, u64, u64, u32)>;
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

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("cpu-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "CPU",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            // CPU identity — model + core/thread count.
            gtk::Label {
                add_css_class: "cpu-dashboard-model",
                #[watch]
                set_label: &model.identity_line(),
                #[watch]
                set_visible: !model.cpu_model.is_empty(),
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                set_wrap: true,
            },

            // Hero — aggregate CPU + frequency + temp.
            gtk::Box {
                #[watch]
                set_css_classes: &[
                    "cpu-dashboard-hero",
                    severity_class(model.cpu_percent, model.temp_celsius),
                ],
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 16,
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
                        set_label: &if model.freq_ghz > 0.0 {
                            format!("{:.1}", model.freq_ghz)
                        } else {
                            "—".to_string()
                        },
                        set_halign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-caption",
                        set_label: "GHZ",
                        set_halign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-hero-subcaption",
                        #[watch]
                        set_visible: model.freq_max_ghz > 0.0,
                        #[watch]
                        set_label: &format!("{:.1}–{:.1}", model.freq_min_ghz, model.freq_max_ghz),
                        set_halign: gtk::Align::Center,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    gtk::Label {
                        // Semantic temperature colour: amber at ≥75 °C,
                        // red at ≥85 °C. Replaces any accent borrowing
                        // (DESIGN.md §Severity ladder).
                        #[watch]
                        set_css_classes: &[
                            "cpu-dashboard-hero-value",
                            temp_label_class(model.temp_celsius),
                        ],
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

            // User vs System split + history sparkline.
            gtk::Label {
                add_css_class: "cpu-dashboard-split",
                #[watch]
                set_label: &format!(
                    "User {}%      System {}%",
                    model.user_percent, model.system_percent,
                ),
                set_halign: gtk::Align::Start,
            },
            #[name = "sparkline"]
            gtk::DrawingArea {
                add_css_class: "cpu-dashboard-sparkline",
                set_hexpand: true,
                set_content_height: 44,
            },

            // Per-core load — a compact heat-grid (tile per core), built
            // dynamically once the core count is known (see the poll handler).
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "PER-CORE",
                set_halign: gtk::Align::Start,
            },
            #[name = "cores_box"]
            gtk::Grid {
                add_css_class: "cpu-dashboard-core-grid",
                set_row_spacing: 4,
                set_column_spacing: 4,
                set_column_homogeneous: true,
            },

            // Sensors — one tidy line: the average across all hwmon
            // temperature sensors (CPU/GPU/NVMe/…) + the average fan
            // RPM. Each half hides when it has no readings; the whole
            // line hides on a sensorless host.
            gtk::Box {
                add_css_class: "cpu-dashboard-sensor-line",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 18,
                set_margin_top: 2,
                #[watch]
                set_visible: !model.temps.is_empty() || !model.fans.is_empty(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    #[watch]
                    set_visible: !model.temps.is_empty(),
                    gtk::Image {
                        add_css_class: "current-weather-detail-icon",
                        set_icon_name: Some("temperature-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-section-label",
                        set_label: "TEMP",
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        #[watch]
                        set_css_classes: &[
                            "cpu-dashboard-stat-value",
                            temp_label_class(model.avg_temp()),
                        ],
                        #[watch]
                        set_label: &format!("{}°C", model.avg_temp()),
                        set_valign: gtk::Align::Center,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    #[watch]
                    set_visible: !model.fans.is_empty(),
                    gtk::Image {
                        add_css_class: "current-weather-detail-icon",
                        set_icon_name: Some("weather-windy-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-section-label",
                        set_label: "FAN",
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-stat-value",
                        #[watch]
                        set_label: &format!("{} RPM", model.avg_fan()),
                        set_valign: gtk::Align::Center,
                    },
                },
            },

            // Memory + swap — symmetric labelled bar rows: caption,
            // bar, "used / total", percent. Same row shape so RAM and
            // Swap read as one consistent group.
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "MEMORY",
                set_halign: gtk::Align::Start,
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-caption",
                    set_label: "RAM",
                    set_width_chars: 4,
                    set_xalign: 0.0,
                },
                gtk::ProgressBar {
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
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
                    add_css_class: "cpu-dashboard-stat-detail",
                    #[watch]
                    set_label: &format!(
                        "{} / {}",
                        fmt_gb(model.mem_used_kb), fmt_gb(model.mem_total_kb),
                    ),
                },
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-value",
                    #[watch]
                    set_label: &format!("{}%", model.ram_percent),
                    set_width_chars: 4,
                    set_xalign: 1.0,
                },
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                #[watch]
                set_visible: model.swap_total_kb > 0,
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-caption",
                    set_label: "Swap",
                    set_width_chars: 4,
                    set_xalign: 0.0,
                },
                gtk::ProgressBar {
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_fraction: model.swap_fraction(),
                    add_css_class: "cpu-dashboard-bar",
                    add_css_class: "calm",
                },
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-detail",
                    #[watch]
                    set_label: &format!(
                        "{} / {}",
                        fmt_gb(model.swap_used_kb), fmt_gb(model.swap_total_kb),
                    ),
                },
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-value",
                    #[watch]
                    set_label: &format!("{}%", swap_pct(model.swap_used_kb, model.swap_total_kb)),
                    set_width_chars: 4,
                    set_xalign: 1.0,
                },
            },

            // Storage — per-mount usage, read from the shared
            // SysinfoService (disks). Same labelled-bar row shape as
            // Memory; rows are rebuilt imperatively each poll (the menu
            // reads the service's cached snapshot — no extra disk I/O).
            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "STORAGE",
                set_halign: gtk::Align::Start,
            },
            #[name = "disk_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },

            // Load average + uptime — two labelled stat columns
            // (caption above value), the same caption→value rhythm the
            // hero uses, so the footer doesn't read as raw text.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 16,
                set_homogeneous: true,
                set_margin_top: 4,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 3,
                    gtk::Label {
                        add_css_class: "cpu-dashboard-section-label",
                        set_label: "LOAD AVG · 1 / 5 / 15M",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-stat-value",
                        #[watch]
                        set_label: &format!(
                            "{:.2} · {:.2} · {:.2}",
                            model.load_1m, model.load_5m, model.load_15m,
                        ),
                        set_halign: gtk::Align::Start,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 3,
                    gtk::Label {
                        add_css_class: "cpu-dashboard-section-label",
                        set_label: "UPTIME",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "cpu-dashboard-stat-value",
                        #[watch]
                        set_label: &model.uptime,
                        set_halign: gtk::Align::Start,
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (prev_total, prev_idle, prev_user, prev_system) =
            read_cpu_breakdown().unwrap_or_else(|| {
                let (t, i) = read_cpu_stat_pub();
                (t, i, 0, 0)
            });
        let sensor_path = find_cpu_temp_sensor_pub();
        let temp_celsius = sensor_path
            .as_ref()
            .and_then(read_temp_millideg_pub)
            .map(|t| t / 1000)
            .unwrap_or(0);
        let (mem_used_kb, mem_total_kb, swap_used_kb, swap_total_kb) =
            read_mem_detail().unwrap_or((0, 0, 0, 0));
        let ram_percent = ram_pct(mem_used_kb, mem_total_kb);
        let (load_1m, load_5m, load_15m) = read_loadavg().unwrap_or((0.0, 0.0, 0.0));
        let (cpu_model, cpu_cores, cpu_threads) = read_cpu_info();

        // Reveal gate, shared with the poll timer below. `true` because
        // the widget is built on its menu's first reveal.
        let revealed = Rc::new(Cell::new(true));

        // Self-cancelling — Break when the receiver hangs up, so
        // the timer dies with the widget instead of running for
        // the rest of the shell session. Skips the (heavy: /proc + two
        // hwmon walks) `Poll` while the dashboard is closed.
        let sender_clone = sender.clone();
        let revealed_timer = revealed.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if !revealed_timer.get() {
                return relm4::gtk::glib::ControlFlow::Continue;
            }
            if sender_clone
                .input_sender()
                .send(CpuDashboardMenuWidgetInput::Poll)
                .is_err()
            {
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });

        let (freq_min_ghz, freq_avg_ghz, freq_max_ghz) =
            read_cpu_freq_stats().unwrap_or((0.0, 0.0, 0.0));

        let model = CpuDashboardMenuWidgetModel {
            cpu_percent: 0,
            user_percent: 0,
            system_percent: 0,
            freq_ghz: freq_avg_ghz,
            freq_min_ghz,
            freq_max_ghz,
            temp_celsius,
            temps: read_all_temp_sensors_pub(),
            fans: read_all_fans_pub(),
            ram_percent,
            mem_used_kb,
            mem_total_kb,
            swap_used_kb,
            swap_total_kb,
            load_1m,
            load_5m,
            load_15m,
            uptime: read_uptime().unwrap_or_default(),
            cpu_model,
            cpu_cores,
            cpu_threads,
            prev_total,
            prev_idle,
            prev_user,
            prev_system,
            cores: CoreDeltas::default(),
            sensor_path,
            core_rows: Vec::new(),
            history: Rc::new(RefCell::new(Vec::with_capacity(HISTORY_LEN))),
            revealed,
        };

        let widgets = view_output!();

        // Sparkline: a filled area + line of the CPU history. The
        // DrawingArea's CSS `color` (var(--primary)) is read at paint
        // time so the curve tracks the matugen accent.
        let hist = model.history.clone();
        widgets.sparkline.set_draw_func(move |area, cr, w, h| {
            let hist = hist.borrow();
            if hist.len() < 2 {
                return;
            }
            let w = w as f64;
            let h = h as f64;
            let c = area.color();
            let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
            let n = hist.len();
            let step = w / (n - 1) as f64;
            // Inset the curve a hair top/bottom so the stroke isn't clipped.
            let y_of = |p: u32| h - (p.min(100) as f64 / 100.0) * (h - 3.0) - 1.5;

            // Faint baseline so an idle (flat-low) or pegged (flat-high) trace
            // still reads as a graph rather than an empty band.
            cr.set_source_rgba(r, g, b, 0.12);
            cr.set_line_width(1.0);
            cr.move_to(0.0, h - 0.5);
            cr.line_to(w, h - 0.5);
            let _ = cr.stroke();

            // Filled area under the curve — a vertical gradient (accent near
            // the line, fading toward the baseline) so it reads as a polished
            // area chart, not a flat slab.
            cr.move_to(0.0, h);
            for (i, p) in hist.iter().enumerate() {
                cr.line_to(i as f64 * step, y_of(*p));
            }
            cr.line_to((n - 1) as f64 * step, h);
            cr.close_path();
            let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
            grad.add_color_stop_rgba(0.0, r, g, b, 0.34);
            grad.add_color_stop_rgba(1.0, r, g, b, 0.03);
            if cr.set_source(&grad).is_ok() {
                let _ = cr.fill();
            }

            // Line on top.
            for (i, p) in hist.iter().enumerate() {
                if i == 0 {
                    cr.move_to(0.0, y_of(*p));
                } else {
                    cr.line_to(i as f64 * step, y_of(*p));
                }
            }
            cr.set_source_rgba(r, g, b, 0.95);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        });

        // Subscribe to the shared service's disk metrics. wayle's pollers
        // only run while a property has a `.watch()` subscriber — a `.get()`
        // never wakes them — so the menu MUST watch (not poll) for the
        // Storage section to populate. The poller emits immediately on the
        // first subscribe, then on its own (slow) cadence.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = sys_info_service().disks.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(disks) => {
                            // Real filesystems only (≥1 GiB drops efi/tmpfs/loop
                            // noise), sorted + de-duplicated + capped.
                            let mut rows: Vec<(String, u64, u64, u32)> = disks
                                .into_iter()
                                .filter(|d| d.total_bytes >= 1024 * 1024 * 1024)
                                .map(|d| {
                                    (
                                        d.mount_point.to_string_lossy().into_owned(),
                                        d.used_bytes / 1024,
                                        d.total_bytes / 1024,
                                        d.usage_percent.round() as u32,
                                    )
                                })
                                .collect();
                            rows.sort_by(|a, b| a.0.cmp(&b.0));
                            rows.dedup_by(|a, b| a.0 == b.0);
                            rows.truncate(6);
                            let _ = out.send(rows);
                        }
                        None => break,
                    },
                }
            }
        });

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
            CpuDashboardMenuWidgetInput::ParentRevealChanged(visible) => {
                self.revealed.set(visible);
                // Refresh immediately on open so reappearing shows live
                // numbers instead of a (up to 2 s) stale snapshot.
                if visible {
                    sender.input(CpuDashboardMenuWidgetInput::Poll);
                }
            }
            CpuDashboardMenuWidgetInput::Poll => {
                if let Some((total, idle, user, system)) = read_cpu_breakdown() {
                    let delta_total = total.saturating_sub(self.prev_total);
                    if delta_total > 0 {
                        let delta_idle = idle.saturating_sub(self.prev_idle);
                        let busy = delta_total.saturating_sub(delta_idle);
                        self.cpu_percent = ((busy * 100) / delta_total) as u32;
                        self.user_percent =
                            ((user.saturating_sub(self.prev_user) * 100) / delta_total) as u32;
                        self.system_percent =
                            ((system.saturating_sub(self.prev_system) * 100) / delta_total) as u32;
                    }
                    self.prev_total = total;
                    self.prev_idle = idle;
                    self.prev_user = user;
                    self.prev_system = system;
                }

                // Push the fresh sample into the sparkline ring + repaint.
                {
                    let mut hist = self.history.borrow_mut();
                    hist.push(self.cpu_percent);
                    if hist.len() > HISTORY_LEN {
                        hist.remove(0);
                    }
                }
                widgets.sparkline.queue_draw();

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

                let cols = core_grid_cols(self.cores.percent.len());
                while self.core_rows.len() < self.cores.percent.len() {
                    let i = self.core_rows.len();
                    // Tile: a small vertical card — dim core index over the
                    // load %. The tile's background class carries the heat.
                    let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    tile.set_css_classes(&["cpu-dashboard-core-tile", "heat-0"]);
                    let num = gtk::Label::new(Some(&format!("c{i}")));
                    num.add_css_class("cpu-dashboard-core-num");
                    num.set_halign(gtk::Align::Center);
                    let pct_label = gtk::Label::new(Some("0%"));
                    pct_label.add_css_class("cpu-dashboard-core-pct");
                    pct_label.set_halign(gtk::Align::Center);
                    tile.append(&num);
                    tile.append(&pct_label);
                    let col = (i as i32) % cols;
                    let row = (i as i32) / cols;
                    widgets.cores_box.attach(&tile, col, row, 1, 1);
                    self.core_rows.push(CoreRow { tile, pct_label });
                }
                while self.core_rows.len() > self.cores.percent.len() {
                    if let Some(row) = self.core_rows.pop() {
                        widgets.cores_box.remove(&row.tile);
                    }
                }

                for (i, p) in self.cores.percent.iter().enumerate() {
                    if let Some(row) = self.core_rows.get(i) {
                        row.tile
                            .set_css_classes(&["cpu-dashboard-core-tile", core_heat_class(*p)]);
                        row.pct_label.set_label(&format!("{p}%"));
                    }
                }

                if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg_pub(p)
                {
                    self.temp_celsius = t / 1000;
                }

                if let Some((min, avg, max)) = read_cpu_freq_stats() {
                    self.freq_ghz = avg;
                    self.freq_min_ghz = min;
                    self.freq_max_ghz = max;
                }

                self.temps = read_all_temp_sensors_pub();
                self.fans = read_all_fans_pub();

                if let Some((used, total, swap_used, swap_total)) = read_mem_detail() {
                    self.mem_used_kb = used;
                    self.mem_total_kb = total;
                    self.swap_used_kb = swap_used;
                    self.swap_total_kb = swap_total;
                    self.ram_percent = ram_pct(used, total);
                }

                if let Some((a, b, c)) = read_loadavg() {
                    self.load_1m = a;
                    self.load_5m = b;
                    self.load_15m = c;
                }
                self.uptime = read_uptime().unwrap_or_else(|| self.uptime.clone());
            }
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        // A fresh disk snapshot arrived from the watch stream — repaint the
        // Storage rows.
        rebuild_disk_rows(&widgets.disk_box, &message);
    }
}

impl CpuDashboardMenuWidgetModel {
    fn identity_line(&self) -> String {
        format!(
            "{} · {}C / {}T",
            self.cpu_model, self.cpu_cores, self.cpu_threads
        )
    }

    fn swap_fraction(&self) -> f64 {
        if self.swap_total_kb == 0 {
            0.0
        } else {
            (self.swap_used_kb as f64) / (self.swap_total_kb as f64)
        }
    }

    /// Mean temperature across all hwmon sensors, rounded to °C. 0 when
    /// no sensors (the line is hidden in that case anyway).
    fn avg_temp(&self) -> i32 {
        if self.temps.is_empty() {
            return 0;
        }
        let sum: i32 = self.temps.iter().map(|(_, c)| *c).sum();
        sum / self.temps.len() as i32
    }

    /// Mean fan speed across all hwmon fans, rounded to RPM.
    fn avg_fan(&self) -> u32 {
        if self.fans.is_empty() {
            return 0;
        }
        let sum: u64 = self.fans.iter().map(|(_, rpm)| *rpm as u64).sum();
        (sum / self.fans.len() as u64) as u32
    }
}

/// KiB → "X.Y GB".
fn fmt_gb(kb: u64) -> String {
    format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
}

fn ram_pct(used_kb: u64, total_kb: u64) -> u32 {
    if total_kb == 0 {
        0
    } else {
        ((used_kb * 100) / total_kb) as u32
    }
}

fn swap_pct(used_kb: u64, total_kb: u64) -> u32 {
    ram_pct(used_kb, total_kb)
}

/// Storage severity bucket — disks fill slowly, so the thresholds sit
/// high (warn 80 %, danger 92 %). Same calm/warn/danger class names the
/// memory + hero bars use.
fn disk_severity(pct: u32) -> &'static str {
    if pct >= 92 {
        "danger"
    } else if pct >= 80 {
        "warn"
    } else {
        "calm"
    }
}

/// One storage row: mount caption + usage bar + "used / total" + percent —
/// the same shape as the Memory rows. Built with builders so it needs no
/// extra `*Ext` trait imports.
fn build_disk_row(mount: &str, used_kb: u64, total_kb: u64, pct: u32) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(mount)
            .css_classes(["cpu-dashboard-stat-caption"])
            .width_chars(6)
            .max_width_chars(10)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .xalign(0.0)
            .build(),
    );
    row.append(
        &gtk::ProgressBar::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .fraction((pct.min(100) as f64) / 100.0)
            .css_classes(["cpu-dashboard-bar", disk_severity(pct)])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(format!("{} / {}", fmt_gb(used_kb), fmt_gb(total_kb)))
            .css_classes(["cpu-dashboard-stat-detail"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(format!("{pct}%"))
            .css_classes(["cpu-dashboard-stat-value"])
            .width_chars(4)
            .xalign(1.0)
            .build(),
    );
    row
}

/// Rebuild the storage rows in `container` from a `(mount, used_kb,
/// total_kb, percent)` snapshot delivered by the disk watch stream.
fn rebuild_disk_rows(container: &gtk::Box, rows: &[(String, u64, u64, u32)]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for (mount, used_kb, total_kb, pct) in rows {
        container.append(&build_disk_row(mount, *used_kb, *total_kb, *pct));
    }
}

/// Read the aggregate `cpu` line of `/proc/stat` and return
/// `(total, idle, user_busy, system_busy)`. `user_busy = user + nice`,
/// `system_busy = system + irq + softirq`, `idle = idle + iowait`.
fn read_cpu_breakdown() -> Option<(u64, u64, u64, u64)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if f.len() < 4 {
        return None;
    }
    let g = |i: usize| f.get(i).copied().unwrap_or(0);
    let total: u64 = f.iter().sum();
    let idle = g(3) + g(4); // idle + iowait
    let user = g(0) + g(1); // user + nice
    let system = g(2) + g(5) + g(6); // system + irq + softirq
    Some((total, idle, user, system))
}

/// `(min, avg, max)` current CPU frequency in GHz across all logical
/// CPUs, from `/proc/cpuinfo`'s `cpu MHz` lines. The spread is useful
/// on modern CPUs where idle cores park at a low clock while one boosts.
fn read_cpu_freq_stats() -> Option<(f32, f32, f32)> {
    let s = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut sum = 0.0f32;
    let mut n = 0u32;
    let mut min = f32::MAX;
    let mut max = 0.0f32;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("cpu MHz")
            && let Some(v) = rest
                .split(':')
                .nth(1)
                .and_then(|x| x.trim().parse::<f32>().ok())
        {
            sum += v;
            n += 1;
            min = min.min(v);
            max = max.max(v);
        }
    }
    if n == 0 {
        return None;
    }
    Some((min / 1000.0, sum / n as f32 / 1000.0, max / 1000.0))
}

/// `(model name, physical cores, logical threads)` from `/proc/cpuinfo`.
fn read_cpu_info() -> (String, usize, usize) {
    let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") else {
        return (String::new(), 0, 0);
    };
    let mut model = String::new();
    let mut threads = 0usize;
    let mut cores = 0usize;
    for line in s.lines() {
        if line.starts_with("processor") {
            threads += 1;
        } else if model.is_empty()
            && let Some(rest) = line.strip_prefix("model name")
        {
            model = rest
                .split(':')
                .nth(1)
                .map(|x| x.trim().to_string())
                .unwrap_or_default();
        } else if cores == 0
            && let Some(rest) = line.strip_prefix("cpu cores")
        {
            cores = rest
                .split(':')
                .nth(1)
                .and_then(|x| x.trim().parse().ok())
                .unwrap_or(0);
        }
    }
    if cores == 0 {
        cores = threads;
    }
    // Trim the marketing noise ("(R)", "(TM)", "CPU @ 3.0GHz") a touch
    // so the identity line fits the menu width.
    let model = model
        .replace("(R)", "")
        .replace("(TM)", "")
        .split(" @ ")
        .next()
        .unwrap_or(&model)
        .trim()
        .to_string();
    (model, cores, threads)
}

/// `(used_kb, total_kb, swap_used_kb, swap_total_kb)` from `/proc/meminfo`.
fn read_mem_detail() -> Option<(u64, u64, u64, u64)> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = 0u64;
    let mut avail = 0u64;
    let mut swap_total = 0u64;
    let mut swap_free = 0u64;
    for line in s.lines() {
        let val = |rest: &str| -> u64 {
            rest.split_whitespace()
                .next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(0)
        };
        if let Some(r) = line.strip_prefix("MemTotal:") {
            total = val(r);
        } else if let Some(r) = line.strip_prefix("MemAvailable:") {
            avail = val(r);
        } else if let Some(r) = line.strip_prefix("SwapTotal:") {
            swap_total = val(r);
        } else if let Some(r) = line.strip_prefix("SwapFree:") {
            swap_free = val(r);
        }
    }
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(avail);
    let swap_used = swap_total.saturating_sub(swap_free);
    Some((used, total, swap_used, swap_total))
}

fn read_uptime() -> Option<String> {
    let s = std::fs::read_to_string("/proc/uptime").ok()?;
    let secs: u64 = s.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    Some(if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    })
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

fn read_loadavg() -> Option<(f32, f32, f32)> {
    let s = std::fs::read_to_string("/proc/loadavg").ok()?;
    let mut it = s.split_whitespace();
    let a: f32 = it.next()?.parse().ok()?;
    let b: f32 = it.next()?.parse().ok()?;
    let c: f32 = it.next()?.parse().ok()?;
    Some((a, b, c))
}
