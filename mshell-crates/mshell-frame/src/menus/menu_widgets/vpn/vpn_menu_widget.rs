//! The native VPN menu content widget.
//!
//! All state comes from the unprivileged `mvpn` CLI (`status --json`,
//! `toggles`, `fav list`) and every action shells back out to it
//! (`toggle`, `random`, `fastest`, `fav add|remove|connect`, `lockdown`,
//! `auto-connect`, `quantum`, `obf`). No daemon/IPC — `mvpn` + the Mullvad
//! daemon are the source of truth, exactly like the bar pill + Settings page.
//!
//! Polling is lazy: the refresh loop only starts on the first reveal (and
//! probes only while visible), so a menu the user never opens spawns no
//! `mvpn` processes — see [`VpnMenuWidgetInput::ParentRevealChanged`].

use super::super::dns::dns_menu_widget::{
    DnsMenuWidgetInit, DnsMenuWidgetInput, DnsMenuWidgetModel,
};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// Anti-censorship (obfuscation) modes offered in the dropdown (index ↔ string).
const OBF_MODES: &[&str] = &["auto", "off", "udp2tcp", "shadowsocks", "quic"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fav {
    relay: String,
    ping: String,
}

pub(crate) struct VpnMenuWidgetModel {
    connected: bool,
    /// Currently-connected relay id (empty when down).
    connected_relay: String,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    obf: String,
    favs: Vec<Fav>,
    /// Widget refs synced imperatively (avoids `#[watch] set_model` churn on
    /// the dropdown, and lets the favourites list rebuild in place).
    status_label: gtk::Label,
    badge: gtk::Label,
    toggle_btn: gtk::Button,
    lockdown_sw: gtk::Switch,
    autoconnect_sw: gtk::Switch,
    quantum_sw: gtk::Switch,
    obf_drop: gtk::DropDown,
    fav_box: gtk::Box,
    expiry_label: gtk::Label,
    /// Lazy-poll gates — see `ParentRevealChanged`.
    poll_started: bool,
    visible: Arc<AtomicBool>,
    /// Embedded DNS section (Blocky / Default / presets), collapsed by
    /// default. Its own probe loop only runs while the section is expanded
    /// AND the menu is visible — see `forward_dns_reveal`.
    dns: Controller<DnsMenuWidgetModel>,
    dns_expanded: bool,
    menu_visible: bool,
}

impl std::fmt::Debug for VpnMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnMenuWidgetModel")
            .field("connected", &self.connected)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetInput {
    Toggle,
    Random,
    Fastest,
    AddCurrent,
    Connect(String),
    Remove(String),
    SetLockdown(bool),
    SetAutoconnect(bool),
    SetQuantum(bool),
    SetObf(u32),
    RefreshNow,
    /// Sent by the host menu on show/hide. The poll loop starts lazily on the
    /// first reveal and probes only while visible.
    ParentRevealChanged(bool),
    /// The DNS section's expander toggled.
    DnsExpanded(bool),
}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetOutput {}

pub(crate) struct VpnMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetCommandOutput {
    Loaded {
        status: String,
        connected: bool,
        /// Currently-connected relay id (empty when down) — used to mark the
        /// matching favourites row as active.
        relay: String,
        favs: Vec<Fav>,
        lockdown: bool,
        autoconnect: bool,
        quantum: bool,
        obf: String,
        /// Account expiry date (`YYYY-MM-DD`), from `mvpn toggles`.
        expiry: String,
    },
}

#[relm4::component(pub(crate))]
impl Component for VpnMenuWidgetModel {
    type CommandOutput = VpnMenuWidgetCommandOutput;
    type Input = VpnMenuWidgetInput;
    type Output = VpnMenuWidgetOutput;
    type Init = VpnMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "vpn-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("network-vpn-symbolic"),
                    set_valign: gtk::Align::Center,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        add_css_class: "panel-title",
                        set_label: "VPN",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    status_label_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_wrap_mode: gtk::pango::WrapMode::WordChar,
                    },
                },

                #[local_ref]
                badge_widget -> gtk::Label {
                    add_css_class: "vpn-badge",
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Primary actions ─────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                #[local_ref]
                toggle_btn_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Random",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::Random),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Fastest",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::Fastest),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Add",
                    set_tooltip_text: Some("Add the current relay to favourites"),
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::AddCurrent),
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Settings",
                set_xalign: 0.0,
            },

            // Toggle rows + anti-censorship dropdown, built imperatively.
            #[local_ref]
            settings_box -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Favourites",
                set_xalign: 0.0,
            },

            #[local_ref]
            fav_box_widget -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            // ── DNS section (Blocky / Default / presets) ────────
            // Collapsed by default; the embedded DnsMenuWidget's child is
            // attached + its expand signal wired in `init`.
            #[name = "dns_expander"]
            gtk::Expander {
                add_css_class: "vpn-dns-expander",
                set_label: Some("DNS  ·  Blocky · presets"),
                set_expanded: false,
                connect_expanded_notify[sender] => move |e| {
                    sender.input(VpnMenuWidgetInput::DnsExpanded(e.is_expanded()));
                },
            },

            // Account expiry — small dim line, hidden until known.
            #[local_ref]
            expiry_label_widget -> gtk::Label {
                add_css_class: "label-small",
                add_css_class: "dim-label",
                set_xalign: 0.0,
                set_margin_top: 2,
            },

            // ── Footer ──────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_margin_top: 4,

                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::RefreshNow),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let status_label_widget = gtk::Label::new(Some("Loading…"));
        let badge_widget = gtk::Label::new(Some("Idle"));
        let toggle_btn_widget = gtk::Button::with_label("Connect");
        {
            let s = sender.clone();
            toggle_btn_widget.connect_clicked(move |_| s.input(VpnMenuWidgetInput::Toggle));
        }

        // Build the three toggle rows + the anti-censorship dropdown, holding
        // refs so `sync_view` can drive them without `#[watch]`.
        let settings_box = gtk::Box::new(gtk::Orientation::Vertical, 6);

        let lockdown_sw = gtk::Switch::new();
        lockdown_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Lockdown mode",
            "Block all traffic when the VPN drops.",
            &lockdown_sw,
        ));
        let autoconnect_sw = gtk::Switch::new();
        autoconnect_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Auto-connect",
            "Bring the tunnel up when the daemon starts.",
            &autoconnect_sw,
        ));
        let quantum_sw = gtk::Switch::new();
        quantum_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Quantum-resistant",
            "WireGuard post-quantum key exchange.",
            &quantum_sw,
        ));

        let obf_drop =
            gtk::DropDown::new(Some(gtk::StringList::new(OBF_MODES)), gtk::Expression::NONE);
        obf_drop.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Anti-censorship",
            "Obfuscation: auto / off / udp2tcp / shadowsocks / quic.",
            &obf_drop,
        ));

        // Wire the toggle signals. Compare-guards in `update` stop the
        // imperative `sync_view` set from echoing back into an `mvpn` call.
        {
            let s = sender.clone();
            lockdown_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetLockdown(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            autoconnect_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetAutoconnect(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            quantum_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetQuantum(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            obf_drop.connect_selected_notify(move |d| {
                s.input(VpnMenuWidgetInput::SetObf(d.selected()));
            });
        }

        let fav_box_widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let expiry_label_widget = gtk::Label::new(None);
        expiry_label_widget.set_visible(false);

        // Embedded DNS section: Blocky / Default / presets, no VPN chrome.
        let dns = DnsMenuWidgetModel::builder()
            .launch(DnsMenuWidgetInit { embedded: true })
            .detach();

        let model = VpnMenuWidgetModel {
            connected: false,
            connected_relay: String::new(),
            lockdown: false,
            autoconnect: false,
            quantum: false,
            obf: "auto".to_string(),
            favs: Vec::new(),
            status_label: status_label_widget.clone(),
            badge: badge_widget.clone(),
            toggle_btn: toggle_btn_widget.clone(),
            lockdown_sw: lockdown_sw.clone(),
            autoconnect_sw: autoconnect_sw.clone(),
            quantum_sw: quantum_sw.clone(),
            obf_drop: obf_drop.clone(),
            fav_box: fav_box_widget.clone(),
            expiry_label: expiry_label_widget.clone(),
            poll_started: false,
            visible: Arc::new(AtomicBool::new(false)),
            dns,
            dns_expanded: false,
            menu_visible: false,
        };

        let widgets = view_output!();
        // Drop the embedded DNS widget into the expander now that both exist.
        widgets.dns_expander.set_child(Some(model.dns.widget()));
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnMenuWidgetInput::Toggle => act(&sender, vec!["toggle".into()]),
            VpnMenuWidgetInput::Random => act(&sender, vec!["random".into()]),
            VpnMenuWidgetInput::Fastest => act(&sender, vec!["fastest".into()]),
            VpnMenuWidgetInput::AddCurrent => act(&sender, vec!["fav".into(), "add".into()]),
            // `mvpn fav connect` picks the fastest favourite (no per-relay
            // connect in the CLI); the relay arg is kept for clarity.
            VpnMenuWidgetInput::Connect(_relay) => {
                act(&sender, vec!["fav".into(), "connect".into()]);
            }
            VpnMenuWidgetInput::Remove(relay) => {
                act(&sender, vec!["fav".into(), "remove".into(), relay]);
            }
            VpnMenuWidgetInput::SetLockdown(on) => {
                if on != self.lockdown {
                    act(&sender, vec!["lockdown".into(), bool_arg(on)]);
                }
            }
            VpnMenuWidgetInput::SetAutoconnect(on) => {
                if on != self.autoconnect {
                    act(&sender, vec!["auto-connect".into(), bool_arg(on)]);
                }
            }
            VpnMenuWidgetInput::SetQuantum(on) => {
                if on != self.quantum {
                    act(&sender, vec!["quantum".into()]); // quantum is a toggle
                }
            }
            VpnMenuWidgetInput::SetObf(idx) => {
                let mode = OBF_MODES.get(idx as usize).copied().unwrap_or("auto");
                if mode != self.obf {
                    act(&sender, vec!["obf".into(), mode.into()]);
                }
            }
            VpnMenuWidgetInput::RefreshNow => reload(&sender),
            VpnMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible {
                    if !self.poll_started {
                        self.poll_started = true;
                        start_polling(&sender, self.visible.clone());
                    }
                    reload(&sender);
                }
                self.menu_visible = visible;
                self.forward_dns_reveal();
            }
            VpnMenuWidgetInput::DnsExpanded(expanded) => {
                self.dns_expanded = expanded;
                self.forward_dns_reveal();
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let VpnMenuWidgetCommandOutput::Loaded {
            status,
            connected,
            relay,
            favs,
            lockdown,
            autoconnect,
            quantum,
            obf,
            expiry,
        } = message;
        self.connected = connected;
        self.connected_relay = relay;
        self.lockdown = lockdown;
        self.autoconnect = autoconnect;
        self.quantum = quantum;
        self.obf = obf;
        self.favs = favs;

        self.status_label.set_label(&status);
        self.badge
            .set_label(if connected { "Connected" } else { "Idle" });
        if connected {
            self.badge.add_css_class("ok");
        } else {
            self.badge.remove_css_class("ok");
        }
        self.toggle_btn
            .set_label(if connected { "Disconnect" } else { "Connect" });
        // Each `set_active` fires `connect_state_set` → an input, but the
        // compare-guard there drops it since the field already holds the value.
        self.lockdown_sw.set_active(lockdown);
        self.autoconnect_sw.set_active(autoconnect);
        self.quantum_sw.set_active(quantum);
        let sel = OBF_MODES.iter().position(|m| *m == self.obf).unwrap_or(0) as u32;
        if self.obf_drop.selected() != sel {
            self.obf_drop.set_selected(sel);
        }

        // Account expiry line — hidden until known / unparseable.
        let show_expiry = !expiry.is_empty() && expiry != "—";
        if show_expiry {
            self.expiry_label
                .set_label(&format!("Account expires {expiry}"));
        }
        self.expiry_label.set_visible(show_expiry);

        rebuild_favs(&self.fav_box, &self.favs, &self.connected_relay, &sender);
    }
}

impl VpnMenuWidgetModel {
    /// Drive the embedded DNS section's lazy probe: it runs only while the
    /// menu is visible *and* the DNS section is expanded.
    fn forward_dns_reveal(&self) {
        let reveal = self.menu_visible && self.dns_expanded;
        let _ = self
            .dns
            .sender()
            .send(DnsMenuWidgetInput::ParentRevealChanged(reveal));
    }
}

/// Build a DESIGN.md settings row: title + description on the left, the
/// control widget right-aligned.
fn toggle_row(title: &str, desc: &str, control: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::Box {
    use relm4::gtk::prelude::Cast;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("ok-button-surface");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_xalign(0.0);
    let d = gtk::Label::new(Some(desc));
    d.add_css_class("label-small");
    d.set_xalign(0.0);
    d.set_wrap(true);
    d.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.append(&t);
    text.append(&d);
    row.append(&text);
    let control: &gtk::Widget = control.upcast_ref();
    row.append(control);
    row
}

fn bool_arg(on: bool) -> String {
    if on { "on" } else { "off" }.to_string()
}

/// Clear + repopulate the favourites list (relay + ping, per-row connect/remove).
/// The row whose relay is `connected_relay` is marked active: its button reads
/// "Connected" and the row + button carry the `.active` accent class.
fn rebuild_favs(
    b: &gtk::Box,
    favs: &[Fav],
    connected_relay: &str,
    sender: &ComponentSender<VpnMenuWidgetModel>,
) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    if favs.is_empty() {
        let l = gtk::Label::new(Some("No favourites yet — connect, then “Add”."));
        l.add_css_class("label-small");
        l.set_xalign(0.0);
        b.append(&l);
        return;
    }
    for f in favs {
        let is_active = !connected_relay.is_empty() && f.relay == connected_relay;
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("ok-button-surface");
        if is_active {
            row.add_css_class("active");
        }
        let name = gtk::Label::new(Some(&f.relay));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        let ping = gtk::Label::new(Some(&f.ping));
        ping.add_css_class("label-small");
        let connect = gtk::Button::with_label(if is_active { "Connected" } else { "Connect" });
        connect.add_css_class("ok-button-cell");
        if is_active {
            connect.add_css_class("active");
            connect.set_sensitive(false);
        }
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            connect.connect_clicked(move |_| s.input(VpnMenuWidgetInput::Connect(r.clone())));
        }
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            remove.connect_clicked(move |_| s.input(VpnMenuWidgetInput::Remove(r.clone())));
        }
        row.append(&name);
        row.append(&ping);
        row.append(&connect);
        row.append(&remove);
        b.append(&row);
    }
}

