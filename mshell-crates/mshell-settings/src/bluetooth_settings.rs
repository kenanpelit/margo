use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{BluetoothConfig, BluetoothDevice, ConfigStoreFields};
use mshell_services::bluetooth_service;
use mshell_utils::bluetooth::{
    get_bluetooth_device_icon, spawn_bluetooth_devices_watcher, spawn_bluetooth_enabled_watcher,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

#[derive(Debug, Clone)]
pub(crate) struct BluetoothSettingsModel {
    available: bool,
    enabled: bool,
    devices: Vec<Arc<Device>>,
    /// Auto-connect + audio-routing config snapshot (Settings owns the
    /// edits; the engine in mshell-core reads it at login / on toggle).
    ac: BluetoothConfig,
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

    // ── Auto-connect config edits ───────────────────────────────
    /// Master "auto-connect at login" switch.
    AcToggleEnabled(bool),
    /// Post-startup connect delay, seconds.
    AcSetDelay(u32),
    /// Route the device as the default audio output on connect.
    AcToggleRouteOutput(bool),
    /// Also route it as the default mic (degrades A2DP → HSP/HFP).
    AcToggleRouteInput(bool),
    /// Add a device to the auto-connect list (MAC, friendly name).
    AcAddDevice(String, String),
    /// Remove a device from the auto-connect list (by MAC).
    AcRemoveDevice(String),
    /// Reorder a device by a signed step (drag grip).
    AcReorder(String, i32),
    /// Run the smart toggle now (same as `mshellctl bluetooth toggle`).
    AcToggleNow,
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
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[watch]
                    set_visible: model.available,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
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
                },

                // ── Auto-connect ─────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    #[watch]
                    set_visible: model.available,

                    gtk::Label {
                        add_css_class: "label-large-bold",
                        set_label: "Auto-connect",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Connect your headset automatically at login and route \
                                    audio to it — no scripts or services needed. Devices are \
                                    tried in order; the first that connects wins.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::Box {
                        add_css_class: "boxed-list",
                        set_orientation: gtk::Orientation::Vertical,

                        // Master switch
                        gtk::Box {
                            add_css_class: "action-row",
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 20,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Auto-connect at login",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            gtk::Switch {
                                set_valign: gtk::Align::Center,
                                set_active: model.ac.autoconnect_enabled,
                                connect_active_notify[sender] => move |sw| {
                                    sender.input(BluetoothSettingsInput::AcToggleEnabled(sw.is_active()));
                                },
                            },
                        },

                        // Delay
                        gtk::Box {
                            add_css_class: "action-row",
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 20,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Delay after login (seconds)",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            gtk::SpinButton {
                                set_valign: gtk::Align::Center,
                                set_digits: 0,
                                set_range: (0.0, 120.0),
                                set_increments: (1.0, 5.0),
                                set_value: model.ac.autoconnect_delay_secs as f64,
                                set_tooltip_text: Some("Wait this long before the first connect attempt"),
                                connect_value_changed[sender] => move |s| {
                                    sender.input(BluetoothSettingsInput::AcSetDelay(s.value().max(0.0) as u32));
                                },
                            },
                        },

                        // Route audio output
                        gtk::Box {
                            add_css_class: "action-row",
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 20,
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_valign: gtk::Align::Center,
                                set_hexpand: true,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_label: "Use as audio output",
                                    set_halign: gtk::Align::Start,
                                },
                                gtk::Label {
                                    add_css_class: "label-small",
                                    set_label: "Make it the default speaker when it connects.",
                                    set_halign: gtk::Align::Start,
                                    set_xalign: 0.0,
                                    set_wrap: true,
                                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                                },
                            },
                            gtk::Switch {
                                set_valign: gtk::Align::Center,
                                set_active: model.ac.route_audio_output,
                                connect_active_notify[sender] => move |sw| {
                                    sender.input(BluetoothSettingsInput::AcToggleRouteOutput(sw.is_active()));
                                },
                            },
                        },

                        // Route audio input (mic)
                        gtk::Box {
                            add_css_class: "action-row",
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 20,
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_valign: gtk::Align::Center,
                                set_hexpand: true,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_label: "Use as microphone",
                                    set_halign: gtk::Align::Start,
                                },
                                gtk::Label {
                                    add_css_class: "label-small",
                                    set_label: "Also set it as the default mic. Off by default: a \
                                                headset mic forces the low-quality HSP/HFP codec and \
                                                degrades playback.",
                                    set_halign: gtk::Align::Start,
                                    set_xalign: 0.0,
                                    set_wrap: true,
                                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                                },
                            },
                            gtk::Switch {
                                set_valign: gtk::Align::Center,
                                set_active: model.ac.route_audio_input,
                                connect_active_notify[sender] => move |sw| {
                                    sender.input(BluetoothSettingsInput::AcToggleRouteInput(sw.is_active()));
                                },
                            },
                        },
                    },

                    // Add a device by MAC
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        #[name = "ac_mac_entry"]
                        gtk::Entry {
                            set_hexpand: true,
                            set_placeholder_text: Some("MAC, e.g. F4:9D:8A:3D:CB:30"),
                        },
                        #[name = "ac_name_entry"]
                        gtk::Entry {
                            set_placeholder_text: Some("name (optional)"),
                        },
                        gtk::Button {
                            add_css_class: "ok-button-primary",
                            set_label: "Add",
                            connect_clicked[sender, ac_mac_entry, ac_name_entry] => move |_| {
                                sender.input(BluetoothSettingsInput::AcAddDevice(
                                    ac_mac_entry.text().to_string(),
                                    ac_name_entry.text().to_string(),
                                ));
                                ac_mac_entry.set_text("");
                                ac_name_entry.set_text("");
                            },
                        },
                    },

                    // The configured device list (rebuilt in update_with_view)
                    #[name = "ac_list_box"]
                    gtk::Box {
                        add_css_class: "boxed-list",
                        set_orientation: gtk::Orientation::Vertical,
                    },

                    // Toggle-now button
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_halign: gtk::Align::Start,
                        set_label: "Toggle now",
                        set_tooltip_text: Some("Connect, or disconnect if already connected (same as the keybind)"),
                        connect_clicked[sender] => move |_| {
                            sender.input(BluetoothSettingsInput::AcToggleNow);
                        },
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
            available: bt.as_ref().map(|b| b.available.get()).unwrap_or(false),
            enabled: bt.as_ref().map(|b| b.enabled.get()).unwrap_or(false),
            devices: bt.as_ref().map(|b| b.devices.get()).unwrap_or_default(),
            ac: config_manager().config().bluetooth().get_untracked(),
        };

        let widgets = view_output!();

        // Initial population of the auto-connect list (config-driven, so it
        // isn't filled by the live-device watchers).
        Self::rebuild_ac_list(&widgets.ac_list_box, &model.ac.devices, &sender);

        // Start/stop discovery based on page visibility (map = shown, unmap = hidden).
        // This gates scanning to only when this settings page is the visible child
        // in the stack — no need for a separate routing path.
        {
            let root_widget = root.clone();
            root_widget.connect_map(|_| {
                if let Some(bt) = bluetooth_service() {
                    tokio::spawn(async move {
                        let _ = bt.start_discovery().await;
                    });
                }
            });
            root_widget.connect_unmap(|_| {
                if let Some(bt) = bluetooth_service() {
                    tokio::spawn(async move {
                        let _ = bt.stop_discovery().await;
                    });
                }
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
                self.available = bt.as_ref().map(|b| b.available.get()).unwrap_or(false);
                self.enabled = bt.as_ref().map(|b| b.enabled.get()).unwrap_or(false);
                self.devices = bt.as_ref().map(|b| b.devices.get()).unwrap_or_default();
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

            // ── State refresh (triggered by update_cmd) ───────────
            BluetoothSettingsInput::RefreshState => {
                // Model already updated in update_cmd; rebuild device list below.
            }

            // ── Auto-connect config edits ─────────────────────────
            BluetoothSettingsInput::AcToggleEnabled(v) => {
                self.ac.autoconnect_enabled = v;
                persist_ac(&self.ac);
            }
            BluetoothSettingsInput::AcSetDelay(s) => {
                self.ac.autoconnect_delay_secs = s;
                persist_ac(&self.ac);
            }
            BluetoothSettingsInput::AcToggleRouteOutput(v) => {
                self.ac.route_audio_output = v;
                persist_ac(&self.ac);
            }
            BluetoothSettingsInput::AcToggleRouteInput(v) => {
                self.ac.route_audio_input = v;
                persist_ac(&self.ac);
            }
            BluetoothSettingsInput::AcAddDevice(mac, name) => {
                let mac = mac.trim().to_string();
                let dup = self
                    .ac
                    .devices
                    .iter()
                    .any(|d| d.mac.eq_ignore_ascii_case(&mac));
                if !mac.is_empty() && !dup {
                    self.ac.devices.push(BluetoothDevice {
                        mac,
                        name: name.trim().to_string(),
                    });
                    persist_ac(&self.ac);
                }
            }
            BluetoothSettingsInput::AcRemoveDevice(mac) => {
                self.ac.devices.retain(|d| d.mac != mac);
                persist_ac(&self.ac);
            }
            BluetoothSettingsInput::AcReorder(mac, delta) => {
                if let Some(from) = self.ac.devices.iter().position(|d| d.mac == mac) {
                    let to =
                        (from as i32 + delta).clamp(0, self.ac.devices.len() as i32 - 1) as usize;
                    if to != from {
                        let item = self.ac.devices.remove(from);
                        self.ac.devices.insert(to, item);
                        persist_ac(&self.ac);
                    }
                }
            }
            BluetoothSettingsInput::AcToggleNow => {
                mshell_services::tokio_rt().spawn(mshell_services::bluetooth::toggle());
            }
        }

        // Rebuild the device list box from the current model every time we
        // process a message that could have changed it.
        Self::rebuild_device_list(&widgets.device_list_box, &self.devices, &sender);
        Self::rebuild_ac_list(&widgets.ac_list_box, &self.ac.devices, &sender);

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

    /// Rebuild the auto-connect device list: one row per configured entry —
    /// a drag grip, the friendly name + MAC, and a delete button. Rows are
    /// tried top-to-bottom by the engine, so order is meaningful.
    fn rebuild_ac_list(
        list_box: &gtk::Box,
        devices: &[BluetoothDevice],
        sender: &ComponentSender<BluetoothSettingsModel>,
    ) {
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        if devices.is_empty() {
            let empty = gtk::Label::new(Some(
                "No devices yet. Add one by MAC above (e.g. your headset).",
            ));
            empty.add_css_class("label-small");
            empty.set_halign(gtk::Align::Start);
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            list_box.append(&empty);
            return;
        }

        for dev in devices {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .build();
            row.add_css_class("launcher-script-row");
            row.add_css_class("action-row");

            let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
            grip.add_css_class("reorder-grip");
            grip.set_tooltip_text(Some("Drag to reorder (first match wins)"));
            row.append(&grip);

            let label_text = if dev.name.is_empty() {
                dev.mac.clone()
            } else {
                format!("{}  ·  {}", dev.name, dev.mac)
            };
            let label = gtk::Label::builder()
                .label(&label_text)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .xalign(0.0)
                .build();
            label.add_css_class("label-medium");
            row.append(&label);

            let delete = gtk::Button::from_icon_name("user-trash-symbolic");
            delete.add_css_class("ok-button-flat");
            delete.set_valign(gtk::Align::Center);
            delete.set_tooltip_text(Some("Remove from auto-connect list"));
            {
                let mac = dev.mac.clone();
                let sender = sender.clone();
                delete.connect_clicked(move |_| {
                    sender.input(BluetoothSettingsInput::AcRemoveDevice(mac.clone()));
                });
            }
            row.append(&delete);

            // Drag-to-reorder via the grip handle (same pattern as the bar /
            // menu widget lists).
            {
                let mac = dev.mac.clone();
                let sender = sender.clone();
                crate::reorder_dnd::attach_grip_drag(&grip, &row, move |delta| {
                    sender.input(BluetoothSettingsInput::AcReorder(mac.clone(), delta));
                });
            }

            list_box.append(&row);
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

/// Persist the auto-connect config back to `config.bluetooth`. The engine
/// in mshell-services reads it at login / on toggle.
fn persist_ac(ac: &BluetoothConfig) {
    let ac = ac.clone();
    config_manager().update_config(move |c| {
        c.bluetooth = ac.clone();
    });
}
