use mshell_common::WatcherToken;
use mshell_services::network_service;
use mshell_utils::network::{
    get_wifi_icon_for_strength, spawn_available_wifi_networks_watcher, spawn_network_watcher,
    spawn_wifi_watcher, spawn_wired_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, FileExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::ops::Not;
use std::sync::Arc;
use wayle_network::core::access_point::{AccessPoint, SecurityType, Ssid};
use wayle_network::types::states::NetworkStatus;

use crate::net::connection_editor::{
    ConnectionEditorInput, ConnectionEditorModel, ConnectionEditorOutput,
};
use crate::net::nmcli::{self, ConnRow};

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct NetworkSettingsModel {
    wifi_available: bool,
    wifi_enabled: bool,
    wifi_ssid: Option<String>,
    access_points: Vec<Arc<AccessPoint>>,
    wired_available: bool,
    wired_status: NetworkStatus,
    vpn_connections: Vec<ConnRow>,
    /// All connections — kept for OpenEditor lookups (name + kind).
    all_connections: Vec<ConnRow>,
    wifi_watcher_token: WatcherToken,
    wired_watcher_token: WatcherToken,
    /// Embedded connection editor — lives in the internal stack.
    editor_controller: Controller<ConnectionEditorModel>,
}

impl std::fmt::Debug for NetworkSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkSettingsModel")
            .field("wifi_available", &self.wifi_available)
            .field("wifi_enabled", &self.wifi_enabled)
            .field("wired_available", &self.wired_available)
            .field("all_connections", &self.all_connections.len())
            .finish()
    }
}

// ── Input ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum NetworkSettingsInput {
    SetWifiEnabled(bool),
    ConnectAp(String),
    ConnectApWithPassword(String, String),
    /// Forget saved connections for an SSID (delete via wayle settings).
    ForgetConn(String),
    /// Open the connection editor for the given UUID.
    OpenEditor(String),
    /// Editor closed (Back or successful Apply) — switch back to the list.
    EditorClosed,
    UpConn(String),
    DownConn(String),
    DeleteConn(String),
    ImportVpnClicked,
    Toast(String),
    /// Re-read wayle state into model + rebuild lists.
    RefreshState,
    ConnectionsReloaded(Vec<ConnRow>),
}

// ── Output / Init / CommandOutput ─────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum NetworkSettingsOutput {}

pub(crate) struct NetworkSettingsInit {}

#[derive(Debug)]
pub(crate) enum NetworkSettingsCommandOutput {
    StateChanged,
    WifiChanged,
    WiredChanged,
    AvailableNetworksChanged,
}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub)]
impl Component for NetworkSettingsModel {
    type CommandOutput = NetworkSettingsCommandOutput;
    type Input = NetworkSettingsInput;
    type Output = NetworkSettingsOutput;
    type Init = NetworkSettingsInit;

