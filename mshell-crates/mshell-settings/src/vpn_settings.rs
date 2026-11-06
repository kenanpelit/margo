//! Settings → VPN.
//!
//! Favourite-relay management + quick connect for the `mvpn` binary. Shells out
//! to `mvpn` (status / fav list / fav add|remove|connect / connect / disconnect)
//! — all unprivileged. Native page, so it themes from matugen like the rest of
//! Settings. The rich control panel still lives in `mvpn menu`; this page is the
//! favourites editor the bar pill / menu don't expose for add-remove.

use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, TrustedNetworksConfig};
use mshell_utils::mvpn::{VpnFav as Fav, VpnStatus, parse_fav_list, toggle_value};
use reactive_graph::traits::GetUntracked;

pub struct VpnSettingsInit {}

pub struct VpnSettingsModel {
    status: String,
    connected: bool,
    favs: Vec<Fav>,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    multihop: bool,
    obf: String,
    obf_model: gtk::StringList,
    /// `trusted_networks` (shell config), mirrored for the switches + list.
    trust: TrustedNetworksConfig,
}

/// Obfuscation modes offered in the dropdown (index ↔ string).
const OBF_MODES: &[&str] = &["auto", "off", "udp2tcp", "shadowsocks", "quic"];

#[derive(Debug)]
pub enum VpnSettingsInput {
    Refresh,
    Toggle,
    AddCurrent,
    Connect(String),
    Remove(String),
    Random,
    Fastest,
    SetLockdown(bool),
    SetAutoconnect(bool),
    SetQuantum(bool),
    SetMultihop(bool),
    SetObf(u32),
    SetTrustAutoConnect(bool),
    SetTrustDisconnect(bool),
    SetTrustWired(bool),
    /// Add a typed SSID to the trusted list.
    TrustAdd(String),
    /// Add the Wi-Fi network the machine is on.
    TrustCurrent,
    TrustRemove(String),
}

#[derive(Debug)]
pub enum VpnSettingsCmd {
    Loaded {
        status: String,
        connected: bool,
        favs: Vec<Fav>,
        lockdown: bool,
        autoconnect: bool,
        quantum: bool,
        multihop: bool,
        obf: String,
    },
}

