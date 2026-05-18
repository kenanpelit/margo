//! Dashboard "System" tile — compact three-metric card combining
//! the active power profile, battery state, and package CPU
//! temperature into one row so the right column doesn't sprout
//! three separate widgets for what reads as one health glance.
//!
//! Layout:
//!
//!   ┌─────────────────────────────────────────────────┐
//!   │  ⚡ Performance    🔋 84%        🌡 58°C        │
//!   │  Power Profile    Discharging    Normal         │
//!   └─────────────────────────────────────────────────┘
//!
//! Each metric block is its own column inside a horizontal Box;
//! faint vertical rules (CSS, not widgets) separate them so the
//! card reads as a unified summary instead of three glued tiles.
//!
//! Data sources reused from the existing widgets:
//!   - power_profile_service() → active profile (D-Bus, reactive)
//!   - battery_service() + line_power_service() → battery %, state
//!   - hwmon `temp1_input` discovery for package CPU temp (2 s
//!     poll — the same path the CpuTemp sysstat pill walks).
//!
//! Severity colouring is intentionally subtle: battery < 20 % gets
//! an `--error` tint, temp > 80 °C the same. The pill stays "calm"
//! by default so the dashboard reads quiet.

use mshell_common::scoped_effects::EffectScope;
use mshell_services::{battery_service, line_power_service, power_profile_service};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::power_profile::{
    get_power_profile_icon, get_power_profile_label, spawn_active_profile_watcher,
};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

// Battery + temp thresholds — sized so the tile stays calm in the
// common case and only colours when there's something to act on.
const BATTERY_LOW_PERCENT: i32 = 20;
const TEMP_WARN_CELSIUS: i32 = 80;

pub(crate) struct SystemStatusModel {
    has_battery: bool,
    battery_percent: i32,
    battery_charging: bool,
    battery_state: DeviceState,
    profile: PowerProfile,
    temp_celsius: i32,
    temp_sensor_path: Option<PathBuf>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum SystemStatusInput {
    BatteryChanged,
    ProfileChanged,
    PollTemp,
}

#[derive(Debug)]
pub(crate) enum SystemStatusOutput {}

pub(crate) struct SystemStatusInit {}

#[derive(Debug)]
pub(crate) enum SystemStatusCommandOutput {
    BatteryChanged,
    ProfileChanged,
}

#[relm4::component(pub)]
impl Component for SystemStatusModel {
    type CommandOutput = SystemStatusCommandOutput;
    type Input = SystemStatusInput;
    type Output = SystemStatusOutput;
    type Init = SystemStatusInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "system-status-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 4,

            // ── Power profile row ───────────────────────────────
            gtk::Box {
                add_css_class: "system-status-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                gtk::Image {
                    add_css_class: "system-status-icon",
                    #[watch]
                    set_icon_name: Some(get_power_profile_icon(&model.profile)),
                },
                gtk::Label {
                    add_css_class: "system-status-caption",
                    set_label: "Power Mode",
                    set_halign: gtk::Align::Start,
                },
                // Right-pushed value.
                gtk::Box {
                    set_hexpand: true,
                },
                gtk::Label {
                    add_css_class: "system-status-value",
                    #[watch]
                    set_label: get_power_profile_label(&model.profile),
                    set_halign: gtk::Align::End,
                },
            },

            // ── Battery row ─────────────────────────────────────
            gtk::Box {
                #[watch]
                set_visible: model.has_battery,
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                #[watch]
                set_css_classes: &[
                    "system-status-row",
                    if model.battery_percent <= BATTERY_LOW_PERCENT
                        && !model.battery_charging
                    {
                        "warn"
                    } else {
                        "calm"
                    },
                ],

                gtk::Image {
                    add_css_class: "system-status-icon",
                    #[watch]
                    set_icon_name: Some(battery_icon_name(
                        model.battery_percent,
                        model.battery_charging,
                    )),
                },
                gtk::Label {
                    add_css_class: "system-status-caption",
                    #[watch]
                    set_label: battery_state_label(model.battery_state, model.battery_charging),
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    set_hexpand: true,
                },
                gtk::Label {
                    add_css_class: "system-status-value",
                    #[watch]
                    set_label: &format!("{}%", model.battery_percent),
                    set_halign: gtk::Align::End,
                },
            },

