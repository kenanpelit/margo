//! Battery bar pill.
//!
//! Shows three things at a glance, which the old icon-only widget
//! didn't:
//!   * **charge %** — a text label next to the icon.
//!   * **on AC vs on battery** — the icon uses the charging
//!     (bolt) variant whenever the line-power adapter is online
//!     (or UPower reports Charging / Fully charged), and the
//!     plain variant on battery.
//!   * **detail** — a tooltip spells out the power source and the
//!     UPower device state.
//!
//! Watches both the battery device (percentage / state /
//! presence) and the line-power adapter (`online`), so plugging
//! the charger in or out updates the pill immediately.

use mshell_services::{battery_service, line_power_service};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_battery::types::DeviceState;

#[derive(Debug, Clone)]
pub(crate) struct BatteryModel {}

#[derive(Debug)]
pub(crate) enum BatteryInput {}

#[derive(Debug)]
pub(crate) enum BatteryOutput {}

pub(crate) struct BatteryInit {}

#[derive(Debug)]
pub(crate) enum BatteryCommandOutput {
    BatteryStateChanged,
}

#[relm4::component(pub)]
impl Component for BatteryModel {
    type CommandOutput = BatteryCommandOutput;
    type Input = BatteryInput;
    type Output = BatteryOutput;
    type Init = BatteryInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["battery-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
                set_vexpand: true,

                #[name = "image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },

                #[name = "label"]
                gtk::Label {
                    add_css_class: "battery-bar-label",
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_battery_watcher(&sender, || BatteryCommandOutput::BatteryStateChanged);
        spawn_battery_online_watcher(&sender, || BatteryCommandOutput::BatteryStateChanged);

        let model = BatteryModel {};

        let widgets = view_output!();

        apply_battery(&widgets);

        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BatteryCommandOutput::BatteryStateChanged => {
                apply_battery(widgets);
            }
        }
    }
}

fn apply_battery(widgets: &BatteryModelWidgets) {
    let battery = battery_service().device.clone();

    let exists = battery.is_present.get();
    widgets.root.set_visible(exists);
    if !exists {
        return;
    }

    let percent = battery.percentage.get();
    let percent_int = percent.round().clamp(0.0, 100.0) as i32;
    let state = battery.state.get();

    // The line-power adapter is the direct "plugged in" signal;
    // fall back to the UPower device state when there's no
    // line-power device (some setups don't expose one).
    let on_ac = line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(state == DeviceState::Charging || state == DeviceState::FullyCharged);

    let charging_icon =
        on_ac || state == DeviceState::Charging || state == DeviceState::FullyCharged;
    if charging_icon {
        widgets
            .image
            .set_icon_name(Some(get_charging_battery_icon(percent)));
    } else {
        widgets.image.set_icon_name(Some(get_battery_icon(percent)));
    }

    widgets.label.set_label(&format!("{percent_int}%"));

    let state_str = match state {
        DeviceState::Charging => "Charging",
        DeviceState::Discharging => "Discharging",
        DeviceState::FullyCharged => "Fully charged",
        DeviceState::Empty => "Empty",
        DeviceState::PendingCharge => "Pending charge",
        DeviceState::PendingDischarge => "Pending discharge",
        DeviceState::Unknown => "Unknown",
    };
    let source = if on_ac { "On AC adapter" } else { "On battery" };
    widgets
        .root
        .set_tooltip_text(Some(&format!("{source}\n{state_str} · {percent_int}%")));
}
