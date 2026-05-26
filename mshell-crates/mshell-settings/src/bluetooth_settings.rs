use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    get_bluetooth_device_icon, spawn_bluetooth_devices_watcher, spawn_bluetooth_enabled_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

#[derive(Debug, Clone)]
pub(crate) struct BluetoothSettingsModel {
    available: bool,
    enabled: bool,
    devices: Vec<Arc<Device>>,
}

#[derive(Debug)]
pub(crate) enum BluetoothSettingsInput {
    SetEnabled(bool),
    Connect(String),
    Disconnect(String),
    Pair(String),
    Forget(String),
    SetTrusted(String, bool),
    /// Stub: allows external callers to drive discovery start/stop.
    /// Currently driven by the root widget's map/unmap signals; kept
    /// for future routing-path wiring.
    #[allow(dead_code)]
    ParentRevealChanged(bool),
    /// Internal: re-read service state into model after a watcher fires.
    RefreshState,
}

#[derive(Debug)]
pub(crate) enum BluetoothSettingsOutput {}

pub(crate) struct BluetoothSettingsInit {}

#[derive(Debug)]
pub(crate) enum BluetoothSettingsCommandOutput {
    StateChanged,
    DevicesChanged,
}

#[relm4::component(pub)]
impl Component for BluetoothSettingsModel {
    type CommandOutput = BluetoothSettingsCommandOutput;
    type Input = BluetoothSettingsInput;
    type Output = BluetoothSettingsOutput;
    type Init = BluetoothSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                // ── Hero header ──────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("bluetooth-active-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Bluetooth",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Manage Bluetooth power and nearby devices — pair, connect, or forget.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Hardware-missing banner (hidden when available) ──
                gtk::Box {
                    add_css_class: "bt-hardware-missing",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: !model.available,

                    gtk::Image {
                        set_icon_name: Some("bluetooth-hardware-disabled-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Bluetooth hardware missing — no adapter detected.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // ── Power toggle ─────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Power",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.available,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.available,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Enabled",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Power the Bluetooth adapter on or off.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(bt_enabled_handler)]
                        set_active: model.enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BluetoothSettingsInput::SetEnabled(enabled));
                            glib::Propagation::Proceed
                        } @bt_enabled_handler,
                    },
                },

                // ── Device list ──────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Devices",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.available && model.enabled,
                },

                // Radio-off state: enabled=false, show a hint
                gtk::Box {
                    add_css_class: "bt-radio-off",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.available && !model.enabled,

                    gtk::Image {
                        set_icon_name: Some("bluetooth-disabled-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Enable Bluetooth above to see and manage devices.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // Empty-state: enabled but no devices
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "No devices found — discovery is running while this page is open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_visible: model.available && model.enabled && model.devices.is_empty(),
                },

                // Device list container (rebuilt in update_with_view)
                #[name = "device_list_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    #[watch]
                    set_visible: model.available && model.enabled && !model.devices.is_empty(),
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let bt = bluetooth_service();

        spawn_bluetooth_enabled_watcher(&sender, || BluetoothSettingsCommandOutput::StateChanged);
        spawn_bluetooth_devices_watcher(&sender, || BluetoothSettingsCommandOutput::DevicesChanged);

        let model = BluetoothSettingsModel {
            available: bt.available.get(),
            enabled: bt.enabled.get(),
            devices: bt.devices.get(),
        };

        let widgets = view_output!();

        // Start/stop discovery based on page visibility (map = shown, unmap = hidden).
        // This gates scanning to only when this settings page is the visible child
        // in the stack — no need for a separate routing path.
        {
            let root_widget = root.clone();
            root_widget.connect_map(|_| {
                let bt = bluetooth_service();
                tokio::spawn(async move {
                    let _ = bt.start_discovery().await;
                });
            });
            root_widget.connect_unmap(|_| {
                let bt = bluetooth_service();
                tokio::spawn(async move {
                    let _ = bt.stop_discovery().await;
                });
            });
        }

        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BluetoothSettingsCommandOutput::StateChanged
            | BluetoothSettingsCommandOutput::DevicesChanged => {
                let bt = bluetooth_service();
                self.available = bt.available.get();
                self.enabled = bt.enabled.get();
                self.devices = bt.devices.get();
                // Trigger a view rebuild (update_with_view will be called via
                // a RefreshState input so the device_list_box is repopulated).
                sender.input(BluetoothSettingsInput::RefreshState);
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
            // ── Power toggle ─────────────────────────────────────
            BluetoothSettingsInput::SetEnabled(enabled) => {
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

            // ── Device actions ───────────────────────────────────
            BluetoothSettingsInput::Connect(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.connect().await;
                    });
                }
            }
            BluetoothSettingsInput::Disconnect(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.disconnect().await;
                    });
                }
            }
            BluetoothSettingsInput::Pair(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.pair().await;
                    });
                }
            }
            BluetoothSettingsInput::Forget(address) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.forget().await;
                    });
                }
            }
            BluetoothSettingsInput::SetTrusted(address, trusted) => {
                if let Some(device) = self.find_device(&address) {
                    tokio::spawn(async move {
                        let _ = device.set_trusted(trusted).await;
                    });
                }
            }

            // ── Discovery routing (map/unmap drives this; kept for API completeness) ──
            BluetoothSettingsInput::ParentRevealChanged(revealed) => {
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

            // ── State refresh (triggered by update_cmd) ───────────
            BluetoothSettingsInput::RefreshState => {
                // Model already updated in update_cmd; rebuild device list below.
            }
        }

        // Rebuild the device list box from the current model every time we
        // process a message that could have changed it.
        Self::rebuild_device_list(&widgets.device_list_box, &self.devices, &sender);

        self.update_view(widgets, sender);
    }
}

