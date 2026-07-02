//! Control Center power-profile segmented control.
//!
//! ```text
//! [ Saver ] [ Balanced ] [ Performance ]
//! ```
//!
//! Reads the live profile from `PowerProfilesService` and sets it
//! (async, off the UI thread via `spawn_future_local`) on click. The
//! whole row is hidden by the parent when the user disables it in
//! Settings → Widgets → Control Center.

use mshell_services::power_profile_service;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_power_profiles::types::profile::PowerProfile;

pub(crate) struct ControlCenterPowerProfileModel {
    active: PowerProfile,
}

#[derive(Debug)]
pub(crate) enum ControlCenterPowerProfileInput {
    /// Re-read the active profile (called on panel reveal).
    Refresh,
    /// User picked a profile.
    Set(PowerProfile),
}

pub(crate) struct ControlCenterPowerProfileInit {}

fn read_active() -> PowerProfile {
    power_profile_service()
        .map(|s| s.power_profiles.active_profile.get())
        .unwrap_or(PowerProfile::Unknown)
}

#[relm4::component(pub(crate))]
impl Component for ControlCenterPowerProfileModel {
    type CommandOutput = ();
    type Input = ControlCenterPowerProfileInput;
    type Output = ();
    type Init = ControlCenterPowerProfileInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "control-center-power-profile",
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 6,
            set_homogeneous: true,

            gtk::Button {
                #[watch]
                set_css_classes: if model.active == PowerProfile::PowerSaver {
                    &["control-center-pp-btn", "active"]
                } else {
                    &["control-center-pp-btn"]
                },
                set_tooltip_text: Some("Power Saver"),
                connect_clicked[sender] => move |_| {
                    sender.input(ControlCenterPowerProfileInput::Set(PowerProfile::PowerSaver));
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    gtk::Image { set_icon_name: Some("power-profile-power-saver-symbolic") },
                    gtk::Label { set_label: "Saver" },
                },
            },

            gtk::Button {
                #[watch]
                set_css_classes: if model.active == PowerProfile::Balanced {
                    &["control-center-pp-btn", "active"]
                } else {
                    &["control-center-pp-btn"]
                },
                set_tooltip_text: Some("Balanced"),
                connect_clicked[sender] => move |_| {
                    sender.input(ControlCenterPowerProfileInput::Set(PowerProfile::Balanced));
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    gtk::Image { set_icon_name: Some("power-profile-balanced-symbolic") },
                    gtk::Label { set_label: "Balanced" },
                },
            },

            gtk::Button {
                #[watch]
                set_css_classes: if model.active == PowerProfile::Performance {
                    &["control-center-pp-btn", "active"]
                } else {
                    &["control-center-pp-btn"]
                },
                set_tooltip_text: Some("Performance"),
                connect_clicked[sender] => move |_| {
                    sender.input(ControlCenterPowerProfileInput::Set(PowerProfile::Performance));
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    gtk::Image { set_icon_name: Some("power-profile-performance-symbolic") },
                    gtk::Label { set_label: "Performance" },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = &sender;
        let model = ControlCenterPowerProfileModel {
            active: read_active(),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ControlCenterPowerProfileInput::Refresh => {
                self.active = read_active();
            }
            ControlCenterPowerProfileInput::Set(profile) => {
                // Optimistic highlight; the daemon confirms via the next
                // Refresh (panel reveal) if it differs.
                self.active = profile;
                glib::spawn_future_local(async move {
                    let Some(svc) = power_profile_service() else {
                        return;
                    };
                    if let Err(e) = svc.power_profiles.set_active_profile(profile).await {
                        tracing::warn!(error = %e, "control-center: set power profile failed");
                    }
                });
            }
        }
    }
}
