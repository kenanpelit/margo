//! Power profile bar pill — port of the noctalia `power`
//! plugin's bar half.
//!
//! Render-only widget. Reactive: the profile comes from
//! `power_profile_service()` (power-profiles-daemon over D-Bus)
//! and battery / power-source from `battery_service()` +
//! `line_power_service()` (UPower over D-Bus) — no subprocess
//! polling. Click emits `PowerOutput::Clicked`; frame toggles
//! `MenuType::Power`.
//!
//! The system already ships a plain `PowerProfile` bar widget
//! backed by the same daemon, but it's icon-only with no panel.
//! This is the richer port: a profile switcher panel plus
//! battery / power-source readout, with the bar pill
//! colour-coded — performance = red, balanced = neutral,
//! power-saver = green.

use mshell_services::{battery_service, line_power_service, power_profile_service};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::power_profile::spawn_active_profile_watcher;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    PowerSaver,
    Balanced,
    Performance,
    Unknown,
}

impl Profile {
    pub(crate) fn from_wayle(p: &PowerProfile) -> Self {
        match p {
            PowerProfile::PowerSaver => Profile::PowerSaver,
            PowerProfile::Balanced => Profile::Balanced,
            PowerProfile::Performance => Profile::Performance,
            PowerProfile::Unknown => Profile::Unknown,
        }
    }

    /// The wayle `PowerProfile` this maps to — used to drive
    /// `PowerProfilesService::set_active_profile`.
    pub(crate) fn to_wayle(self) -> PowerProfile {
        match self {
            Profile::PowerSaver => PowerProfile::PowerSaver,
            Profile::Balanced => PowerProfile::Balanced,
            Profile::Performance => PowerProfile::Performance,
            Profile::Unknown => PowerProfile::Balanced,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Profile::PowerSaver => "Power Saver",
            Profile::Balanced => "Balanced",
            Profile::Performance => "Performance",
            Profile::Unknown => "Unknown",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Profile::PowerSaver => "power-profile-power-saver-symbolic",
            Profile::Balanced => "power-profile-balanced-symbolic",
            Profile::Performance => "power-profile-performance-symbolic",
            Profile::Unknown => "power-profile-balanced-symbolic",
        }
    }

    /// CSS state class on the bar pill / menu button:
    /// performance → red, power-saver → green, balanced → neutral.
    pub(crate) fn css_class(self) -> &'static str {
        match self {
            Profile::PowerSaver => "profile-saver",
            Profile::Balanced => "profile-balanced",
            Profile::Performance => "profile-performance",
            Profile::Unknown => "profile-unknown",
        }
    }
}

// No `Eq`: the new f64 detail fields (power draw / capacity Wh) aren't `Eq`.
// `PartialEq` is all the change-detection (`self.state != state`) needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PowerState {
    pub(crate) profile: Option<Profile>,
    /// "ac" / "battery" / "unknown".
    pub(crate) power_source: String,
    pub(crate) battery_available: bool,
    /// 0..=100, or None when no battery.
    pub(crate) battery_percent: Option<u8>,
    /// "Charging" / "Discharging" / "Full" / "Not charging" …
    pub(crate) battery_status: String,
    /// Battery health (UPower `capacity`, full charge vs design), 0..=100.
    pub(crate) battery_health: Option<u8>,
    /// Seconds until empty (discharging) or full (charging); `None`/0 hidden.
    pub(crate) time_remaining_secs: Option<i64>,
    /// `true` when `time_remaining_secs` counts up to full (charging).
    pub(crate) time_to_full: bool,
    /// Instantaneous power draw / charge rate in watts (UPower `energy_rate`).
    pub(crate) power_draw_w: Option<f64>,
    /// Present full-charge capacity in watt-hours (UPower `energy_full`).
    pub(crate) energy_full_wh: Option<f64>,
    /// Charge cycle count, when the firmware reports it.
    pub(crate) charge_cycles: Option<i32>,
    /// Battery charge limit (end threshold, %) via the kernel
    /// `charge_control_end_threshold` sysfs (thinkpad_acpi / generic).
    /// `None` = the platform doesn't expose one.
    pub(crate) charge_limit: Option<u8>,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PowerModel {
    state: PowerState,
}