    view! {
        // Root box wraps an internal stack so the connection editor can be
        // embedded without opening a separate toplevel gtk::Window (which
        // would fail inside a layer-shell surface).  "list" holds the existing
        // network overview; "editor" holds the connection editor.
        #[root]
        gtk::Box {
            set_hexpand: true,
            set_vexpand: true,

            #[name = "page_stack"]
            gtk::Stack {
                set_hexpand: true,
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 150,

                // ── List view ─────────────────────────────────────────────
                add_named[Some("list")] = &gtk::ScrolledWindow {
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

                    // ── Hero header ──────────────────────────────────────────
                    gtk::Box {
                        add_css_class: "settings-hero",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Start,
                        set_spacing: 16,
                        gtk::Image {
                            add_css_class: "settings-hero-icon",
                            set_icon_name: Some("network-wireless-symbolic"),
                            set_valign: gtk::Align::Center,
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            gtk::Label {
                                add_css_class: "settings-hero-title",
                                set_label: "Network",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "settings-hero-subtitle",
                                set_label: "Manage Wi-Fi, wired connections, and VPN profiles.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                    },

                // ── Wi-Fi section ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Wi-Fi",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.wifi_available,
                },

                // Enable toggle row
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.wifi_available,

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
                            set_label: "Turn Wi-Fi hardware on or off.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(wifi_enabled_handler)]
                        set_active: model.wifi_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(NetworkSettingsInput::SetWifiEnabled(enabled));
                            glib::Propagation::Proceed
                        } @wifi_enabled_handler,
                    },
                },

                // Radio-off hint
                gtk::Box {
                    add_css_class: "wifi-radio-off",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.wifi_available && !model.wifi_enabled,

                    gtk::Image {
                        set_icon_name: Some("network-wireless-disabled-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Enable Wi-Fi above to see and connect to nearby networks.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // AP list empty state
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "No networks found — scanning while this page is open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_visible: model.wifi_available
                        && model.wifi_enabled
                        && model.access_points.is_empty(),
                },

                // AP list box (rebuilt in update_with_view)
                #[name = "ap_list_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    #[watch]
                    set_visible: model.wifi_available
                        && model.wifi_enabled
                        && !model.access_points.is_empty(),
                },

                // ── Wired section ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Wired",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.wired_available,
                },

                #[name = "wired_row"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    add_css_class: "wired-row",
                    #[watch]
                    set_visible: model.wired_available,

                    gtk::Image {
                        set_icon_name: Some("network-wired-symbolic"),
                        set_valign: gtk::Align::Center,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_label: "Ethernet",
                            set_halign: gtk::Align::Start,
                        },

                        #[name = "wired_status_label"]
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            #[watch]
                            set_label: net_status_str(model.wired_status),
                        },
                    },

                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_icon_name: "emblem-system-symbolic",
                        set_valign: gtk::Align::Center,
                        set_tooltip_text: Some("Edit connection (coming soon)"),
                        connect_clicked[sender] => move |_| {
                            // TODO(task4): pass the active wired connection UUID
                            sender.input(NetworkSettingsInput::OpenEditor(String::new()));
                        },
                    },
                },

                // ── VPN section ───────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "VPN & WireGuard",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "No VPN profiles found.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_visible: model.vpn_connections.is_empty(),
                },

                // VPN list box (rebuilt in update_with_view)
                #[name = "vpn_list_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    #[watch]
                    set_visible: !model.vpn_connections.is_empty(),
                },

                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_label: "Import VPN…",
                    set_halign: gtk::Align::Start,
                    connect_clicked[sender] => move |_| {
                        sender.input(NetworkSettingsInput::ImportVpnClicked);
                    },
                },
            }
        },

            // ── Editor view (embedded — no separate toplevel) ─────────
            add_named[Some("editor")] = model.editor_controller.widget(),
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Spawn long-lived network watchers
        spawn_network_watcher(
            &sender,
            || NetworkSettingsCommandOutput::StateChanged,
            || NetworkSettingsCommandOutput::WifiChanged,
            || NetworkSettingsCommandOutput::WiredChanged,
        );

        let network = network_service();
        let wifi_opt = network.wifi.get();
        let wired_opt = network.wired.get();

        // Build the editor controller first — its widget is referenced in
        // view_output!() via `model.editor_controller.widget()`.
        let editor_controller = ConnectionEditorModel::builder()
            .launch(())
            .forward(sender.input_sender(), |output| match output {
                ConnectionEditorOutput::Closed => NetworkSettingsInput::EditorClosed,
            });

        let model = NetworkSettingsModel {
            wifi_available: wifi_opt.is_some(),
            wifi_enabled: wifi_opt.as_ref().map(|w| w.enabled.get()).unwrap_or(false),
            wifi_ssid: wifi_opt.as_ref().and_then(|w| w.ssid.get()),
            access_points: wifi_opt.as_ref().map(|w| w.access_points.get()).unwrap_or_default(),
            wired_available: wired_opt.is_some(),
            wired_status: wired_opt
                .as_ref()
                .map(|w| w.connectivity.get())
                .unwrap_or(NetworkStatus::Disconnected),
            vpn_connections: Vec::new(),
            all_connections: Vec::new(),
            wifi_watcher_token: WatcherToken::new(),
            wired_watcher_token: WatcherToken::new(),
            editor_controller,
        };

        let widgets = view_output!();

        // On page show: rescan Wi-Fi and reload VPN list
        {
            let root_w = root.clone();
            root_w.connect_map(move |_| {
                let sender_map = sender.clone();
                glib::spawn_future_local(async move {
                    let _ = nmcli::wifi_rescan().await;
                    match nmcli::list_connections().await {
                        Ok(rows) => {
                            sender_map.input(NetworkSettingsInput::ConnectionsReloaded(rows))
                        }
                        Err(e) => sender_map.input(NetworkSettingsInput::Toast(e)),
                    }
                });
                // Also request a device-level scan via wayle
                if let Some(wifi) = network_service().wifi.get() {
                    tokio::spawn(async move {
                        let _ = wifi.device.request_scan().await;
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
            NetworkSettingsCommandOutput::StateChanged => {
                let network = network_service();
                let wifi_opt = network.wifi.get();
                let wired_opt = network.wired.get();
                self.wifi_available = wifi_opt.is_some();
                self.wifi_enabled = wifi_opt.as_ref().map(|w| w.enabled.get()).unwrap_or(false);
                self.wifi_ssid = wifi_opt.as_ref().and_then(|w| w.ssid.get());
                self.wired_available = wired_opt.is_some();
                self.wired_status = wired_opt
                    .as_ref()
                    .map(|w| w.connectivity.get())
                    .unwrap_or(NetworkStatus::Disconnected);
                sender.input(NetworkSettingsInput::RefreshState);
            }
            NetworkSettingsCommandOutput::WifiChanged => {
                let token = self.wifi_watcher_token.reset();
                let token2 = token.clone();
                spawn_wifi_watcher(&sender, token2, || {
                    NetworkSettingsCommandOutput::StateChanged
                });
                let token3 = token.clone();
                spawn_available_wifi_networks_watcher(&sender, token3, || {
                    NetworkSettingsCommandOutput::AvailableNetworksChanged
                });
            }
            NetworkSettingsCommandOutput::WiredChanged => {
                let token = self.wired_watcher_token.reset();
                spawn_wired_watcher(&sender, token, || {
                    NetworkSettingsCommandOutput::StateChanged
                });
            }
            NetworkSettingsCommandOutput::AvailableNetworksChanged => {
                let network = network_service();
                self.access_points = network
                    .wifi
                    .get()
                    .map(|w| w.access_points.get())
                    .unwrap_or_default();
                self.wifi_ssid = network.wifi.get().and_then(|w| w.ssid.get());
                sender.input(NetworkSettingsInput::RefreshState);
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
            // ── Wi-Fi enable toggle ───────────────────────────────────────
            NetworkSettingsInput::SetWifiEnabled(enabled) => {
                if let Some(wifi) = network_service().wifi.get() {
                    tokio::spawn(async move {
                        let _ = wifi.set_enabled(enabled).await;
                    });
                }
            }

            // ── Connect to AP (saved profile or open network) ─────────────
            NetworkSettingsInput::ConnectAp(ssid) => {
                let network = network_service();
                let has_saved = network
                    .settings
                    .connections_for_ssid(&Ssid::from(ssid.as_str()))
                    .is_empty()
                    .not();
                if let Some(wifi) = network.wifi.get() {
                    if has_saved {
                        // Re-resolve the live AP path at action time so the
                        // message enum stays a plain String — no zbus type
                        // leaks into the input API (mirrors network_toggle).
                        if let Some(ap_path) = self
                            .access_points
                            .iter()
                            .find(|a| a.ssid.get().to_string() == ssid)
                            .map(|a| a.object_path().clone())
                        {
                            tokio::spawn(async move {
                                let _ = wifi.connect(ap_path, None).await;
                            });
                        }
                    } else {
                        let sender_c = sender.clone();
                        glib::spawn_future_local(async move {
                            if let Err(e) = nmcli::wifi_connect(&ssid, None).await {
                                sender_c.input(NetworkSettingsInput::Toast(e));
                            }
                        });
                    }
                }
            }

            // ── Connect to secured AP with password ───────────────────────
            NetworkSettingsInput::ConnectApWithPassword(ssid, password) => {
                if let Some(wifi) = network_service().wifi.get()
                    && let Some(ap_path) = self
                        .access_points
                        .iter()
                        .find(|a| a.ssid.get().to_string() == ssid)
                        .map(|a| a.object_path().clone())
                {
                    let sender_c = sender.clone();
                    tokio::spawn(async move {
                        if let Err(e) = wifi.connect(ap_path, Some(password)).await {
                            let msg = format!("Failed to connect: {e}");
                            glib::idle_add_once(move || {
                                sender_c.input(NetworkSettingsInput::Toast(msg));
                            });
                        }
                    });
                }
            }

            // ── Forget saved connections for an SSID ──────────────────────
            NetworkSettingsInput::ForgetConn(ssid) => {
                let network = network_service();
                let ssid_val = Ssid::from(ssid.as_str());
                let settings = network.settings.clone();
                let sender_c = sender.clone();
                tokio::spawn(async move {
                    settings.delete_connections_for_ssid(&ssid_val).await;
                    glib::idle_add_once(move || {
                        sender_c.input(NetworkSettingsInput::RefreshState);
                    });
                });
            }

            // ── VPN up ────────────────────────────────────────────────────
            NetworkSettingsInput::UpConn(uuid) => {
                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = nmcli::up(&uuid).await {
                        sender_c.input(NetworkSettingsInput::Toast(e));
                    } else {
                        reload_vpn_list(&sender_c).await;
                    }
                });
            }

            // ── VPN down ──────────────────────────────────────────────────
            NetworkSettingsInput::DownConn(uuid) => {
                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = nmcli::down(&uuid).await {
                        sender_c.input(NetworkSettingsInput::Toast(e));
                    } else {
                        reload_vpn_list(&sender_c).await;
                    }
                });
            }

            // ── VPN delete ────────────────────────────────────────────────
            NetworkSettingsInput::DeleteConn(uuid) => {
                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = nmcli::delete(&uuid).await {
                        sender_c.input(NetworkSettingsInput::Toast(e));
                    } else {
                        reload_vpn_list(&sender_c).await;
                    }
                });
            }

            // ── Import VPN ────────────────────────────────────────────────
            NetworkSettingsInput::ImportVpnClicked => {
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Import VPN Profile");

                let filter = gtk::FileFilter::new();
                filter.set_name(Some("VPN profiles (*.ovpn, *.conf)"));
                filter.add_pattern("*.ovpn");
                filter.add_pattern("*.conf");
                let store = relm4::gtk::gio::ListStore::new::<gtk::FileFilter>();
                store.append(&filter);
                dialog.set_filters(Some(&store));

                let sender_c = sender.clone();
                dialog.open(
                    None::<&gtk::Window>,
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result
                            && let Some(path) = file.path()
                        {
                            let path_str = path.to_string_lossy().to_string();
                            let kind = if path_str.ends_with(".ovpn") {
                                "openvpn"
                            } else {
                                "wireguard"
                            }
                            .to_string();
                            let sender_i = sender_c.clone();
                            glib::spawn_future_local(async move {
                                match nmcli::import_vpn(&path_str, &kind).await {
                                    Ok(_) => {
                                        mshell_launcher::notify::toast(
                                            "Network",
                                            "VPN profile imported.",
                                        );
                                        reload_vpn_list(&sender_i).await;
                                    }
                                    Err(e) => {
                                        sender_i.input(NetworkSettingsInput::Toast(e));
                                    }
                                }
                            });
                        }
                        // Cancelled or no path: no-op
                    },
                );
            }

            // ── Toast ─────────────────────────────────────────────────────
            NetworkSettingsInput::Toast(msg) => {
                mshell_launcher::notify::toast("Network", &msg);
            }

            // ── RefreshState — model already updated, rebuild lists ────────
            NetworkSettingsInput::RefreshState => {}

            // ── Connections reloaded ──────────────────────────────────────
            NetworkSettingsInput::ConnectionsReloaded(rows) => {
                self.all_connections = rows.clone();
                self.vpn_connections = rows
                    .into_iter()
                    .filter(|r| r.kind == "vpn" || r.kind == "wireguard")
                    .collect();
            }

            // ── Open connection editor ────────────────────────────────────
            NetworkSettingsInput::OpenEditor(uuid) => {
                if uuid.is_empty() {
                    mshell_launcher::notify::toast(
                        "Network",
                        "Cannot open editor: no connection UUID.",
                    );
                } else {
                    // Look up the ConnRow so we can pass the display name + wifi flag.
                    let (conn_name, is_wifi) = self
                        .all_connections
                        .iter()
                        .find(|r| r.uuid == uuid)
                        .map(|r| (r.name.clone(), r.kind == "802-11-wireless"))
                        .unwrap_or_else(|| (uuid.clone(), false));

                    self.editor_controller
                        .sender()
                        .send(ConnectionEditorInput::Load(uuid, conn_name, is_wifi))
                        .ok();
                    widgets.page_stack.set_visible_child_name("editor");
                }
            }

            // ── Editor closed ─────────────────────────────────────────────
            NetworkSettingsInput::EditorClosed => {
                widgets.page_stack.set_visible_child_name("list");
                // Reload connections list so any changes are reflected.
                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    reload_vpn_list(&sender_c).await;
                });
            }
        }

        // Rebuild dynamic lists after every input
        Self::rebuild_ap_list(
            &widgets.ap_list_box,
            &self.access_points,
            &self.wifi_ssid,
            &sender,
        );
        Self::rebuild_vpn_list(&widgets.vpn_list_box, &self.vpn_connections, &sender);

        self.update_view(widgets, sender);
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Reload VPN connections from nmcli and feed back into the model.
async fn reload_vpn_list(sender: &ComponentSender<NetworkSettingsModel>) {
    match nmcli::list_connections().await {
        Ok(rows) => sender.input(NetworkSettingsInput::ConnectionsReloaded(rows)),
        Err(e) => sender.input(NetworkSettingsInput::Toast(e)),
    }
}

