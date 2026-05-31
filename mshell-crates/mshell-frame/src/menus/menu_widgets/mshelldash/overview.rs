//! mshelldash Overview tab — an at-a-glance mosaic.
//!
//! Self-contained (no shared services): a live clock/date hero plus a
//! system glance card (CPU + memory gauges) sampled straight from
//! `/proc` on a 1 s `glib` timer. DESIGN.md compliant — surface cards
//! (no rings), --font-* type, matugen tokens, severity ladder on the
//! gauges (calm accent → warn primary → danger error).

use chrono::{Local, Timelike};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, RelmWidgetExt, SimpleComponent, gtk};

pub(crate) struct OverviewModel {
    time_text: String,
    date_text: String,
    greeting_text: String,
    cpu_pct: u32,
    cpu_detail: String,
    ram_pct: u32,
    ram_detail: String,
    uptime_text: String,
    // Previous `/proc/stat` (total, idle) sample for the CPU delta.
    prev_cpu: Option<(u64, u64)>,
}

#[derive(Debug)]
pub(crate) enum OverviewInput {
    Tick,
}

pub(crate) struct OverviewInit {}

#[relm4::component(pub(crate))]
impl SimpleComponent for OverviewModel {
    type Init = OverviewInit;
    type Input = OverviewInput;
    type Output = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "mshelldash-overview",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,
            set_hexpand: true,

            // ── Clock hero ─────────────────────────────────────────
            gtk::Box {
                add_css_class: "mshelldash-hero",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                set_hexpand: true,

                gtk::Label {
                    add_css_class: "mshelldash-hero-time",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &model.time_text,
                },
                gtk::Label {
                    add_css_class: "mshelldash-hero-date",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &model.date_text,
                },
                gtk::Label {
                    add_css_class: "mshelldash-hero-greeting",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 6,
                    #[watch]
                    set_label: &model.greeting_text,
                },
            },

            // ── System glance ──────────────────────────────────────
            gtk::Box {
                add_css_class: "mshelldash-card",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 10,
                set_hexpand: true,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    gtk::Label {
                        add_css_class: "mshelldash-section-label",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_label: "SYSTEM",
                    },
                    gtk::Label {
                        add_css_class: "mshelldash-stat-detail",
                        set_halign: gtk::Align::End,
                        #[watch]
                        set_label: &model.uptime_text,
                    },
                },

                // CPU row
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        gtk::Label {
                            add_css_class: "mshelldash-stat-caption",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                            set_label: "CPU",
                        },
                        gtk::Label {
                            add_css_class: "mshelldash-stat-detail",
                            set_halign: gtk::Align::End,
                            #[watch]
                            set_label: &model.cpu_detail,
                        },
                        gtk::Label {
                            add_css_class: "mshelldash-stat-value",
                            set_halign: gtk::Align::End,
                            set_margin_start: 8,
                            #[watch]
                            set_label: &format!("{}%", model.cpu_pct),
                        },
                    },
                    gtk::ProgressBar {
                        add_css_class: "mshelldash-bar",
                        #[watch]
                        set_fraction: model.cpu_pct as f64 / 100.0,
                        #[watch]
                        set_class_active: ("warn", model.cpu_pct >= 70 && model.cpu_pct < 90),
                        #[watch]
                        set_class_active: ("danger", model.cpu_pct >= 90),
                    },
                },

                // Memory row
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        gtk::Label {
                            add_css_class: "mshelldash-stat-caption",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                            set_label: "Memory",
                        },
                        gtk::Label {
                            add_css_class: "mshelldash-stat-detail",
                            set_halign: gtk::Align::End,
                            #[watch]
                            set_label: &model.ram_detail,
                        },
                        gtk::Label {
                            add_css_class: "mshelldash-stat-value",
                            set_halign: gtk::Align::End,
                            set_margin_start: 8,
                            #[watch]
                            set_label: &format!("{}%", model.ram_pct),
                        },
                    },
                    gtk::ProgressBar {
                        add_css_class: "mshelldash-bar",
                        #[watch]
                        set_fraction: model.ram_pct as f64 / 100.0,
                        #[watch]
                        set_class_active: ("warn", model.ram_pct >= 70 && model.ram_pct < 90),
                        #[watch]
                        set_class_active: ("danger", model.ram_pct >= 90),
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = OverviewModel {
            time_text: String::new(),
            date_text: String::new(),
            greeting_text: String::new(),
            cpu_pct: 0,
            cpu_detail: String::new(),
            ram_pct: 0,
            ram_detail: String::new(),
            uptime_text: String::new(),
            prev_cpu: None,
        };

        let widgets = view_output!();

        // Populate immediately, then refresh once a second.
        sender.input(OverviewInput::Tick);
        let s = sender.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            if s.input_sender().send(OverviewInput::Tick).is_err() {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            OverviewInput::Tick => {
                let now = Local::now();
                self.time_text = now.format("%H:%M").to_string();
                self.date_text = now.format("%A, %-d %B").to_string();
                self.greeting_text = greeting(now.hour());

                // CPU — busy delta vs. the previous sample.
                if let Some((total, idle)) = read_cpu_sample() {
                    if let Some((pt, pi)) = self.prev_cpu {
                        let dt = total.saturating_sub(pt);
                        let di = idle.saturating_sub(pi);
                        self.cpu_pct = if dt > 0 {
                            (((dt - di) * 100) / dt) as u32
                        } else {
                            self.cpu_pct
                        };
                    }
                    self.prev_cpu = Some((total, idle));
                }
                if let Some(ghz) = read_cpu_freq_ghz() {
                    self.cpu_detail = format!("{ghz:.1} GHz");
                }

                // Memory.
                if let Some((used, total)) = read_mem() {
                    self.ram_pct = if total > 0 {
                        ((used * 100) / total) as u32
                    } else {
                        0
                    };
                    self.ram_detail = format!("{} / {}", fmt_gb(used), fmt_gb(total));
                }

                if let Some(up) = read_uptime() {
                    self.uptime_text = format!("up {up}");
                }
            }
        }
    }
}

fn greeting(hour: u32) -> String {
    let part = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Good night",
    };
    match std::env::var("USER") {
        Ok(u) if !u.is_empty() => format!("{part}, {u}"),
        _ => part.to_string(),
    }
}

/// `(total, idle)` jiffies from the aggregate `/proc/stat` cpu line.
fn read_cpu_sample() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if f.len() < 5 {
        return None;
    }
    let total: u64 = f.iter().sum();
    let idle = f[3] + f[4]; // idle + iowait
    Some((total, idle))
}

fn read_cpu_freq_ghz() -> Option<f32> {
    let s = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut sum = 0.0f32;
    let mut n = 0u32;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("cpu MHz")
            && let Some(v) = rest
                .split(':')
                .nth(1)
                .and_then(|x| x.trim().parse::<f32>().ok())
        {
            sum += v;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    Some(sum / n as f32 / 1000.0)
}

/// `(used_kb, total_kb)` from `/proc/meminfo`.
fn read_mem() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = 0u64;
    let mut avail = 0u64;
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
        }
    }
    if total == 0 {
        return None;
    }
    Some((total.saturating_sub(avail), total))
}

fn fmt_gb(kb: u64) -> String {
    format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
}

fn read_uptime() -> Option<String> {
    let s = std::fs::read_to_string("/proc/uptime").ok()?;
    let secs: u64 = s.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    Some(if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    })
}
