//! Bluetooth Dashboard menu widget — content surface for
//! `MenuType::Bluetooth`.
//!
//! Mirrors the Audio Dashboard design language:
//!   * **panel-header** — icon + title + power Switch.
//!   * **DEVICES section label** — shown when adapter is on.
//!   * **Flat device rows** — icon + alias + battery % + connected
//!     check accent.  Row click → connect/disconnect (or pair if
//!     unpaired).  Compact Forget / Trust inline buttons.
//!
//! State is read from `bluetooth_service()` (wayle-bluetooth over
//! D-Bus).  Async actions are dispatched with `tokio::spawn`.
//! Discovery starts/stops on `ParentRevealChanged(true/false)`.

use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    get_bluetooth_device_icon, spawn_bluetooth_devices_watcher, spawn_bluetooth_enabled_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

#[derive(Debug)]
pub(crate) struct BluetoothMenuWidgetModel {
    available: bool,
    enabled: bool,
    devices: Vec<Arc<Device>>,
}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetInput {
    /// Sent by the frame when the Bluetooth menu surface is
    /// shown (`true`) or hidden (`false`). Drives discovery.
    ParentRevealChanged(bool),
    /// Internal: toggle power.
    SetEnabled(bool),
    /// Internal: connect or disconnect a device by address.
    ConnectToggle(String),
    /// Internal: pair an unpaired device.
    Pair(String),
    /// Internal: remove a paired device.
    Forget(String),
    /// Internal: flip the trusted flag.
    SetTrusted(String, bool),
    /// Internal: re-read service state + rebuild list.
    RefreshState,
}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetOutput {}

pub(crate) struct BluetoothMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetCommandOutput {
    BluetoothStateChanged,
    BluetoothDevicesChanged,
}

#[relm4::component(pub(crate))]
impl Component for BluetoothMenuWidgetModel {
    type CommandOutput = BluetoothMenuWidgetCommandOutput;
    type Input = BluetoothMenuWidgetInput;
    type Output = BluetoothMenuWidgetOutput;
    type Init = BluetoothMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "bluetooth-dashboard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("bluetooth-active-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Bluetooth",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
                gtk::Switch {
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_visible: model.available,
                    #[watch]
                    #[block_signal(bt_power_handler)]
                    set_active: model.enabled,
                    connect_state_set[sender] => move |_, enabled| {
                        sender.input(BluetoothMenuWidgetInput::SetEnabled(enabled));
                        glib::Propagation::Proceed
                    } @bt_power_handler,
                },
            },

            // ── Hardware-missing note ────────────────────────────
            gtk::Label {
                add_css_class: "label-small",
                set_label: "Bluetooth hardware missing",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                #[watch]
                set_visible: !model.available,
            },

            // ── DEVICES section label ────────────────────────────
            gtk::Label {
                add_css_class: "bluetooth-dashboard-section-label",
                set_label: "DEVICES",
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.available && model.enabled,
            },

            // ── Radio-off hint ───────────────────────────────────
            gtk::Label {
                add_css_class: "label-small",
                set_label: "Enable Bluetooth to see devices",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                #[watch]
                set_visible: model.available && !model.enabled,
            },

            // ── Empty-state ──────────────────────────────────────
            gtk::Label {
                add_css_class: "label-small",
                set_label: "Scanning for devices…",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                #[watch]
                set_visible: model.available && model.enabled && model.devices.is_empty(),
            },

            // ── Device list (rebuilt imperatively) ───────────────
            #[name = "device_list_box"]
            gtk::Box {
                add_css_class: "bluetooth-dashboard-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                #[watch]
                set_visible: model.available && model.enabled && !model.devices.is_empty(),
            },
        }
    }

    fn init(
        _params: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let bt = bluetooth_service();

        spawn_bluetooth_enabled_watcher(&sender, || {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged
        });
        spawn_bluetooth_devices_watcher(&sender, || {
            BluetoothMenuWidgetCommandOutput::BluetoothDevicesChanged
        });

        let model = BluetoothMenuWidgetModel {
            available: bt.available.get(),
            enabled: bt.enabled.get(),
            devices: bt.devices.get(),
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged
            | BluetoothMenuWidgetCommandOutput::BluetoothDevicesChanged => {
                let bt = bluetooth_service();
                self.available = bt.available.get();
                self.enabled = bt.enabled.get();
                self.devices = bt.devices.get();
                sender.input(BluetoothMenuWidgetInput::RefreshState);
            }
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            // ── Power ────────────────────────────────────────────
            BluetoothMenuWidgetInput::SetEnabled(enabled) => {
                let bt = bluetooth_service();
                if enabled {
                    tokio::spawn(async move {
                        let _ = bt.enable().await;
                    });
                } else {
                    tokio::spawn(async move {
                        let _ = bt.disable().await;
                    });
                }
            }

            // ── Connect / disconnect ─────────────────────────────
            BluetoothMenuWidgetInput::ConnectToggle(address) => {
                if let Some(device) = self.find_device(&address) {
                    if device.connected.get() {
                        tokio::spawn(async move {
                            let _ = device.disconnect().await;
                        });
                    } else {
                        tokio::spawn(async move {
                            let _ = device.connect().await;
                        });
                    }
                }
            }

            // ── Pair ─────────────────────────────────────────────
            BluetoothMenuWidgetInput::Pair(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.pair().await;
                    });
                }
            }

            // ── Forget ───────────────────────────────────────────
            BluetoothMenuWidgetInput::Forget(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.forget().await;
                    });
                }
            }

            // ── Trust ────────────────────────────────────────────
            BluetoothMenuWidgetInput::SetTrusted(address, trusted) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.set_trusted(trusted).await;
                    });
                }
            }

            // ── Discovery routing (frame drives this) ────────────
            BluetoothMenuWidgetInput::ParentRevealChanged(revealed) => {
                let bt = bluetooth_service();
                if revealed {
                    tokio::spawn(async move {
                        let _ = bt.start_discovery().await;
                    });
                } else {
                    tokio::spawn(async move {
                        let _ = bt.stop_discovery().await;
                    });
                }
            }

            // ── State refresh (fired by update_cmd) ──────────────
            BluetoothMenuWidgetInput::RefreshState => {
                // Model already updated in update_cmd; list rebuild below.
            }
        }

        // Rebuild device list every message cycle.
        Self::rebuild_device_list(&widgets.device_list_box, &self.devices, &sender);

        self.update_view(widgets, sender);
    }
}