impl NetworkSettingsModel {
    /// Rebuild the Wi-Fi access-point list box from the current model.
    ///
    /// The AP list in the Settings page is expected to be short (< ~30 entries),
    /// so a full rebuild on each change is acceptable — no virtualization needed.
    fn rebuild_ap_list(
        list_box: &gtk::Box,
        aps: &[Arc<AccessPoint>],
        active_ssid: &Option<String>,
        sender: &ComponentSender<NetworkSettingsModel>,
    ) {
        use relm4::gtk::prelude::*;

        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        // Sort: connected AP first, then descending signal
        let mut sorted: Vec<&Arc<AccessPoint>> = aps
            .iter()
            .filter(|a| !a.is_hidden.get())
            .collect();
        sorted.sort_by(|a, b| {
            let a_active = active_ssid
                .as_deref()
                .map(|s| s == a.ssid.get().to_string())
                .unwrap_or(false);
            let b_active = active_ssid
                .as_deref()
                .map(|s| s == b.ssid.get().to_string())
                .unwrap_or(false);
            b_active
                .cmp(&a_active)
                .then_with(|| b.strength.get().cmp(&a.strength.get()))
        });

        for ap in sorted {
            list_box.append(&Self::build_ap_row(ap, active_ssid, sender));
        }
    }

