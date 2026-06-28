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

use mshell_common::WatcherToken;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{BluetoothDevice, ConfigStoreFields};
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    get_bluetooth_device_icon, spawn_bluetooth_device_battery_watcher,
    spawn_bluetooth_device_watcher, spawn_bluetooth_devices_watcher,
    spawn_bluetooth_enabled_watcher,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

#[derive(Debug)]
pub(crate) struct BluetoothMenuWidgetModel {
    available: bool,
    enabled: bool,
    devices: Vec<Arc<Device>>,
    /// Cancels the previous per-device battery/state watchers when the
    /// device list changes.
    device_watcher_token: WatcherToken,
    /// Addresses with a connect/disconnect in flight → spinner on the row
    /// (§16 loading). Maps address → the connected state we're waiting for,
    /// so the spinner clears exactly when the op settles.
    busy: HashMap<String, bool>,
    /// MACs (uppercased) configured for login auto-connect — drives the
    /// ★ pin on each row. Snapshotted from config, refreshed on edits.
    autoconnect_macs: Vec<String>,
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
    /// Internal: add/remove a device from the login auto-connect list
    /// (MAC, friendly name).
    TogglePin(String, String),
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
    /// A device's battery / connected / paired / trusted property changed.
    DeviceStateChanged,
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

        let mut model = BluetoothMenuWidgetModel {
            available: bt.as_ref().map(|b| b.available.get()).unwrap_or(false),
            enabled: bt.as_ref().map(|b| b.enabled.get()).unwrap_or(false),
            devices: bt.as_ref().map(|b| b.devices.get()).unwrap_or_default(),
            device_watcher_token: WatcherToken::new(),
            busy: HashMap::new(),
            autoconnect_macs: read_autoconnect_macs(),
        };

        let widgets = view_output!();

