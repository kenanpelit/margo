//! System stats bar pills — CPU load %, RAM used %, CPU temp °C.
//!
//! Three independent components sharing the same poll cadence
//! (2 s glib timer). Each pill reads its own sysfs / procfs file
//! per tick — cheap enough that a shared broadcast service would
//! be overkill. GPU monitoring is intentionally not included
//! here: GPU sysfs paths vary per vendor (nvidia-smi vs
//! amdgpu_busy_percent vs Intel) and a one-vendor implementation
//! invites bugs on every other rig. Add it later as a separate
//! widget when there's a portable backend.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

// ── CPU ─────────────────────────────────────────────────────────

pub(crate) struct CpuMonitorModel {
    percent: u32,
    prev_total: u64,
    prev_idle: u64,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum CpuMonitorInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum CpuMonitorOutput {}

pub(crate) struct CpuMonitorInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for CpuMonitorModel {
    type CommandOutput = ();
    type Input = CpuMonitorInput;
    type Output = CpuMonitorOutput;
    type Init = CpuMonitorInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "sysstat-bar-widget",
            add_css_class: "cpu",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_spacing: 4,

            gtk::Image {
                set_icon_name: Some("computer-symbolic"),
            },
            gtk::Label {
                add_css_class: "sysstat-bar-label",
                #[watch]
                set_label: &format!("{}%", model.percent),
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Prime the running delta so the first tick produces a
        // meaningful number instead of "100 % since boot".
        let (prev_total, prev_idle) = read_cpu_stat();
        schedule_poll(sender.clone(), || CpuMonitorInput::Poll);

        let model = CpuMonitorModel {
            percent: 0,
            prev_total,
            prev_idle,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CpuMonitorInput::Poll => {
                let (total, idle) = read_cpu_stat();
                let dt = total.saturating_sub(self.prev_total);
                let di = idle.saturating_sub(self.prev_idle);
                if dt > 0 {
                    self.percent = ((dt - di) * 100 / dt) as u32;
                }
                self.prev_total = total;
                self.prev_idle = idle;
            }
        }
    }
}

// ── RAM ─────────────────────────────────────────────────────────

pub(crate) struct RamMonitorModel {
    percent: u32,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum RamMonitorInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum RamMonitorOutput {}

pub(crate) struct RamMonitorInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for RamMonitorModel {
    type CommandOutput = ();
    type Input = RamMonitorInput;
    type Output = RamMonitorOutput;
    type Init = RamMonitorInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "sysstat-bar-widget",
            add_css_class: "ram",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_spacing: 4,

            gtk::Image {
                set_icon_name: Some("drive-harddisk-symbolic"),
            },
            gtk::Label {
                add_css_class: "sysstat-bar-label",
                #[watch]
                set_label: &format!("{}%", model.percent),
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        schedule_poll(sender.clone(), || RamMonitorInput::Poll);
        let model = RamMonitorModel {
            percent: read_ram_used_percent().unwrap_or(0),
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            RamMonitorInput::Poll => {
                if let Some(p) = read_ram_used_percent() {
                    self.percent = p;
                }
            }
        }
    }
}

// ── CPU temperature ──────────────────────────────────────────────

pub(crate) struct TempMonitorModel {
    celsius: i32,
    /// Cached hwmon path so we don't walk /sys every tick.
    /// `None` means we tried and there's no acceptable sensor.
    sensor_path: Option<PathBuf>,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum TempMonitorInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum TempMonitorOutput {}

pub(crate) struct TempMonitorInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for TempMonitorModel {
    type CommandOutput = ();
    type Input = TempMonitorInput;
    type Output = TempMonitorOutput;
    type Init = TempMonitorInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "sysstat-bar-widget",
            add_css_class: "temp",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_spacing: 4,
            #[watch]
            set_visible: model.sensor_path.is_some(),

            gtk::Image {
                set_icon_name: Some("temperature-symbolic"),
            },
            gtk::Label {
                add_css_class: "sysstat-bar-label",
                #[watch]
                set_label: &format!("{}°C", model.celsius),
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        schedule_poll(sender.clone(), || TempMonitorInput::Poll);
        let sensor_path = find_cpu_temp_sensor();
        let celsius = sensor_path
            .as_ref()
            .and_then(|p| read_temp_millideg(p))
            .map(|t| t / 1000)
            .unwrap_or(0);

        let model = TempMonitorModel {
            celsius,
            sensor_path,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            TempMonitorInput::Poll => {
                if let Some(p) = &self.sensor_path
                    && let Some(t) = read_temp_millideg(p)
                {
                    self.celsius = t / 1000;
                }
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Drive the bar widget's update tick via glib's main-loop
/// timer. One generic helper for the three components since they
/// all want the same 2 s cadence on the same thread. The closure
/// is monomorphised per call site so each component's enum is
/// untouched.
fn schedule_poll<I, F>(sender: relm4::ComponentSender<I>, make_msg: F)
where
    I: relm4::Component,
    I::Input: 'static,
    F: Fn() -> I::Input + 'static,
{
    relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
        sender.input(make_msg());
        relm4::gtk::glib::ControlFlow::Continue
    });
}

fn read_cpu_stat() -> (u64, u64) {
    let Ok(s) = std::fs::read_to_string("/proc/stat") else {
        return (0, 0);
    };
    let Some(first) = s.lines().next() else {
        return (0, 0);
    };
    // Format: "cpu user nice system idle iowait irq softirq steal guest guest_nice"
    let parts: Vec<u64> = first
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() < 4 {
        return (0, 0);
    }
    let total: u64 = parts.iter().sum();
    // `idle` (col 3) + `iowait` (col 4) both count as not-busy.
    let idle = parts[3] + parts.get(4).copied().unwrap_or(0);
    (total, idle)
}

fn read_ram_used_percent() -> Option<u32> {
    // /proc/meminfo has lines like:
    //   MemTotal:       16380000 kB
    //   MemAvailable:    8400000 kB
    // "Used" by the modern definition is `total - available`,
    // which matches what `free -h` shows in its "used" column.
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

/// Locate a CPU package temperature sensor under
/// `/sys/class/hwmon`. Preferred drivers in order: `coretemp`
/// (Intel), `k10temp` (AMD), `zenpower` (newer AMD third-party),
/// `acpitz` (generic ACPI thermal zone — used by ThinkPads, etc).
/// Caches `temp1_input` once found; the chosen device's path
/// won't move at runtime.
fn find_cpu_temp_sensor() -> Option<PathBuf> {
    // Logged once on cold-start so a missing sensor traces to a
    // recognisable place. After the first probe the result is
    // sticky for the lifetime of the bar widget.
    static LOGGED: AtomicBool = AtomicBool::new(false);

    let preferred = ["coretemp", "k10temp", "zenpower", "acpitz"];
    for want in preferred {
        let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
            return None;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
                continue;
            };
            if name.trim() == want {
                let p = dir.join("temp1_input");
                if p.exists() {
                    if !LOGGED.swap(true, Ordering::Relaxed) {
                        tracing::info!(
                            sensor = %name.trim(),
                            path = %p.display(),
                            "sysstat: cpu temperature sensor selected"
                        );
                    }
                    return Some(p);
                }
            }
        }
    }
    None
}

fn read_temp_millideg(path: &PathBuf) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
