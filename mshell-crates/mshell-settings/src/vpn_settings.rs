//! Settings → VPN.
//!
//! Favourite-relay management + quick connect for the `mvpn` binary. Shells out
//! to `mvpn` (status / fav list / fav add|remove|connect / connect / disconnect)
//! — all unprivileged. Native page, so it themes from matugen like the rest of
//! Settings. The rich control panel still lives in `mvpn menu`; this page is the
//! favourites editor the bar pill / menu don't expose for add-remove.

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fav {
    relay: String,
    ping: String,
}

pub struct VpnSettingsInit {}

pub struct VpnSettingsModel {
    status: String,
    connected: bool,
    favs: Vec<Fav>,
}

#[derive(Debug)]
pub enum VpnSettingsInput {
    Refresh,
    Toggle,
    AddCurrent,
    Connect(String),
    Remove(String),
}

#[derive(Debug)]
pub enum VpnSettingsCmd {
    Loaded {
        status: String,
        connected: bool,
        favs: Vec<Fav>,
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
        };
        let widgets = view_output!();
        reload(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnSettingsInput::Refresh => reload(&sender),
            VpnSettingsInput::Toggle => act(&sender, vec!["toggle".into()]),
            VpnSettingsInput::AddCurrent => act(&sender, vec!["fav".into(), "add".into()]),
            VpnSettingsInput::Connect(r) => {
                act(&sender, vec!["fav".into(), "connect".into()]);
                let _ = r; // `fav connect` picks the fastest; relay arg unused here
            }
            VpnSettingsInput::Remove(r) => act(&sender, vec!["fav".into(), "remove".into(), r]),
        }
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
        } = message;
        self.status = status;
        self.connected = connected;
        self.favs = favs;
        rebuild_favs(&widgets.fav_box, &self.favs, &sender);
    }
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
    let status_raw = capture(&["status", "--json"]).await;
    let connected = status_raw.contains("\"connected\":true");
    let status = if connected {
        let relay = json_str(&status_raw, "relay");
        let loc = json_str(&status_raw, "location");
        format!(
            "Connected · {relay}{}",
            if loc.is_empty() {
                String::new()
            } else {
                format!(" · {loc}")
            }
        )
    } else {
        "Disconnected".to_string()
    };
    let favs = parse_fav_list(&capture(&["fav", "list"]).await);
    VpnSettingsCmd::Loaded {
        status,
        connected,
        favs,
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

fn parse_fav_list(out: &str) -> Vec<Fav> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let relay = line.split_whitespace().last()?.to_string();
            if relay.is_empty() {
                return None;
            }
            // Everything before the relay token is the ping label.
            let ping = line[..line.len() - relay.len()].trim().to_string();
            Some(Fav { relay, ping })
        })
        .collect()
}

fn json_str(json: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let Some(i) = json.find(&needle) else {
        return String::new();
    };
    let rest = &json[i + needle.len()..];
    rest.find('"')
        .map(|e| rest[..e].to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fav_list() {
        let out = "     45 ms  de-fra-wg-002\n    N/A    fr-par-wg-001\n";
        let favs = parse_fav_list(out);
        assert_eq!(favs.len(), 2);
        assert_eq!(favs[0].relay, "de-fra-wg-002");
        assert_eq!(favs[0].ping, "45 ms");
        assert_eq!(favs[1].relay, "fr-par-wg-001");
        assert_eq!(favs[1].ping, "N/A");
    }
}