#[derive(Debug)]
pub(crate) enum PowerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum PowerOutput {
    Clicked,
}

pub(crate) struct PowerInit {}

#[derive(Debug)]
pub(crate) enum PowerCommandOutput {
    /// The profile or battery state changed (D-Bus watcher fired).
    StateChanged,
}

#[relm4::component(pub)]
impl Component for PowerModel {
    type CommandOutput = PowerCommandOutput;
    type Input = PowerInput;
    type Output = PowerOutput;
    type Init = PowerInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "power-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(PowerInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    // ── Profile slot (fallback only) ────────────
                    //
                    // The pill leads with the battery; the power-
                    // profile glyph shows only when there's no
                    // battery (desktops) so the pill never goes
                    // empty.
                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: !model.state.battery_available,
                    },

                    // ── Battery slot (icon + percent) ───────────
                    //
                    // Wrapped so they show/hide as a unit; a
                    // dangling % label without its icon would
                    // look broken.
                    #[name="battery_slot"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: model.state.battery_available,

                        #[name="battery_image"]
                        gtk::Image {
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },
                        #[name="battery_label"]
                        gtk::Label {
                            add_css_class: "battery-bar-label",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Reactive — no polling. The profile comes from
        // power-profiles-daemon and battery / line-power from
        // UPower, all over D-Bus.
        spawn_active_profile_watcher(&sender, None, || PowerCommandOutput::StateChanged);
        spawn_battery_watcher(&sender, || PowerCommandOutput::StateChanged);
        spawn_battery_online_watcher(&sender, || PowerCommandOutput::StateChanged);

        let model = PowerModel {
            state: read_power_state(),
        };
        let widgets = view_output!();

        apply_visual(&widgets, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            PowerInput::Clicked => {
                let _ = sender.output(PowerOutput::Clicked);
            }
        }
        apply_visual(widgets, root, &self.state);
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            PowerCommandOutput::StateChanged => {
                let state = read_power_state();
                if self.state != state {
                    self.state = state;
                    apply_visual(widgets, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(widgets: &PowerModelWidgets, root: &gtk::Box, s: &PowerState) {
    let profile = s.profile.unwrap_or(Profile::Unknown);
    widgets.image.set_icon_name(Some(profile.icon()));

    // Battery icon + percent label. The slot wrapper's #[watch]
    // set_visible already hides the cluster when battery is
    // unavailable, but we still want to write the icon name so
    // a hot-plugged battery shows up correctly on the next state
    // change.
    if let Some(pct) = s.battery_percent {
        let pct_f = pct as f64;
        let on_ac = s.power_source == "ac"
            || s.battery_status == "Charging"
            || s.battery_status == "Full"
            || s.battery_status == "Fully charged";
        let icon = if on_ac {
            get_charging_battery_icon(pct_f)
        } else {
            get_battery_icon(pct_f)
        };
        widgets.battery_image.set_icon_name(Some(icon));
        widgets.battery_label.set_label(&format!("{pct}%"));
    }

    let tooltip = if let Some(err) = &s.error {
        format!("Power: {err}")
    } else {
        let mut lines = vec![format!("Profile: {}", profile.label())];
        if s.battery_available
            && let Some(pct) = s.battery_percent {
                lines.push(format!(
                    "Battery: {}% ({})",
                    pct,
                    if s.battery_status.is_empty() {
                        "unknown"
                    } else {
                        &s.battery_status
                    }
                ));
            }
        lines.push(format!(
            "Power source: {}",
            match s.power_source.as_str() {
                "ac" => "AC adapter",
                "battery" => "Battery",
                _ => "unknown",
            }
        ));
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    // Three-state colour: performance = red, power-saver =
    // green, balanced = neutral. Reset all before adding the
    // current so stale state doesn't pile up across refreshes.
    for c in ["profile-saver", "profile-balanced", "profile-performance", "profile-unknown"] {
        root.remove_css_class(c);
    }
    root.add_css_class(profile.css_class());
}

/// Snapshot the power state from the D-Bus-backed services.
/// Exposed `pub(crate)` so the menu widget reuses it.
pub(crate) fn read_power_state() -> PowerState {
    let mut state = PowerState::default();

    let profile = power_profile_service()
        .power_profiles
        .active_profile
        .get();
    state.profile = Some(Profile::from_wayle(&profile));

    let battery = battery_service().device.clone();
    let dev_state = battery.state.get();
    if battery.is_present.get() {
        state.battery_available = true;
        state.battery_percent =
            Some(battery.percentage.get().round().clamp(0.0, 100.0) as u8);
        state.battery_status = match dev_state {
            DeviceState::Charging => "Charging",
            DeviceState::Discharging => "Discharging",
            DeviceState::FullyCharged => "Full",
            DeviceState::Empty => "Empty",
            DeviceState::PendingCharge => "Not charging",
            DeviceState::PendingDischarge => "Not charging",
            DeviceState::Unknown => "",
        }
        .to_string();

        // Detail stats (shown in the power menu). Each is hidden when the
        // firmware reports nothing useful (0 / unknown).
        let health = battery.capacity.get();
        if health > 0.0 {
            state.battery_health = Some(health.round().clamp(0.0, 100.0) as u8);
        }
        let rate = battery.energy_rate.get();
        if rate > 0.01 {
            state.power_draw_w = Some(rate);
        }
        let full = battery.energy_full.get();
        if full > 0.0 {
            state.energy_full_wh = Some(full);
        }
        let cycles = battery.charge_cycles.get();
        if cycles > 0 {
            state.charge_cycles = Some(cycles);
        }
        match dev_state {
            DeviceState::Charging => {
                let t = battery.time_to_full.get();
                if t > 0 {
                    state.time_remaining_secs = Some(t);
                    state.time_to_full = true;
                }
            }
            DeviceState::Discharging => {
                let t = battery.time_to_empty.get();
                if t > 0 {
                    state.time_remaining_secs = Some(t);
                }
            }
            _ => {}
        }
    }

    // The line-power adapter is the direct "plugged in" signal;
    // fall back to the UPower device state when there's no
    // line-power device.
    let on_ac = line_power_service().map(|s| s.device.online.get()).unwrap_or(
        dev_state == DeviceState::Charging || dev_state == DeviceState::FullyCharged,
    );
    state.power_source = if on_ac {
        "ac".to_string()
    } else if state.battery_available {
        "battery".to_string()
    } else {
        "unknown".to_string()
    };

    state.charge_limit = read_charge_limit();

    state
}

/// Path to the battery's charge-limit (end threshold) sysfs file, if the
/// platform exposes one (`thinkpad_acpi` and the generic power-supply driver
/// both publish `charge_control_end_threshold`). World-readable; writing it
/// needs root.
pub(crate) fn charge_limit_end_path() -> Option<std::path::PathBuf> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let p = entry.path().join("charge_control_end_threshold");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Current charge limit (end threshold %), or `None` when unsupported.
pub(crate) fn read_charge_limit() -> Option<u8> {
    let p = charge_limit_end_path()?;
    std::fs::read_to_string(p).ok()?.trim().parse::<u8>().ok()
}

/// Set the charge limit (end threshold) via `pkexec` — the sysfs file is
/// root-owned, so the polkit agent prompts. The start threshold is nudged
/// just below the limit first (ThinkPad's EC rejects start ≥ end).
pub(crate) fn set_charge_limit(limit: u8) {
    let limit = limit.clamp(20, 100);
    let Some(end_path) = charge_limit_end_path() else {
        return;
    };
    let start_path = end_path.with_file_name("charge_control_start_threshold");
    let start = limit.saturating_sub(5);
    let script = format!(
        "echo {start} > '{}' 2>/dev/null; echo {limit} > '{}'",
        start_path.display(),
        end_path.display(),
    );
    let _ = std::process::Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(script)
        .spawn();
}
