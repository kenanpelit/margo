//! Bar bluetooth indicator.
//!
//! Renders an icon for the adapter state (hardware missing /
//! disabled / on) and additionally flips a `.connected` class on
//! the widget root whenever at least one paired device is currently
//! connected, so the SCSS in `_bluetooth_widget.scss` can tint the
//! icon with the active matugen accent — a "this is hooked up to
//! my headphones right now" cue that just an icon swap doesn't
//! convey clearly at a glance.
//!
//! Three reactivity layers:
//!   * `available` + `enabled` (adapter properties) → icon change
//!   * `devices` (list) → re-spawn per-device watchers when device
//!                        set changes (pairing / unpairing)
//!   * `device.connected` (per device) → recompute `.connected` class

use mshell_common::WatcherToken;
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    set_bluetooth_icon, spawn_bluetooth_device_watcher, spawn_bluetooth_devices_watcher,
    spawn_bluetooth_enabled_watcher,
};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct BluetoothModel {
    /// Holds the cancellation tokens for the currently-attached
    /// per-device connectivity watchers. Reset (== cancel all) when
    /// the device list changes, then immediately re-populated below.
    devices_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum BluetoothInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum BluetoothOutput {
    Clicked,
}

pub(crate) struct BluetoothInit {}

#[derive(Debug)]
pub(crate) enum BluetoothCommandOutput {
    /// Adapter or device list changed — re-render icon + class, and
    /// re-attach per-device connectivity watchers.
    StatusChanged,
    /// At least one device.connected flipped — re-render the
    /// `.connected` class. Doesn't need to touch the device list.
    ConnectionChanged,
}

#[relm4::component(pub)]
impl Component for BluetoothModel {
    type CommandOutput = BluetoothCommandOutput;
    type Input = BluetoothInput;
    type Output = BluetoothOutput;
    type Init = BluetoothInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["bluetooth-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(BluetoothInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },

                    // Connected device name. Hidden (icon-only) until a
                    // device connects; `refresh` fills + reveals it.
                    #[name="label"]
                    gtk::Label {
                        add_css_class: "bluetooth-bar-label",
                        set_visible: false,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        set_max_width_chars: 18,
                    },
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_bluetooth_enabled_watcher(&sender, || BluetoothCommandOutput::StatusChanged);
        spawn_bluetooth_devices_watcher(&sender, || BluetoothCommandOutput::StatusChanged);

        let mut model = BluetoothModel {
            devices_token: WatcherToken::new(),
        };

        let widgets = view_output!();

        refresh(&widgets.image, &widgets.label, &root);

        // Initial per-device connectivity watchers — the watcher
        // for the device LIST only re-fires on add/remove, so
        // without a per-device listener `connected` flips would only
        // surface on the next adapter event.
        let token = model.devices_token.reset();
        for device in bluetooth_service().devices.get() {
            spawn_bluetooth_device_watcher(&device, token.clone(), &sender, || {
                BluetoothCommandOutput::ConnectionChanged
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            BluetoothInput::Clicked => {
                let _ = sender.output(BluetoothOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            BluetoothCommandOutput::StatusChanged => {
                refresh(&widgets.image, &widgets.label, root);

                // Device list may have grown / shrunk — cancel the
                // previous batch of per-device watchers and respawn
                // with the new set. WatcherToken::reset() cancels
                // every clone of the previous token in one shot.
                let token = self.devices_token.reset();
                for device in bluetooth_service().devices.get() {
                    spawn_bluetooth_device_watcher(&device, token.clone(), &sender, || {
                        BluetoothCommandOutput::ConnectionChanged
                    });
                }
            }
            BluetoothCommandOutput::ConnectionChanged => {
                refresh(&widgets.image, &widgets.label, root);
            }
        }
    }
}

fn refresh(image: &gtk::Image, label: &gtk::Label, root: &gtk::Box) {
    set_bluetooth_icon(image);

    let svc = bluetooth_service();
    // Name of the first currently-connected device, if any. Only the
    // adapter being present + on counts — a stale `connected` flag on a
    // disabled adapter shouldn't surface a name.
    let connected_name = if svc.available.get() && svc.enabled.get() {
        svc.devices
            .get()
            .iter()
            .find(|d| d.connected.get())
            .map(|d| d.alias.get().to_string())
    } else {
        None
    };

    match connected_name {
        Some(name) => {
            label.set_text(&name);
            label.set_visible(true);
            root.add_css_class("connected");
        }
        None => {
            label.set_visible(false);
            root.remove_css_class("connected");
        }
    }
}
