//! Bluetooth menu widget — dashboard-style layout matching the
//! Audio Dashboard menu visual language. Replaces the previous
//! revealer-row implementation.
//!
//! Sections:
//!   - Header: adapter on/off + scan toggle
//!   - Connected devices (active = primary tint, "Disconnect" button)
//!   - Paired but disconnected (with "Connect" button)
//!   - Available / discovered (with "Pair" button, only when scanning)

use mshell_common::WatcherToken;
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    spawn_bluetooth_device_watcher, spawn_bluetooth_devices_watcher,
    spawn_bluetooth_enabled_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

struct DeviceRow {
    container: gtk::Box,
    device: Arc<Device>,
}

pub(crate) struct BluetoothMenuWidgetModel {
    enabled: bool,
    discovering: bool,
    connected_rows: Vec<DeviceRow>,
    paired_rows: Vec<DeviceRow>,
    available_rows: Vec<DeviceRow>,
    /// Re-spawnable per-device watchers so a connected flip
    /// repaints the section a device belongs to.
    device_watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetInput {
    Refresh,
    ToggleAdapter,
    ToggleScan,
    ConnectDevice(Arc<Device>),
    DisconnectDevice(Arc<Device>),
}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetOutput {}

pub(crate) struct BluetoothMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum BluetoothMenuWidgetCommandOutput {
    BluetoothStateChanged,
}

#[relm4::component(pub)]
impl Component for BluetoothMenuWidgetModel {
    type CommandOutput = BluetoothMenuWidgetCommandOutput;
    type Input = BluetoothMenuWidgetInput;
    type Output = BluetoothMenuWidgetOutput;
    type Init = BluetoothMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "bluetooth-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,
            set_margin_all: 14,

            // ── Header: adapter toggle ──────────────────────
            gtk::Box {
                add_css_class: "bluetooth-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                gtk::Label {
                    add_css_class: "bluetooth-title",
                    set_label: "BLUETOOTH",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },
                gtk::Button {
                    add_css_class: "bluetooth-toggle",
                    #[watch]
                    set_label: if model.enabled { "On" } else { "Off" },
                    #[watch]
                    set_css_classes: &[
                        "bluetooth-toggle",
                        if model.enabled { "active" } else { "calm" },
                    ],
                    connect_clicked[sender] => move |_| {
                        sender.input(BluetoothMenuWidgetInput::ToggleAdapter);
                    },
                },
            },

            // ── Connected section ───────────────────────────
            gtk::Label {
                add_css_class: "bluetooth-section-label",
                set_label: "CONNECTED",
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: !model.connected_rows.is_empty(),
            },
            #[name = "connected_box"]
            gtk::Box {
                add_css_class: "bluetooth-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
            },

            // ── Paired section ──────────────────────────────
            gtk::Label {
                add_css_class: "bluetooth-section-label",
                set_label: "PAIRED",
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: !model.paired_rows.is_empty(),
            },
            #[name = "paired_box"]
            gtk::Box {
                add_css_class: "bluetooth-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
            },

            // ── Available section + scan toggle ──────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                gtk::Label {
                    add_css_class: "bluetooth-section-label",
                    set_label: "AVAILABLE",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },
                gtk::Button {
                    #[watch]
                    set_label: if model.discovering { "Stop Scan" } else { "Scan" },
                    #[watch]
                    set_css_classes: &[
                        "bluetooth-scan",
                        if model.discovering { "active" } else { "calm" },
                    ],
                    #[watch]
                    set_sensitive: model.enabled,
                    connect_clicked[sender] => move |_| {
                        sender.input(BluetoothMenuWidgetInput::ToggleScan);
                    },
                },
            },
            #[name = "available_box"]
            gtk::Box {
                add_css_class: "bluetooth-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
            },

            gtk::Label {
                add_css_class: "bluetooth-empty",
                #[watch]
                set_visible: model.connected_rows.is_empty()
                    && model.paired_rows.is_empty()
                    && model.available_rows.is_empty(),
                #[watch]
                set_label: if !model.enabled {
                    "Bluetooth is off"
                } else if model.discovering {
                    "Scanning…"
                } else {
                    "Press Scan to discover devices"
                },
                set_halign: gtk::Align::Center,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_bluetooth_enabled_watcher(&sender, || {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged
        });
        spawn_bluetooth_devices_watcher(&sender, || {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged
        });

        let svc = bluetooth_service();
        let enabled = svc.enabled.get();
        let discovering = svc
            .primary_adapter
            .get()
            .as_ref()
            .map(|a| a.discovering.get())
            .unwrap_or(false);

        let model = BluetoothMenuWidgetModel {
            enabled,
            discovering,
            connected_rows: Vec::new(),
            paired_rows: Vec::new(),
            available_rows: Vec::new(),
            device_watcher_token: WatcherToken::new(),
        };

        let widgets = view_output!();

        let _ = root;
        let mut parts = ComponentParts { model, widgets };
        rebuild_all(&mut parts.model, &parts.widgets, &sender);
        parts
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged => {
                sender.input(BluetoothMenuWidgetInput::Refresh);
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
            BluetoothMenuWidgetInput::Refresh => {
                let svc = bluetooth_service();
                self.enabled = svc.enabled.get();
                self.discovering = svc
                    .primary_adapter
                    .get()
                    .as_ref()
                    .map(|a| a.discovering.get())
                    .unwrap_or(false);
                rebuild_all(self, widgets, &sender);
            }
            BluetoothMenuWidgetInput::ToggleAdapter => {
                let target = !self.enabled;
                glib::spawn_future_local(async move {
                    let svc = bluetooth_service();
                    let _ = if target {
                        svc.enable().await
                    } else {
                        svc.disable().await
                    };
                });
            }
            BluetoothMenuWidgetInput::ToggleScan => {
                let stop = self.discovering;
                glib::spawn_future_local(async move {
                    let svc = bluetooth_service();
                    let _ = if stop {
                        svc.stop_discovery().await
                    } else {
                        svc.start_discovery().await
                    };
                });
            }
            BluetoothMenuWidgetInput::ConnectDevice(d) => {
                glib::spawn_future_local(async move {
                    let _ = d.connect().await;
                });
            }
            BluetoothMenuWidgetInput::DisconnectDevice(d) => {
                glib::spawn_future_local(async move {
                    let _ = d.disconnect().await;
                });
            }
        }
        self.update_view(widgets, sender);
    }
}