impl BluetoothMenuWidgetModel {
    fn find_device(&self, address: &str) -> Option<Arc<Device>> {
        self.devices
            .iter()
            .find(|d| d.address.get() == address)
            .cloned()
    }

    /// Rebuild the flat device list.  Full-wipe + repopulate; the list
    /// is short (< ~20 entries) so this is fine without a factory.
    fn rebuild_device_list(
        list_box: &gtk::Box,
        devices: &[Arc<Device>],
        sender: &ComponentSender<BluetoothMenuWidgetModel>,
    ) {
        use relm4::gtk::prelude::*;

        // Clear existing children.
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        let mut paired: Vec<&Arc<Device>> =
            devices.iter().filter(|d| d.paired.get()).collect();
        let mut unpaired: Vec<&Arc<Device>> =
            devices.iter().filter(|d| !d.paired.get()).collect();

        paired.sort_by_key(|d| d.alias.get());
        unpaired.sort_by_key(|d| d.alias.get());

        for device in paired.iter().chain(unpaired.iter()) {
            list_box.append(&Self::build_device_row(device, sender));
        }
    }

    /// Build one flat dashboard-style device row.
    fn build_device_row(
        device: &Arc<Device>,
        sender: &ComponentSender<BluetoothMenuWidgetModel>,
    ) -> gtk::Box {
        use relm4::gtk::prelude::*;

        let address = device.address.get();
        let alias = device.alias.get();
        let connected = device.connected.get();
        let paired = device.paired.get();
        let trusted = device.trusted.get();
        let battery = device.battery_percentage.get();
        let icon_name = get_bluetooth_device_icon(device.clone());

        // ── Outer row — clickable flat surface ───────────────────
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("bluetooth-dashboard-device-row");
        row.set_cursor_from_name(Some("pointer"));

        // Device icon
        let icon = gtk::Image::from_icon_name(&icon_name);
        icon.add_css_class("bluetooth-dashboard-icon");
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        // Alias label (hexpand)
        let name_label = gtk::Label::new(Some(&alias));
        name_label.set_halign(gtk::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_valign(gtk::Align::Center);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name_label.set_max_width_chars(20);
        row.append(&name_label);

        // Battery % (if available)
        if let Some(pct) = battery {
            let bat_label = gtk::Label::new(Some(&format!("{}%", pct)));
            bat_label.add_css_class("bluetooth-dashboard-value");
            bat_label.set_valign(gtk::Align::Center);
            row.append(&bat_label);
        }

        // Connected check accent
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.add_css_class("bluetooth-dashboard-device-check");
        check.set_valign(gtk::Align::Center);
        check.set_visible(connected);
        row.append(&check);

        // ── Secondary actions box (Forget + Trust, compact) ──────
        if paired {
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            actions.set_valign(gtk::Align::Center);

            // Forget
            let addr_f = address.clone();
            let sender_f = sender.clone();
            let forget_btn = gtk::Button::with_label("Forget");
            forget_btn.add_css_class("bluetooth-dashboard-action-button");
            forget_btn.connect_clicked(move |_| {
                sender_f.input(BluetoothMenuWidgetInput::Forget(addr_f.clone()));
            });
            actions.append(&forget_btn);

            // Trust / Untrust
            let trust_label = if trusted { "Untrust" } else { "Trust" };
            let addr_t = address.clone();
            let sender_t = sender.clone();
            let trust_btn = gtk::Button::with_label(trust_label);
            trust_btn.add_css_class("bluetooth-dashboard-action-button");
            trust_btn.connect_clicked(move |_| {
                sender_t
                    .input(BluetoothMenuWidgetInput::SetTrusted(addr_t.clone(), !trusted));
            });
            actions.append(&trust_btn);

            row.append(&actions);
        }

        // ── Row click → connect/disconnect/pair ──────────────────
        let addr_click = address.clone();
        let sender_click = sender.clone();
        let gesture = gtk::GestureClick::new();
        gesture.connect_pressed(move |_, _, _, _| {
            if paired {
                sender_click
                    .input(BluetoothMenuWidgetInput::ConnectToggle(addr_click.clone()));
            } else {
                sender_click.input(BluetoothMenuWidgetInput::Pair(addr_click.clone()));
            }
        });
        row.add_controller(gesture);

        row
    }
}