/// Spawn an `mvpn` action off-thread, then reload the menu.
fn act(sender: &ComponentSender<VpnMenuWidgetModel>, args: Vec<String>) {
    sender.command(|out, _shutdown| async move {
        let _ = tokio::process::Command::new("mvpn")
            .args(&args)
            .status()
            .await;
        let _ = out.send(load().await);
    });
}

fn reload(sender: &ComponentSender<VpnMenuWidgetModel>) {
    sender.command(|out, _shutdown| async move {
        let _ = out.send(load().await);
    });
}

/// Perpetual poll loop, started lazily on first reveal; gated on `visible`
/// so a hidden menu only does a cheap timer wake.
fn start_polling(sender: &ComponentSender<VpnMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
            }
            if visible.load(Ordering::Relaxed) {
                let _ = out.send(load().await);
            }
        }
    });
}

/// Query `mvpn` for status + toggles + favourites.
async fn load() -> VpnMenuWidgetCommandOutput {
    let status_raw = capture(&["status", "--json"]).await;
    let connected = status_raw.contains("\"connected\":true");
    let relay = if connected {
        json_str(&status_raw, "relay")
    } else {
        String::new()
    };
    let status = if connected {
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
    let toggles = capture(&["toggles"]).await;
    let kv = |key: &str| -> String {
        toggles
            .lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{key}=")))
            .unwrap_or("")
            .to_string()
    };
    VpnMenuWidgetCommandOutput::Loaded {
        status,
        connected,
        relay,
        favs,
        lockdown: kv("lockdown") == "on",
        autoconnect: kv("autoconnect") == "on",
        quantum: kv("quantum") == "on",
        obf: {
            let m = kv("obf");
            if m.is_empty() { "auto".to_string() } else { m }
        },
        expiry: kv("expiry"),
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
