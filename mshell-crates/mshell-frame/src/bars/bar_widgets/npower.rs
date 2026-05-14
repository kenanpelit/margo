//! Power profile bar pill — port of the noctalia `npower`
//! plugin's bar half.
//!
//! Render-only widget. Reactive: the profile comes from
//! `power_profile_service()` (power-profiles-daemon over D-Bus)
//! and battery / power-source from `battery_service()` +
//! `line_power_service()` (UPower over D-Bus) — no subprocess
//! polling. Click emits `NpowerOutput::Clicked`; frame toggles
//! `MenuType::Npower`.
//!
//! The system already ships a plain `PowerProfile` bar widget
//! backed by the same daemon, but it's icon-only with no panel.
//! This is the richer port: a profile switcher panel plus
//! battery / power-source readout, with the bar pill
//! colour-coded — performance = red, balanced = neutral,
//! power-saver = green.

use mshell_services::{battery_service, line_power_service, power_profile_service};
use mshell_utils::battery::{spawn_battery_online_watcher, spawn_battery_watcher};
use mshell_utils::power_profile::spawn_active_profile_watcher;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PowerState {
    pub(crate) profile: Option<Profile>,
    /// "ac" / "battery" / "unknown".
    pub(crate) power_source: String,
    pub(crate) battery_available: bool,
    /// 0..=100, or None when no battery.
    pub(crate) battery_percent: Option<u8>,
    /// "Charging" / "Discharging" / "Full" / "Not charging" …
    pub(crate) battery_status: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct NpowerModel {
    state: PowerState,
}

#[derive(Debug)]
pub(crate) enum NpowerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NpowerOutput {
    Clicked,
}

pub(crate) struct NpowerInit {}

#[derive(Debug)]
pub(crate) enum NpowerCommandOutput {
    /// The profile or battery state changed (D-Bus watcher fired).
    StateChanged,
}

#[relm4::component(pub)]
impl Component for NpowerModel {
    type CommandOutput = NpowerCommandOutput;
    type Input = NpowerInput;
    type Output = NpowerOutput;
    type Init = NpowerInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "npower-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NpowerInput::Clicked);
                },

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
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
        spawn_active_profile_watcher(&sender, None, || NpowerCommandOutput::StateChanged);
        spawn_battery_watcher(&sender, || NpowerCommandOutput::StateChanged);
        spawn_battery_online_watcher(&sender, || NpowerCommandOutput::StateChanged);

        let model = NpowerModel {
            state: read_power_state(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpowerInput::Clicked => {
                let _ = sender.output(NpowerOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NpowerCommandOutput::StateChanged => {
                let state = read_power_state();
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &PowerState) {
    let profile = s.profile.unwrap_or(Profile::Unknown);
    image.set_icon_name(Some(profile.icon()));

    let tooltip = if let Some(err) = &s.error {
        format!("Power: {err}")
    } else {
        let mut lines = vec![format!("Profile: {}", profile.label())];
        if s.battery_available {
            if let Some(pct) = s.battery_percent {
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

    state
}