            // ── Temperature row ─────────────────────────────────
            gtk::Box {
                #[watch]
                set_visible: model.temp_sensor_path.is_some(),
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                #[watch]
                set_css_classes: &[
                    "system-status-row",
                    if model.temp_celsius >= TEMP_WARN_CELSIUS { "warn" } else { "calm" },
                ],

                gtk::Image {
                    add_css_class: "system-status-icon",
                    set_icon_name: Some("temperature-symbolic"),
                },
                gtk::Label {
                    add_css_class: "system-status-caption",
                    set_label: "CPU Temp",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    set_hexpand: true,
                },
                gtk::Label {
                    add_css_class: "system-status-value",
                    #[watch]
                    set_label: &format!("{}°C", model.temp_celsius),
                    set_halign: gtk::Align::End,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_battery_watcher(&sender, || SystemStatusCommandOutput::BatteryChanged);
        spawn_battery_online_watcher(&sender, || SystemStatusCommandOutput::BatteryChanged);
        spawn_active_profile_watcher(&sender, None, || {
            SystemStatusCommandOutput::ProfileChanged
        });

        // Self-cancelling temp poll (mirrors sysstat pattern).
        let sender_clone = sender.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if sender_clone
                .input_sender()
                .send(SystemStatusInput::PollTemp)
                .is_err()
            {
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });

        // Prime initial state.
        let battery = battery_service().device.clone();
        let has_battery = battery.is_present.get();
        let battery_percent = battery.percentage.get().round().clamp(0.0, 100.0) as i32;
        let battery_state = battery.state.get();
        let battery_charging = is_on_ac(battery_state);

        let profile = power_profile_service().power_profiles.active_profile.get();

        let temp_sensor_path = find_cpu_temp_sensor();
        let temp_celsius = temp_sensor_path
            .as_ref()
            .and_then(read_temp_millideg)
            .map(|t| t / 1000)
            .unwrap_or(0);

        let model = SystemStatusModel {
            has_battery,
            battery_percent,
            battery_charging,
            battery_state,
            profile,
            temp_celsius,
            temp_sensor_path,
            _effects: EffectScope::new(),
        };

        let widgets = view_output!();

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemStatusCommandOutput::BatteryChanged => {
                sender.input(SystemStatusInput::BatteryChanged);
            }
            SystemStatusCommandOutput::ProfileChanged => {
                sender.input(SystemStatusInput::ProfileChanged);
            }
        }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemStatusInput::BatteryChanged => {
                let battery = battery_service().device.clone();
                self.has_battery = battery.is_present.get();
                self.battery_percent =
                    battery.percentage.get().round().clamp(0.0, 100.0) as i32;
                self.battery_state = battery.state.get();
                self.battery_charging = is_on_ac(self.battery_state);
            }
            SystemStatusInput::ProfileChanged => {
                self.profile = power_profile_service().power_profiles.active_profile.get();
            }
            SystemStatusInput::PollTemp => {
                if let Some(p) = &self.temp_sensor_path
                    && let Some(t) = read_temp_millideg(p)
                {
                    self.temp_celsius = t / 1000;
                }
            }
        }
    }
}

fn is_on_ac(state: DeviceState) -> bool {
    line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(state == DeviceState::Charging || state == DeviceState::FullyCharged)
}

fn battery_icon_name(percent: i32, charging: bool) -> &'static str {
    let p = percent.clamp(0, 100) as f64;
    if charging {
        get_charging_battery_icon(p)
    } else {
        get_battery_icon(p)
    }
}

fn battery_state_label(state: DeviceState, charging: bool) -> &'static str {
    if charging {
        match state {
            DeviceState::FullyCharged => "Fully charged",
            _ => "Charging",
        }
    } else {
        match state {
            DeviceState::Discharging => "On battery",
            DeviceState::Empty => "Empty",
            DeviceState::FullyCharged => "Fully charged",
            DeviceState::PendingCharge => "Pending",
            DeviceState::PendingDischarge => "Pending",
            DeviceState::Charging => "Charging",
            DeviceState::Unknown => "Unknown",
        }
    }
}

// ── Temperature reading helpers ─────────────────────────────────
//
// These duplicate the small bit of /sys walking from
// bar_widgets/sysstat.rs so SystemStatus doesn't need to take a
// dep on that module's pub helpers. The logic is short and the
// /sys layout is fixed.

fn find_cpu_temp_sensor() -> Option<PathBuf> {
    let hwmon_dir = std::fs::read_dir("/sys/class/hwmon").ok()?;
    let mut k10temp: Option<PathBuf> = None;
    let mut coretemp: Option<PathBuf> = None;
    let mut acpitz: Option<PathBuf> = None;
    let mut other: Option<PathBuf> = None;
    for entry in hwmon_dir.flatten() {
        let p = entry.path();
        let name_path = p.join("name");
        let Ok(name) = std::fs::read_to_string(&name_path) else {
            continue;
        };
        let name = name.trim();
        let temp_path = p.join("temp1_input");
        if !temp_path.exists() {
            continue;
        }
        match name {
            "k10temp" => k10temp.get_or_insert(temp_path),
            "coretemp" => coretemp.get_or_insert(temp_path),
            "acpitz" => acpitz.get_or_insert(temp_path),
            _ => other.get_or_insert(temp_path),
        };
    }
    k10temp.or(coretemp).or(acpitz).or(other)
}

fn read_temp_millideg(p: &PathBuf) -> Option<i32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}
