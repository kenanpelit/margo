//! Combined Power Profile + Battery bar pill with right-click
//! mode cycling.
//!
//! Default rendering surfaces BOTH metrics in one pill:
//!
//!   ⚡ [profile-icon]   [battery-icon] 84%
//!
//! Right-click cycles the display mode through three states so a
//! user who only wants one of the two can compress the pill on
//! demand:
//!
//!   Both        → BatteryOnly  → ProfileOnly → Both → …
//!
//! Each per-tick mode change is local to the running mshell
//! session (no persistence) — the cycle is meant for ad-hoc bar
//! cleanup, not for a configured default.
//!
//! Data sources are the same wayle services the standalone Battery
//! and PowerProfile pills use; this widget just composes them in
//! one surface so the bar doesn't need two separate slots.

use mshell_services::{battery_service, line_power_service};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::power_profile::{
    get_active_power_profile_icon, get_power_profile_label, spawn_active_profile_watcher,
};
use relm4::gtk::prelude::{BoxExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_battery::types::DeviceState;

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

#[derive(Debug, Clone)]
pub(crate) struct PowerProfileModel {
    mode: DisplayMode,
}

#[derive(Debug)]
pub(crate) enum PowerProfileInput {
    CycleMode,
}

#[derive(Debug)]
pub(crate) enum PowerProfileOutput {}

pub(crate) struct PowerProfileInit {}

#[derive(Debug)]
pub(crate) enum PowerProfileCommandOutput {
    ProfileChanged,
    BatteryChanged,
}

#[relm4::component(pub)]
impl Component for PowerProfileModel {
    type CommandOutput = PowerProfileCommandOutput;
    type Input = PowerProfileInput;
    type Output = PowerProfileOutput;
    type Init = PowerProfileInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "power-profile-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
                set_vexpand: true,

                // ── Profile slot ────────────────────────────────
                #[name = "profile_image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_visible: matches!(
                        model.mode,
                        DisplayMode::Both | DisplayMode::ProfileOnly,
                    ),
                },

                // ── Battery slot (icon + percent) ───────────────
                //
                // Wrapper Box so the two children show/hide as one
                // unit; we don't want the % label hanging in the
                // bar without its icon.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_visible: matches!(
                        model.mode,
                        DisplayMode::Both | DisplayMode::BatteryOnly,
                    ),

                    #[name = "battery_image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },
                    #[name = "battery_label"]
                    gtk::Label {
                        add_css_class: "battery-bar-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
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
        spawn_active_profile_watcher(&sender, None, || {
            PowerProfileCommandOutput::ProfileChanged
        });
        spawn_battery_watcher(&sender, || PowerProfileCommandOutput::BatteryChanged);
        spawn_battery_online_watcher(&sender, || {
            PowerProfileCommandOutput::BatteryChanged
        });

        let model = PowerProfileModel {
            mode: DisplayMode::Both,
        };

        let widgets = view_output!();

        // Wire a right-click gesture to cycle the display mode.
        // The matugen primary tap (left-click) intentionally does
        // nothing — the pill is a status display, not a launcher.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(PowerProfileInput::CycleMode);
        });
        widgets.root.add_controller(gesture);

        apply_profile(&widgets);
        apply_battery(&widgets);

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
            PowerProfileInput::CycleMode => {
                self.mode = self.mode.next();
            }
        }
        // Apply once more after a mode flip so the tooltip text
        // refreshes alongside the visible-slot changes.
        apply_profile(widgets);
        apply_battery(widgets);
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PowerProfileCommandOutput::ProfileChanged => apply_profile(widgets),
            PowerProfileCommandOutput::BatteryChanged => apply_battery(widgets),
        }
        apply_tooltip(widgets);
    }
}

fn apply_profile(widgets: &PowerProfileModelWidgets) {
    widgets
        .profile_image
        .set_icon_name(Some(get_active_power_profile_icon()));
    apply_tooltip(widgets);
}

fn apply_battery(widgets: &PowerProfileModelWidgets) {
    let battery = battery_service().device.clone();
    let exists = battery.is_present.get();
    // Hide both children if there's no battery at all — desktops
    // would otherwise show an empty 0 % label next to a missing
    // icon.
    widgets.battery_image.set_visible(exists);
    widgets.battery_label.set_visible(exists);
    if !exists {
        apply_tooltip(widgets);
        return;
    }

    let percent = battery.percentage.get();
    let percent_int = percent.round().clamp(0.0, 100.0) as i32;
    let state = battery.state.get();
    let on_ac = line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(state == DeviceState::Charging || state == DeviceState::FullyCharged);
    let charging_icon =
        on_ac || state == DeviceState::Charging || state == DeviceState::FullyCharged;
    let icon = if charging_icon {
        get_charging_battery_icon(percent)
    } else {
        get_battery_icon(percent)
    };
    widgets.battery_image.set_icon_name(Some(icon));
    widgets.battery_label.set_label(&format!("{percent_int}%"));

    apply_tooltip(widgets);
}

fn apply_tooltip(widgets: &PowerProfileModelWidgets) {
    // Tooltip always carries the FULL story regardless of which
    // slots are currently visible — right-click cycle is a visual
    // compression, not a data filter.
    let profile = get_power_profile_label(
        &mshell_services::power_profile_service()
            .power_profiles
            .active_profile
            .get(),
    );

    let battery = battery_service().device.clone();
    let battery_line = if battery.is_present.get() {
        let percent = battery.percentage.get().round().clamp(0.0, 100.0) as i32;
        let state = battery.state.get();
        let on_ac = line_power_service()
            .map(|s| s.device.online.get())
            .unwrap_or(state == DeviceState::Charging || state == DeviceState::FullyCharged);
        let state_word = if on_ac {
            "Charging"
        } else {
            match state {
                DeviceState::Discharging => "On battery",
                DeviceState::FullyCharged => "Fully charged",
                _ => "Battery",
            }
        };
        format!("Battery {percent}% · {state_word}")
    } else {
        "No battery".to_string()
    };

    widgets.root.set_tooltip_text(Some(&format!(
        "Power Profile: {profile}\n{battery_line}\n\nRight-click to cycle display mode"
    )));
}