#[relm4::component(pub)]
impl Component for VpnSettingsModel {
    type CommandOutput = VpnSettingsCmd;
    type Input = VpnSettingsInput;
    type Output = ();
    type Init = VpnSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("network-vpn-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "VPN",
                            set_halign: gtk::Align::Start,
                        },
                        #[name="status_label"]
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            #[watch]
                            set_label: &model.status,
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    gtk::Button {
                        #[watch]
                        set_label: if model.connected { "Disconnect" } else { "Connect" },
                        connect_clicked => VpnSettingsInput::Toggle,
                    },
                    gtk::Button {
                        set_label: "Random",
                        connect_clicked => VpnSettingsInput::Random,
                    },
                    gtk::Button {
                        set_label: "Fastest",
                        connect_clicked => VpnSettingsInput::Fastest,
                    },
                    gtk::Button {
                        set_label: "Add current relay",
                        connect_clicked => VpnSettingsInput::AddCurrent,
                    },
                    gtk::Button {
                        set_icon_name: "view-refresh-symbolic",
                        connect_clicked => VpnSettingsInput::Refresh,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Settings",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Lockdown mode" },
                        #[template_child] desc { set_label: "Block all traffic when the VPN drops." },
                        #[name="lockdown_sw"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.lockdown,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetLockdown(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Auto-connect" },
                        #[template_child] desc { set_label: "Bring the tunnel up when the daemon starts." },
                        #[name="autoconnect_sw"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.autoconnect,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetAutoconnect(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Quantum-resistant" },
                        #[template_child] desc { set_label: "WireGuard post-quantum key exchange." },
                        #[name="quantum_sw"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.quantum,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetQuantum(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Multihop" },
                        #[template_child] desc { set_label: "Enter Mullvad through a server in another location first (pick the entry in the VPN menu)." },
                        #[name="multihop_sw"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.multihop,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetMultihop(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Anti-censorship" },
                        #[template_child] desc { set_label: "Obfuscation method (auto / off / udp2tcp / shadowsocks / quic)." },
                        #[name="obf_drop"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.obf_model),
                            #[watch]
                            set_selected: OBF_MODES.iter().position(|m| *m == model.obf).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(VpnSettingsInput::SetObf(d.selected()));
                            },
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Trusted networks",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "dim-label",
                    set_label: "Bring the VPN up when this machine joins a Wi-Fi network that is not on the list below.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Connect on untrusted networks" },
                        #[template_child] desc { set_label: "Connect Mullvad when joining a network that isn't trusted." },
                        gtk::Switch {
                            set_widget_name: "setting:/trusted_networks/auto_connect",
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.trust.auto_connect,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetTrustAutoConnect(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Disconnect on trusted networks" },
                        #[template_child] desc { set_label: "Take a tunnel this rule started down again on a trusted network. A tunnel you started is never touched." },
                        gtk::Switch {
                            set_widget_name: "setting:/trusted_networks/disconnect_on_trusted",
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.trust.disconnect_on_trusted,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetTrustDisconnect(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Trust wired connections" },
                        #[template_child] desc { set_label: "Treat any Ethernet connection as trusted." },
                        gtk::Switch {
                            set_widget_name: "setting:/trusted_networks/trust_wired",
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.trust.trust_wired,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(VpnSettingsInput::SetTrustWired(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                },
                #[name="trust_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[name="trust_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("Wi-Fi network name (SSID)"),
                        connect_activate[sender] => move |e| {
                            sender.input(VpnSettingsInput::TrustAdd(e.text().to_string()));
                            e.set_text("");
                        },
                    },
                    gtk::Button {
                        set_label: "Add",
                        connect_clicked[sender, trust_entry] => move |_| {
                            sender.input(VpnSettingsInput::TrustAdd(trust_entry.text().to_string()));
                            trust_entry.set_text("");
                        },
                    },
                    gtk::Button {
                        set_label: "Trust this network",
                        set_tooltip_text: Some("Add the Wi-Fi network this machine is on"),
                        connect_clicked => VpnSettingsInput::TrustCurrent,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Favourites",
                    set_halign: gtk::Align::Start,
                },

                #[name="fav_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = VpnSettingsModel {
            status: "Loading…".to_string(),
            connected: false,
            favs: Vec::new(),
            lockdown: false,
            autoconnect: false,
            quantum: false,
            multihop: false,
            obf: "auto".to_string(),
            obf_model: gtk::StringList::new(OBF_MODES),
            trust: config_manager().config().trusted_networks().get_untracked(),
        };
        let widgets = view_output!();
        rebuild_trusted(&widgets.trust_box, &model.trust.trusted_ssids, &sender);
        reload(&sender);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            VpnSettingsInput::SetTrustAutoConnect(on) => {
                if on != self.trust.auto_connect {
                    self.trust.auto_connect = on;
                    config_manager().update_config(|c| c.trusted_networks.auto_connect = on);
                }
            }
            VpnSettingsInput::SetTrustDisconnect(on) => {
                if on != self.trust.disconnect_on_trusted {
                    self.trust.disconnect_on_trusted = on;
                    config_manager()
                        .update_config(|c| c.trusted_networks.disconnect_on_trusted = on);
                }
            }
            VpnSettingsInput::SetTrustWired(on) => {
                if on != self.trust.trust_wired {
                    self.trust.trust_wired = on;
                    config_manager().update_config(|c| c.trusted_networks.trust_wired = on);
                }
            }
            VpnSettingsInput::TrustAdd(ssid) => self.edit_trusted(widgets, &sender, |list| {
                add_ssid(list, &ssid);
            }),
            VpnSettingsInput::TrustCurrent => {
                if let Some(ssid) = mshell_services::vpn_trust::current_ssid() {
                    self.edit_trusted(widgets, &sender, |list| {
                        add_ssid(list, &ssid);
                    });
                }
            }
            VpnSettingsInput::TrustRemove(ssid) => {
                self.edit_trusted(widgets, &sender, |list| list.retain(|s| *s != ssid));
            }
            other => self.update_vpn(other, &sender),
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let VpnSettingsCmd::Loaded {
            status,
            connected,
            favs,
            lockdown,
            autoconnect,
            quantum,
            multihop,
            obf,
        } = message;
        self.status = status;
        self.connected = connected;
        self.favs = favs;
        self.lockdown = lockdown;
        self.autoconnect = autoconnect;
        self.quantum = quantum;
        self.multihop = multihop;
        self.obf = obf;
        rebuild_favs(&widgets.fav_box, &self.favs, &sender);
    }
}

impl VpnSettingsModel {
    /// Change the trusted SSID list, save it, and redraw it.
    fn edit_trusted(
        &mut self,
        widgets: &VpnSettingsModelWidgets,
        sender: &ComponentSender<Self>,
        edit: impl FnOnce(&mut Vec<String>),
    ) {
        let mut list = self.trust.trusted_ssids.clone();
        edit(&mut list);
        if list != self.trust.trusted_ssids {
            self.trust.trusted_ssids = list.clone();
            config_manager().update_config(|c| c.trusted_networks.trusted_ssids = list);
        }
        rebuild_trusted(&widgets.trust_box, &self.trust.trusted_ssids, sender);
    }

    fn update_vpn(&mut self, message: VpnSettingsInput, sender: &ComponentSender<Self>) {
        let sender = sender.clone();
        match message {
            VpnSettingsInput::Refresh => reload(&sender),
            VpnSettingsInput::Toggle => act(&sender, vec!["toggle".into()]),
            VpnSettingsInput::AddCurrent => act(&sender, vec!["fav".into(), "add".into()]),
            VpnSettingsInput::Connect(r) => {
                act(&sender, vec!["fav".into(), "connect".into()]);
                let _ = r; // `fav connect` picks the fastest; relay arg unused here
            }
            VpnSettingsInput::Remove(r) => act(&sender, vec!["fav".into(), "remove".into(), r]),
            VpnSettingsInput::Random => act(&sender, vec!["random".into()]),
            VpnSettingsInput::Fastest => act(&sender, vec!["fastest".into()]),
            // The compare-guards stop the `#[watch] set_active` refresh from
            // echoing back into an mvpn command (it sets the switch to the
            // value we already hold → no change → no-op).
            VpnSettingsInput::SetLockdown(on) => {
                if on != self.lockdown {
                    act(&sender, vec!["lockdown".into(), bool_arg(on)]);
                }
            }
            VpnSettingsInput::SetAutoconnect(on) => {
                if on != self.autoconnect {
                    act(&sender, vec!["auto-connect".into(), bool_arg(on)]);
                }
            }
            VpnSettingsInput::SetQuantum(on) => {
                if on != self.quantum {
                    act(&sender, vec!["quantum".into()]); // quantum is a toggle
                }
            }
            VpnSettingsInput::SetMultihop(on) => {
                if on != self.multihop {
                    act(&sender, vec!["multihop".into(), bool_arg(on)]);
                }
            }
            VpnSettingsInput::SetObf(idx) => {
                let mode = OBF_MODES.get(idx as usize).copied().unwrap_or("auto");
                if mode != self.obf {
                    act(&sender, vec!["obf".into(), mode.into()]);
                }
            }
            // Handled in `update_with_view`.
            VpnSettingsInput::SetTrustAutoConnect(_)
            | VpnSettingsInput::SetTrustDisconnect(_)
            | VpnSettingsInput::SetTrustWired(_)
            | VpnSettingsInput::TrustAdd(_)
            | VpnSettingsInput::TrustCurrent
            | VpnSettingsInput::TrustRemove(_) => {}
        }
    }
}

/// Add `ssid` (trimmed) unless it is empty or already listed.
fn add_ssid(list: &mut Vec<String>, ssid: &str) {
    let ssid = ssid.trim();
    if !ssid.is_empty() && !list.iter().any(|s| s == ssid) {
        list.push(ssid.to_string());
    }
}

/// Clear + repopulate the trusted-network list.
fn rebuild_trusted(b: &gtk::Box, ssids: &[String], sender: &ComponentSender<VpnSettingsModel>) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    if ssids.is_empty() {
        let l = gtk::Label::new(Some(
            "No trusted networks yet: every Wi-Fi counts as untrusted.",
        ));
        l.add_css_class("dim-label");
        l.set_halign(gtk::Align::Start);
        b.append(&l);
        return;
    }
    for ssid in ssids {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("ok-button-surface");
        let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
        let name = gtk::Label::new(Some(ssid));
        name.set_halign(gtk::Align::Start);
        name.set_hexpand(true);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        mshell_common::a11y::name(&remove, &format!("Stop trusting {ssid}"));
        let (s, n) = (sender.clone(), ssid.clone());
        remove.connect_clicked(move |_| s.input(VpnSettingsInput::TrustRemove(n.clone())));
        row.append(&icon);
        row.append(&name);
        row.append(&remove);
        b.append(&row);
    }
}

fn bool_arg(on: bool) -> String {
    if on { "on" } else { "off" }.to_string()
}

/// Clear + repopulate the favourites list.
fn rebuild_favs(b: &gtk::Box, favs: &[Fav], sender: &ComponentSender<VpnSettingsModel>) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    if favs.is_empty() {
        let l = gtk::Label::new(Some(
            "No favourites yet — connect, then “Add current relay”.",
        ));
        l.add_css_class("dim-label");
        l.set_halign(gtk::Align::Start);
        b.append(&l);
        return;
    }
    for f in favs {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("ok-button-surface");
        let name = gtk::Label::new(Some(&f.relay));
        name.set_halign(gtk::Align::Start);
        name.set_hexpand(true);
        let ping = gtk::Label::new(Some(&f.ping));
        ping.add_css_class("dim-label");
        let connect = gtk::Button::with_label("Connect");
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            connect.connect_clicked(move |_| s.input(VpnSettingsInput::Connect(r.clone())));
        }
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            remove.connect_clicked(move |_| s.input(VpnSettingsInput::Remove(r.clone())));
        }
        row.append(&name);
        row.append(&ping);
        row.append(&connect);
        row.append(&remove);
        b.append(&row);
    }
}

