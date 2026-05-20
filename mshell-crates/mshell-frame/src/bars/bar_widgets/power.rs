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
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

/// Visual-mode cycle driven by right-click on the pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Both,
    BatteryOnly,
    ProfileOnly,
}

impl DisplayMode {
    fn next(self) -> Self {
        match self {
            DisplayMode::Both => DisplayMode::BatteryOnly,
            DisplayMode::BatteryOnly => DisplayMode::ProfileOnly,
            DisplayMode::ProfileOnly => DisplayMode::Both,
        }
    }
}

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
pub(crate) struct PowerModel {
    state: PowerState,
    /// Cycles via right-click. Default shows both profile icon
    /// and battery icon + %. Ephemeral (in-memory only); the
    /// pill always starts a session in `Both`.
    mode: DisplayMode,
}

#[derive(Debug)]
pub(crate) enum PowerInput {
    Clicked,
    CycleMode,
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

                    // ── Profile slot ────────────────────────────
                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: matches!(
                            model.mode,
                            DisplayMode::Both | DisplayMode::ProfileOnly,
                        ),
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
                        set_visible: matches!(
                            model.mode,
                            DisplayMode::Both | DisplayMode::BatteryOnly,
                        ) && model.state.battery_available,

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
            mode: DisplayMode::Both,
        };
        let widgets = view_output!();

        // Right-click cycles the visible slots: Both → BatteryOnly
        // → ProfileOnly → Both. Left-click already opens the
        // power menu — secondary click is the cycle channel so
        // we don't fight the primary action.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(PowerInput::CycleMode);
        });
        widgets.button.add_controller(gesture);

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
            PowerInput::CycleMode => {
                self.mode = self.mode.next();
            }
        }
        // The state→icon application path also refreshes the
        // tooltip, so run it on every input even when only the
        // mode changed — keeps the tooltip's mode hint accurate.
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
        lines.push(String::new());
        lines.push("Right-click: cycle display mode".to_string());
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