        // Subscribe to each device's battery + state so the rows repaint when
        // BlueZ fills the battery in (it arrives async after connect).
        model.spawn_device_watchers(&sender);

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
                self.available = bt.as_ref().map(|b| b.available.get()).unwrap_or(false);
                self.enabled = bt.as_ref().map(|b| b.enabled.get()).unwrap_or(false);
                self.devices = bt.as_ref().map(|b| b.devices.get()).unwrap_or_default();
                // Device set may have changed — re-subscribe per-device watchers.
                self.spawn_device_watchers(&sender);
                sender.input(BluetoothMenuWidgetInput::RefreshState);
            }
            // A device's battery/connected/etc. changed in place — repaint the
            // rows (same Arcs, fresh property values via .get()).
            BluetoothMenuWidgetCommandOutput::DeviceStateChanged => {
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
                if let Some(bt) = bluetooth_service() {
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
            }

            // ── Connect / disconnect ─────────────────────────────
            BluetoothMenuWidgetInput::ConnectToggle(address) => {
                if let Some(device) = self.find_device(&address) {
                    let was_connected = device.connected.get();
                    // Spinner until the device reports the flipped state.
                    self.busy.insert(address.clone(), !was_connected);
                    if was_connected {
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
                if let Some(bt) = bluetooth_service() {
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
            }

            // ── Auto-connect pin ─────────────────────────────────
            BluetoothMenuWidgetInput::TogglePin(mac, name) => {
                toggle_autoconnect(&mac, &name);
                self.autoconnect_macs = read_autoconnect_macs();
            }

            // ── State refresh (fired by update_cmd) ──────────────
            BluetoothMenuWidgetInput::RefreshState => {
                // Clear the spinner for any device that has reached the
                // state we were waiting for (or vanished from the list).
                let devices = &self.devices;
                self.busy.retain(|addr, want| {
                    matches!(
                        devices.iter().find(|d| d.address.get() == *addr),
                        Some(d) if d.connected.get() != *want
                    )
                });
            }
        }

        // Rebuild device list every message cycle.
        Self::rebuild_device_list(
            &widgets.device_list_box,
            &self.devices,
            &self.busy,
            &self.autoconnect_macs,
            &sender,
        );

        self.update_view(widgets, sender);
    }
}

impl BluetoothMenuWidgetModel {
    /// (Re)subscribe to every current device's battery + paired/connected/
    /// trusted properties, cancelling the previous round. BlueZ populates a
    /// device's battery asynchronously after it connects, so without these
    /// per-device watchers the list (rebuilt only on device-LIST changes)
    /// never repaints to show the battery %.
    fn spawn_device_watchers(&mut self, sender: &ComponentSender<Self>) {
        let token = self.device_watcher_token.reset();
        for device in &self.devices {
            spawn_bluetooth_device_battery_watcher(device, token.clone(), sender, || {
                BluetoothMenuWidgetCommandOutput::DeviceStateChanged
            });
            spawn_bluetooth_device_watcher(device, token.clone(), sender, || {
                BluetoothMenuWidgetCommandOutput::DeviceStateChanged
            });
        }
    }

    fn find_device(&self, address: &str) -> Option<Arc<Device>> {
        self.devices
            .iter()
            .find(|d| d.address.get() == address)
            .cloned()
    }

    /// Rebuild the flat device list.  Full-wipe + repopulate; the list
    /// is short (< ~20 entries) so this is fine without a factory.
    ///
    /// Devices are split into **Paired** and **Available** groups, each
    /// under a small section sub-label, so the menu scans the way the
    /// Settings page does. Connected devices sort to the top of Paired.
    fn rebuild_device_list(
        list_box: &gtk::Box,
        devices: &[Arc<Device>],
        busy: &HashMap<String, bool>,
        autoconnect_macs: &[String],
        sender: &ComponentSender<BluetoothMenuWidgetModel>,
    ) {
        use relm4::gtk::prelude::*;

        // Clear existing children.
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        let mut paired: Vec<&Arc<Device>> = devices.iter().filter(|d| d.paired.get()).collect();
        let mut unpaired: Vec<&Arc<Device>> = devices.iter().filter(|d| !d.paired.get()).collect();

        // Connected first, then alphabetical — the live device is what you
        // most often came here to act on.
        paired.sort_by_key(|d| (!d.connected.get(), d.alias.get()));
        unpaired.sort_by_key(|d| d.alias.get());

        let sub_label = |text: &str| {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("bluetooth-dashboard-subsection-label");
            l.set_halign(gtk::Align::Start);
            l
        };

        if !paired.is_empty() {
            // Only worth labelling the groups when both are present.
            if !unpaired.is_empty() {
                list_box.append(&sub_label("PAIRED"));
            }
            for device in &paired {
                list_box.append(&Self::build_device_row(
                    device,
                    busy,
                    autoconnect_macs,
                    sender,
                ));
            }
        }

        if !unpaired.is_empty() {
            if !paired.is_empty() {
                list_box.append(&sub_label("AVAILABLE"));
            }
            for device in &unpaired {
                list_box.append(&Self::build_device_row(
                    device,
                    busy,
                    autoconnect_macs,
                    sender,
                ));
            }
        }
    }

    /// Build one flat dashboard-style device row.
    fn build_device_row(
        device: &Arc<Device>,
        busy: &HashMap<String, bool>,
        autoconnect_macs: &[String],
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
        let in_flight = busy.contains_key(&address);
        let pinned = autoconnect_macs.contains(&address.to_ascii_uppercase());

        // ── Outer row — clickable flat surface ───────────────────
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("bluetooth-dashboard-device-row");
        if connected {
            row.add_css_class("connected");
        }
        row.set_cursor_from_name(Some("pointer"));

        // Device icon
        let icon = gtk::Image::from_icon_name(&icon_name);
        icon.add_css_class("bluetooth-dashboard-icon");
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        // Alias label (hexpand). Explicit class so the font size is set on
        // the label node itself — GTK4 does not reliably inherit font-size
        // from the row Box onto an unclassed child label.
        let name_label = gtk::Label::new(Some(&alias));
        name_label.add_css_class("bluetooth-dashboard-device-name");
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

        // ── Trailing status: spinner while an op is in flight (§16
        //    loading), else the primary-accent connected check.
        if in_flight {
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("bluetooth-dashboard-spinner");
            spinner.set_valign(gtk::Align::Center);
            spinner.start();
            row.append(&spinner);
        } else {
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.add_css_class("bluetooth-dashboard-device-check");
            check.set_valign(gtk::Align::Center);
            check.set_visible(connected);
            row.append(&check);
        }

        // ── Secondary actions box (Pin + Forget + Trust, compact) ─
        if paired {
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            actions.set_valign(gtk::Align::Center);

            // Auto-connect pin (★) — add/remove this device from the login
            // auto-connect list. Filled star when pinned.
            let pin_icon = if pinned {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            };
            let pin_btn = gtk::Button::from_icon_name(pin_icon);
            pin_btn.add_css_class("bluetooth-dashboard-pin-button");
            if pinned {
                pin_btn.add_css_class("pinned");
            }
            pin_btn.set_tooltip_text(Some(if pinned {
                "Auto-connects at login — click to stop"
            } else {
                "Auto-connect this device at login"
            }));
            {
                let addr_p = address.clone();
                let name_p = alias.clone();
                let sender_p = sender.clone();
                pin_btn.connect_clicked(move |_| {
                    sender_p.input(BluetoothMenuWidgetInput::TogglePin(
                        addr_p.clone(),
                        name_p.clone(),
                    ));
                });
            }
            actions.append(&pin_btn);

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
                sender_t.input(BluetoothMenuWidgetInput::SetTrusted(
                    addr_t.clone(),
                    !trusted,
                ));
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
                sender_click.input(BluetoothMenuWidgetInput::ConnectToggle(addr_click.clone()));
            } else {
                sender_click.input(BluetoothMenuWidgetInput::Pair(addr_click.clone()));
            }
        });
        row.add_controller(gesture);

        row
    }
}

/// Uppercased MACs currently configured for login auto-connect.
fn read_autoconnect_macs() -> Vec<String> {
    config_manager()
        .config()
        .bluetooth()
        .get_untracked()
        .devices
        .iter()
        .map(|d| d.mac.trim().to_ascii_uppercase())
        .collect()
}

/// Add the device to the auto-connect list, or remove it if already there.
/// Matching is by MAC (case-insensitive); `name` seeds a new entry's label.
fn toggle_autoconnect(mac: &str, name: &str) {
    let mac = mac.trim().to_string();
    let name = name.trim().to_string();
    config_manager().update_config(move |c| {
        if let Some(pos) = c
            .bluetooth
            .devices
            .iter()
            .position(|d| d.mac.eq_ignore_ascii_case(&mac))
        {
            c.bluetooth.devices.remove(pos);
        } else {
            c.bluetooth.devices.push(BluetoothDevice {
                mac: mac.clone(),
                name: name.clone(),
            });
        }
    });
}