/// Spawn an `mvpn` action, then reload the page.
fn act(sender: &ComponentSender<VpnSettingsModel>, args: Vec<String>) {
    sender.command(|out, _| async move {
        let _ = tokio::process::Command::new("mvpn")
            .args(&args)
            .status()
            .await;
        let _ = out.send(load().await);
    });
}

fn reload(sender: &ComponentSender<VpnSettingsModel>) {
    sender.command(|out, _| async move {
        let _ = out.send(load().await);
    });
}

/// Query `mvpn` for status + favourites.
async fn load() -> VpnSettingsCmd {
    let vpn = VpnStatus::parse(&capture(&["status", "--json"]).await);
    let connected = vpn.connected;
    let status = vpn.summary();
    let favs = parse_fav_list(&capture(&["fav", "list"]).await);
    let toggles = capture(&["toggles"]).await;
    let kv = |key: &str| toggle_value(&toggles, key);
    VpnSettingsCmd::Loaded {
        status,
        connected,
        favs,
        lockdown: kv("lockdown") == "on",
        autoconnect: kv("autoconnect") == "on",
        quantum: kv("quantum") == "on",
        multihop: kv("multihop") == "on",
        obf: {
            let m = kv("obf");
            if m.is_empty() { "auto".to_string() } else { m }
        },
    }
}

async fn capture(args: &[&str]) -> String {
    tokio::process::Command::new("mvpn")
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::add_ssid;

    #[test]
    fn adding_a_trusted_ssid_trims_and_dedupes() {
        let mut list = vec!["Ken_5".to_string()];
        add_ssid(&mut list, "  Office ");
        add_ssid(&mut list, "Ken_5");
        add_ssid(&mut list, "   ");
        assert_eq!(list, ["Ken_5", "Office"]);
    }
}