    fn build_ap_row(
        ap: &Arc<AccessPoint>,
        active_ssid: &Option<String>,
        sender: &ComponentSender<NetworkSettingsModel>,
    ) -> gtk::Box {
        use relm4::gtk::prelude::*;

        let ssid_str = ap.ssid.get().to_string();
        let strength = ap.strength.get();
        let security = ap.security.get();
        let is_active = active_ssid
            .as_deref()
            .map(|s| s == ssid_str)
            .unwrap_or(false);
        let has_security = security != SecurityType::None;

        let network = network_service();
        let has_saved = network
            .settings
            .connections_for_ssid(&Ssid::from(ssid_str.as_str()))
            .is_empty()
            .not();

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("wifi-row");

        // Strength icon
        let sig_icon = gtk::Image::from_icon_name(get_wifi_icon_for_strength(strength));
        sig_icon.set_valign(gtk::Align::Center);
        row.append(&sig_icon);

        // Info box
        let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info.set_hexpand(true);
        info.set_valign(gtk::Align::Center);

        let name_lbl = gtk::Label::new(Some(&ssid_str));
        name_lbl.add_css_class("label-medium-bold");
        name_lbl.set_halign(gtk::Align::Start);
        info.append(&name_lbl);

        // Status sub-label
        {
            let mut parts: Vec<&str> = Vec::new();
            if is_active {
                parts.push("Connected");
            }
            if has_security {
                parts.push(security.as_str());
            }
            if has_saved && !is_active {
                parts.push("Saved");
            }
            if !parts.is_empty() {
                let sub = gtk::Label::new(Some(&parts.join(" · ")));
                sub.add_css_class("label-small");
                sub.set_halign(gtk::Align::Start);
                info.append(&sub);
            }
        }
        row.append(&info);

        // Lock icon for secured networks
        if has_security {
            let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
            lock.set_valign(gtk::Align::Center);
            row.append(&lock);
        }

        // Actions column: password entry + buttons
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions.set_valign(gtk::Align::Center);

        // Password entry — shown only when the AP is secured and has no saved profile
        let needs_password = has_security && !has_saved;
        let pw_entry = gtk::Entry::new();
        pw_entry.set_placeholder_text(Some("Password"));
        pw_entry.add_css_class("ok-entry-with-border");
        pw_entry.set_visibility(false); // dots
        pw_entry.set_visible(needs_password && !is_active);
        actions.append(&pw_entry);

        let btns_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        btns_row.set_halign(gtk::Align::End);

        if !is_active {
            let ssid_c = ssid_str.clone();
            let pw_ref = pw_entry.clone();
            let sender_c = sender.clone();
            let connect_btn = gtk::Button::with_label("Connect");
            connect_btn.add_css_class("ok-button-primary");
            connect_btn.connect_clicked(move |_| {
                let pw = pw_ref.text().to_string();
                if !pw.is_empty() {
                    sender_c.input(NetworkSettingsInput::ConnectApWithPassword(
                        ssid_c.clone(),
                        pw,
                    ));
                } else {
                    sender_c.input(NetworkSettingsInput::ConnectAp(ssid_c.clone()));
                }
            });
            btns_row.append(&connect_btn);
        }

        if is_active {
            let disconnect_btn = gtk::Button::with_label("Disconnect");
            disconnect_btn.add_css_class("ok-button-primary");
            disconnect_btn.connect_clicked(move |_| {
                if let Some(wifi) = network_service().wifi.get() {
                    tokio::spawn(async move {
                        let _ = wifi.disconnect().await;
                    });
                }
            });
            btns_row.append(&disconnect_btn);
        }

        if has_saved {
            let ssid_c = ssid_str.clone();
            let sender_c = sender.clone();
            let forget_btn = gtk::Button::with_label("Forget");
            forget_btn.add_css_class("ok-button-primary");
            forget_btn.connect_clicked(move |_| {
                sender_c.input(NetworkSettingsInput::ForgetConn(ssid_c.clone()));
            });
            btns_row.append(&forget_btn);
        }

        actions.append(&btns_row);
        row.append(&actions);
        row
    }

