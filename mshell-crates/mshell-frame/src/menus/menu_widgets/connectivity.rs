//! Dashboard "Connectivity" tile — compact WiFi + Bluetooth row.
//!
//! Replaces the standalone Network + Bluetooth widget stack with
//! a single horizontal status row:
//!
//!   📶 Ken_5            🅱 SL4P
//!
//! Two slots split 50/50, each showing icon + truncated label.
//! Informational only — taps don't open a sub-menu; the user has
//! the full Network / Bluetooth menus available from the bar
//! pills if they want to act.
//!
//! Data plumbed from the same wayle services the existing
//! widgets use, so a state change there propagates here for free.

use mshell_common::WatcherToken;
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    set_bluetooth_icon, set_bluetooth_label, spawn_bluetooth_device_watcher,
    spawn_bluetooth_devices_watcher, spawn_bluetooth_enabled_watcher,
};
use mshell_utils::network::{set_network_icon, set_network_label, spawn_network_watcher};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ConnectivityModel {
    /// Per-device connection-state watchers — re-spawned every
    /// time the device list changes (paired / unpaired). Without
    /// these we'd miss individual `connected` flips entirely:
    /// `spawn_bluetooth_devices_watcher` only fires on list
    /// add/remove, not on a paired device toggling its connection.
    bt_device_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum ConnectivityInput {}

#[derive(Debug)]
pub(crate) enum ConnectivityOutput {}

pub(crate) struct ConnectivityInit {}

#[derive(Debug)]
pub(crate) enum ConnectivityCommandOutput {
    NetworkChanged,
    /// Adapter on/off OR device list changed — refresh + respawn
    /// the per-device connection watchers.
    BluetoothStatusChanged,
    /// Some paired device flipped its `connected` flag.
    BluetoothConnectionChanged,
}

#[relm4::component(pub)]
impl Component for ConnectivityModel {
    type CommandOutput = ConnectivityCommandOutput;
    type Input = ConnectivityInput;
    type Output = ConnectivityOutput;
    type Init = ConnectivityInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "connectivity-menu-widget",
            set_orientation: gtk::Orientation::Horizontal,
            set_hexpand: true,
            set_spacing: 12,
            set_homogeneous: true,

            // ── WiFi cell ───────────────────────────────────────
            gtk::Box {
                add_css_class: "connectivity-cell",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,

                #[name = "wifi_image"]
                gtk::Image {
                    add_css_class: "connectivity-icon",
                },
                #[name = "wifi_label"]
                gtk::Label {
                    add_css_class: "connectivity-label",
                    set_ellipsize: relm4::gtk::pango::EllipsizeMode::End,
                    set_max_width_chars: 16,
                    set_xalign: 0.0,
                },
            },

            // ── Bluetooth cell ──────────────────────────────────
            gtk::Box {
                add_css_class: "connectivity-cell",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,

                #[name = "bt_image"]
                gtk::Image {
                    add_css_class: "connectivity-icon",
                },
                #[name = "bt_label"]
                gtk::Label {
                    add_css_class: "connectivity-label",
                    set_ellipsize: relm4::gtk::pango::EllipsizeMode::End,
                    set_max_width_chars: 16,
                    set_xalign: 0.0,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_network_watcher(
            &sender,
            || ConnectivityCommandOutput::NetworkChanged,
            || ConnectivityCommandOutput::NetworkChanged,
            || ConnectivityCommandOutput::NetworkChanged,
        );
        spawn_bluetooth_enabled_watcher(&sender, || {
            ConnectivityCommandOutput::BluetoothStatusChanged
        });
        spawn_bluetooth_devices_watcher(&sender, || {
            ConnectivityCommandOutput::BluetoothStatusChanged
        });

        let mut model = ConnectivityModel {
            bt_device_token: WatcherToken::new(),
        };
        let widgets = view_output!();

        apply_network(&widgets);
        apply_bluetooth(&widgets);

        // Initial per-device connection watchers — see model
        // doc-comment for why this matters.
        let token = model.bt_device_token.reset();
        if let Some(bt) = bluetooth_service() {
            for device in bt.devices.get() {
                spawn_bluetooth_device_watcher(&device, token.clone(), &sender, || {
                    ConnectivityCommandOutput::BluetoothConnectionChanged
                });
            }
        }

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ConnectivityCommandOutput::NetworkChanged => apply_network(widgets),
            ConnectivityCommandOutput::BluetoothStatusChanged => {
                apply_bluetooth(widgets);
                // Device list may have grown / shrunk — recycle
                // the per-device watchers against the new set.
                let token = self.bt_device_token.reset();
                if let Some(bt) = bluetooth_service() {
                    for device in bt.devices.get() {
                        spawn_bluetooth_device_watcher(&device, token.clone(), &sender, || {
                            ConnectivityCommandOutput::BluetoothConnectionChanged
                        });
                    }
                }
            }
            ConnectivityCommandOutput::BluetoothConnectionChanged => apply_bluetooth(widgets),
        }
    }
}

fn apply_network(widgets: &ConnectivityModelWidgets) {
    set_network_icon(&widgets.wifi_image);
    set_network_label(&widgets.wifi_label);
}

fn apply_bluetooth(widgets: &ConnectivityModelWidgets) {
    set_bluetooth_icon(&widgets.bt_image);
    // Bluetooth-only watcher fires when devices change but not on
    // the radio's available/enabled flips — the bar pill subscribes
    // to those too, but here we accept the limitation: clicking
    // through to the BT menu refreshes once the user acts.
    set_bluetooth_label(&widgets.bt_label);
    // Touch the service so the closure-captured Arcs stay alive (no-op when
    // there's no Bluetooth adapter).
    let _ = bluetooth_service().map(|b| b.enabled.get());
}