fn rebuild_all(
    model: &mut BluetoothMenuWidgetModel,
    widgets: &BluetoothMenuWidgetModelWidgets,
    sender: &ComponentSender<BluetoothMenuWidgetModel>,
) {
    for row in model.connected_rows.drain(..) {
        widgets.connected_box.remove(&row.container);
    }
    for row in model.paired_rows.drain(..) {
        widgets.paired_box.remove(&row.container);
    }
    for row in model.available_rows.drain(..) {
        widgets.available_box.remove(&row.container);
    }

    let devices = bluetooth_service().devices.get();

    // Re-spawn per-device connection watchers — without them the
    // .connected flip on a paired device wouldn't move it from
    // PAIRED to CONNECTED until the next adapter event.
    let token = model.device_watcher_token.reset();
    for device in devices.iter() {
        spawn_bluetooth_device_watcher(device, token.clone(), sender, || {
            BluetoothMenuWidgetCommandOutput::BluetoothStateChanged
        });
    }

    for device in devices.iter() {
        let connected = device.connected.get();
        let paired = device.paired.get();
        let (target_box, target_rows, kind) = if connected {
            (&widgets.connected_box, &mut model.connected_rows, RowKind::Connected)
        } else if paired {
            (&widgets.paired_box, &mut model.paired_rows, RowKind::Paired)
        } else {
            (&widgets.available_box, &mut model.available_rows, RowKind::Available)
        };
        let row = build_row(device.clone(), kind, sender);
        target_box.append(&row.container);
        target_rows.push(row);
    }
}

#[derive(Clone, Copy)]
enum RowKind {
    Connected,
    Paired,
    Available,
}

fn build_row(
    device: Arc<Device>,
    kind: RowKind,
    sender: &ComponentSender<BluetoothMenuWidgetModel>,
) -> DeviceRow {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    container.add_css_class("bluetooth-device-row");
    if matches!(kind, RowKind::Connected) {
        container.add_css_class("active");
    }

    let icon = gtk::Image::from_icon_name(
        device
            .icon
            .get()
            .map(|s| format!("{}-symbolic", s))
            .as_deref()
            .unwrap_or("bluetooth-active-symbolic"),
    );
    icon.add_css_class("bluetooth-device-icon");
    container.append(&icon);

    let alias = device.alias.get();
    let label_text = if alias.is_empty() {
        "Unknown device".to_string()
    } else {
        alias
    };
    let label = gtk::Label::new(Some(&label_text));
    label.add_css_class("bluetooth-device-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    container.append(&label);

    let action = gtk::Button::new();
    action.add_css_class("bluetooth-device-action");
    match kind {
        RowKind::Connected => {
            action.set_label("Disconnect");
            let d = device.clone();
            let s = sender.clone();
            action.connect_clicked(move |_| {
                s.input(BluetoothMenuWidgetInput::DisconnectDevice(d.clone()));
            });
        }
        RowKind::Paired => {
            action.set_label("Connect");
            let d = device.clone();
            let s = sender.clone();
            action.connect_clicked(move |_| {
                s.input(BluetoothMenuWidgetInput::ConnectDevice(d.clone()));
            });
        }
        RowKind::Available => {
            action.set_label("Pair");
            let d = device.clone();
            let s = sender.clone();
            // Pair = connect on first attempt; wayle's connect()
            // triggers pairing if needed.
            action.connect_clicked(move |_| {
                s.input(BluetoothMenuWidgetInput::ConnectDevice(d.clone()));
            });
        }
    }
    container.append(&action);

    DeviceRow { container, device }
}