    /// Rebuild the VPN connection list box.
    fn rebuild_vpn_list(
        list_box: &gtk::Box,
        vpns: &[ConnRow],
        sender: &ComponentSender<NetworkSettingsModel>,
    ) {
        use relm4::gtk::prelude::*;

        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        for conn in vpns {
            list_box.append(&Self::build_vpn_row(conn, sender));
        }
    }

    fn build_vpn_row(
        conn: &ConnRow,
        sender: &ComponentSender<NetworkSettingsModel>,
    ) -> gtk::Box {
        use relm4::gtk::prelude::*;

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("vpn-row");

        let icon = gtk::Image::from_icon_name("network-vpn-symbolic");
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info.set_hexpand(true);
        info.set_valign(gtk::Align::Center);

        let name_lbl = gtk::Label::new(Some(&conn.name));
        name_lbl.add_css_class("label-medium-bold");
        name_lbl.set_halign(gtk::Align::Start);
        info.append(&name_lbl);

        let state_lbl =
            gtk::Label::new(Some(if conn.active { "Active" } else { "Inactive" }));
        state_lbl.add_css_class("label-small");
        state_lbl.set_halign(gtk::Align::Start);
        info.append(&state_lbl);

        row.append(&info);

        let btns = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        btns.set_valign(gtk::Align::Center);

        if conn.active {
            let uuid = conn.uuid.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Disconnect");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(NetworkSettingsInput::DownConn(uuid.clone()));
            });
            btns.append(&btn);
        } else {
            let uuid = conn.uuid.clone();
            let sender_c = sender.clone();
            let btn = gtk::Button::with_label("Connect");
            btn.add_css_class("ok-button-primary");
            btn.connect_clicked(move |_| {
                sender_c.input(NetworkSettingsInput::UpConn(uuid.clone()));
            });
            btns.append(&btn);
        }

        let uuid = conn.uuid.clone();
        let sender_c = sender.clone();
        let del_btn = gtk::Button::with_label("Remove");
        del_btn.add_css_class("ok-button-primary");
        del_btn.connect_clicked(move |_| {
            sender_c.input(NetworkSettingsInput::DeleteConn(uuid.clone()));
        });
        btns.append(&del_btn);

        row.append(&btns);
        row
    }
}

// ── Utility ────────────────────────────────────────────────────────────────────

fn net_status_str(s: NetworkStatus) -> &'static str {
    match s {
        NetworkStatus::Connected => "Connected",
        NetworkStatus::Connecting => "Connecting…",
        NetworkStatus::Disconnected => "Disconnected",
    }
}