impl BluetoothSettingsModel {
    /// Look up a device by address string.
    fn find_device(&self, address: &str) -> Option<Arc<Device>> {
        self.devices
            .iter()
            .find(|d| d.address.get() == address)
            .cloned()
    }

    /// Rebuild the device list gtk::Box.
    ///
    /// Called on every update so the list stays in sync with the reactive
    /// model without needing a separate factory/DynamicBox.  The device list
    /// in the Settings page is expected to be short (< ~20 entries), so a
    /// full rebuild on each change is fine — no virtualization needed.
    fn rebuild_device_list(
        list_box: &gtk::Box,
        devices: &[Arc<Device>],
        sender: &ComponentSender<BluetoothSettingsModel>,
    ) {
        use relm4::gtk::prelude::*;

        // Clear existing children
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        let mut paired: Vec<&Arc<Device>> = devices.iter().filter(|d| d.paired.get()).collect();
        let mut unpaired: Vec<&Arc<Device>> = devices.iter().filter(|d| !d.paired.get()).collect();

        // Sort by alias for stable ordering
        paired.sort_by_key(|d| d.alias.get());
        unpaired.sort_by_key(|d| d.alias.get());

        if !paired.is_empty() {
            let section_label = gtk::Label::new(Some("Paired Devices"));
            section_label.add_css_class("label-medium-bold");
            section_label.set_halign(gtk::Align::Start);
            list_box.append(&section_label);

            for device in &paired {
                list_box.append(&Self::build_device_row(device, sender));
            }
        }

        if !unpaired.is_empty() {
            let section_label = gtk::Label::new(Some("Nearby Devices"));
            section_label.add_css_class("label-medium-bold");
            section_label.set_halign(gtk::Align::Start);
            list_box.append(&section_label);

            for device in &unpaired {
                list_box.append(&Self::build_device_row(device, sender));
            }
        }
    }

    /// Build a single device row widget.
    fn build_device_row(
        device: &Arc<Device>,
        sender: &ComponentSender<BluetoothSettingsModel>,
    ) -> gtk::Box {
        use relm4::gtk::prelude::*;

        let address = device.address.get();
        let alias = device.alias.get();
        let connected = device.connected.get();
        let paired = device.paired.get();
        let trusted = device.trusted.get();
        let battery = device.battery_percentage.get();
        let icon_name = get_bluetooth_device_icon(device.clone());

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("bt-device-row");

        // Device icon
        let icon = gtk::Image::from_icon_name(&icon_name);
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        // Device info (alias + status)
        let info_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk::Align::Center);

        let name_label = gtk::Label::new(Some(&alias));
        name_label.add_css_class("label-medium-bold");
        name_label.set_halign(gtk::Align::Start);
        info_box.append(&name_label);

        // Status string
        let status_parts: Vec<&str> = [
            if connected { Some("Connected") } else { None },
            if paired { Some("Paired") } else { None },
            if trusted { Some("Trusted") } else { None },
        ]
        .into_iter()
        .flatten()
        .collect();
        let status = if status_parts.is_empty() {
            "Not Paired".to_string()
        } else {
            status_parts.join(" · ")
        };

        let status_label = gtk::Label::new(Some(&status));
        status_label.add_css_class("label-small");
        status_label.set_halign(gtk::Align::Start);
        info_box.append(&status_label);

        // Battery percentage (if available)
        if let Some(pct) = battery {
            let bat_label = gtk::Label::new(Some(&format!("{}%", pct)));
            bat_label.add_css_class("label-small");
            bat_label.set_halign(gtk::Align::Start);
            info_box.append(&bat_label);
        }

        row.append(&info_box);

        // Action buttons
        let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        actions_box.set_valign(gtk::Align::Center);

        if paired && !connected {
            let addr = address.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Connect");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(BluetoothSettingsInput::Connect(addr.clone()));
            });
            actions_box.append(&btn);
        }

        if paired && connected {
            let addr = address.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Disconnect");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(BluetoothSettingsInput::Disconnect(addr.clone()));
            });
            actions_box.append(&btn);
        }

        if !paired {
            let addr = address.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Pair");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(BluetoothSettingsInput::Pair(addr.clone()));
            });
            actions_box.append(&btn);
        }

        if paired {
            let addr = address.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Forget");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(BluetoothSettingsInput::Forget(addr.clone()));
            });
            actions_box.append(&btn);

            // Trust toggle
            let trust_label = if trusted { "Untrust" } else { "Trust" };
            let addr = address.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label(trust_label);
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(BluetoothSettingsInput::SetTrusted(addr.clone(), !trusted));
            });
            actions_box.append(&btn);
        }

        row.append(&actions_box);

        row
    }
}
