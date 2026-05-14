//! Network Console menu widget — content surface for
//! `MenuType::Nnetwork`.
//!
//! Layout:
//!   * **Hero** — active-connection summary (icon + name +
//!     IP + connectivity badge).
//!   * **Controls** — Wi-Fi radio toggle Switch + Rescan button.
//!   * **Network list** — scrollable rows of scanned APs, each:
//!     signal-strength icon + SSID + lock glyph (if secured) +
//!     Connect / Connected button.
//!
//! Link / Wi-Fi state is read reactively from `network_service()`
//! (NetworkManager over D-Bus) — no polling. Actions are still
//! unprivileged `nmcli` invocations, but only on a user click:
//!   * `nmcli radio wifi on/off`
//!   * `nmcli device wifi rescan`
//!   * `nmcli device wifi connect <ssid>` (NM prompts via its
//!     own agent if the saved secret is missing — for the MVP
//!     we connect to already-known SSIDs; unknown-SSID password
//!     entry is a follow-up).
//!   * `nmcli connection down <name>` to disconnect.
//! After an action the D-Bus watchers refresh the panel — no
//! re-probe.

use crate::bars::bar_widgets::nnetwork::{
    LinkKind, NetworkState, WifiNetwork, read_network_state, wifi_signal_icon,
};
use mshell_common::WatcherToken;
use mshell_utils::network::{
    spawn_available_wifi_networks_watcher, spawn_network_watcher, spawn_wifi_watcher,
    spawn_wired_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, ListBoxRowExt, ObjectExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use tracing::warn;

pub(crate) struct NnetworkMenuWidgetModel {
    state: NetworkState,
    hero_icon: gtk::Image,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    connectivity_badge: gtk::Label,
    wifi_switch: gtk::Switch,
    wifi_switch_signal: glib::SignalHandlerId,
    network_list: gtk::ListBox,
    wifi_watcher_token: WatcherToken,
    wired_watcher_token: WatcherToken,
}

impl std::fmt::Debug for NnetworkMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NnetworkMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NnetworkMenuWidgetInput {
    SetWifiEnabled(bool),
    Rescan,
    Connect(String),
    Disconnect,
}

#[derive(Debug)]
pub(crate) enum NnetworkMenuWidgetOutput {}

pub(crate) struct NnetworkMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NnetworkMenuWidgetCommandOutput {
    /// Link / Wi-Fi / AP state changed (a D-Bus watcher fired).
    NetworkChanged,
    /// The Wi-Fi device was (un)plugged — re-arm its sub-watchers.
    WifiChanged,
    /// The wired device was (un)plugged — re-arm its sub-watcher.
    WiredChanged,
}

#[relm4::component(pub(crate))]
impl Component for NnetworkMenuWidgetModel {
    type CommandOutput = NnetworkMenuWidgetCommandOutput;
    type Input = NnetworkMenuWidgetInput;
    type Output = NnetworkMenuWidgetOutput;
    type Init = NnetworkMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "nnetwork-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "nnetwork-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[local_ref]
                hero_icon_widget -> gtk::Image {
                    set_pixel_size: 32,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    #[local_ref]
                    hero_title_widget -> gtk::Label {
                        add_css_class: "label-large-bold",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    hero_subtitle_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_xalign: 0.0,
                    },
                },

                #[local_ref]
                connectivity_badge_widget -> gtk::Label {
                    add_css_class: "nnetwork-badge",
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Controls ────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Wi-Fi",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                wifi_switch_widget -> gtk::Switch {
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Rescan",
                    connect_clicked[sender] => move |_| {
                        sender.input(NnetworkMenuWidgetInput::Rescan);
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Disconnect",
                    connect_clicked[sender] => move |_| {
                        sender.input(NnetworkMenuWidgetInput::Disconnect);
                    },
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Available networks",
                set_xalign: 0.0,
            },

            // ── Network list ────────────────────────────────────
            gtk::ScrolledWindow {
                set_min_content_height: 240,
                set_max_content_height: 420,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,

                #[local_ref]
                network_list_widget -> gtk::ListBox {
                    add_css_class: "nnetwork-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let hero_icon_widget = gtk::Image::from_icon_name("network-wireless-offline-symbolic");
        let hero_title_widget = gtk::Label::new(Some("Network"));
        let hero_subtitle_widget = gtk::Label::new(Some("…"));
        let connectivity_badge_widget = gtk::Label::new(Some("unknown"));
        let wifi_switch_widget = gtk::Switch::new();
        let network_list_widget = gtk::ListBox::new();

        let toggle_sender = sender.clone();
        let wifi_switch_signal = wifi_switch_widget.connect_state_set(move |_, want_on| {
            toggle_sender.input(NnetworkMenuWidgetInput::SetWifiEnabled(want_on));
            glib::Propagation::Stop
        });

        // Reactive — NetworkManager over D-Bus, no polling.
        spawn_network_watcher(
            &sender,
            || NnetworkMenuWidgetCommandOutput::NetworkChanged,
            || NnetworkMenuWidgetCommandOutput::WifiChanged,
            || NnetworkMenuWidgetCommandOutput::WiredChanged,
        );

        let mut model = NnetworkMenuWidgetModel {
            state: read_network_state(),
            hero_icon: hero_icon_widget.clone(),
            hero_title: hero_title_widget.clone(),
            hero_subtitle: hero_subtitle_widget.clone(),
            connectivity_badge: connectivity_badge_widget.clone(),
            wifi_switch: wifi_switch_widget.clone(),
            wifi_switch_signal,
            network_list: network_list_widget.clone(),
            wifi_watcher_token: WatcherToken::new(),
            wired_watcher_token: WatcherToken::new(),
        };
        arm_wifi_watchers(&sender, &mut model.wifi_watcher_token);
        arm_wired_watcher(&sender, &mut model.wired_watcher_token);

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NnetworkMenuWidgetInput::SetWifiEnabled(on) => {
                let arg = if on { "on" } else { "off" };
                run_nmcli(vec!["radio".into(), "wifi".into(), arg.into()], sender.clone());
            }
            NnetworkMenuWidgetInput::Rescan => {
                run_nmcli(
                    vec!["device".into(), "wifi".into(), "rescan".into()],
                    sender.clone(),
                );
            }
            NnetworkMenuWidgetInput::Connect(ssid) => {
                run_nmcli(
                    vec![
                        "device".into(),
                        "wifi".into(),
                        "connect".into(),
                        ssid,
                    ],
                    sender.clone(),
                );
            }
            NnetworkMenuWidgetInput::Disconnect => {
                let name = self.state.active_name.clone();
                if !name.is_empty() {
                    run_nmcli(
                        vec!["connection".into(), "down".into(), name],
                        sender.clone(),
                    );
                }
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
        match message {
            NnetworkMenuWidgetCommandOutput::NetworkChanged => {}
            NnetworkMenuWidgetCommandOutput::WifiChanged => {
                arm_wifi_watchers(&sender, &mut self.wifi_watcher_token);
            }
            NnetworkMenuWidgetCommandOutput::WiredChanged => {
                arm_wired_watcher(&sender, &mut self.wired_watcher_token);
            }
        }
        let state = read_network_state();
        if self.state != state {
            self.state = state;
            sync_view(self, &sender);
        }
    }
}

/// Arm (or re-arm) the Wi-Fi device sub-watchers — enabled /
/// connectivity / ssid / strength plus the scanned-AP list.
/// The top-level `spawn_network_watcher` only re-fires on
/// hot-plug, so this picks up everything in between.
fn arm_wifi_watchers(
    sender: &ComponentSender<NnetworkMenuWidgetModel>,
    token: &mut WatcherToken,
) {
    let t = token.reset();
    spawn_wifi_watcher(sender, t.clone(), || {
        NnetworkMenuWidgetCommandOutput::NetworkChanged
    });
    spawn_available_wifi_networks_watcher(sender, t, || {
        NnetworkMenuWidgetCommandOutput::NetworkChanged
    });
}

fn arm_wired_watcher(
    sender: &ComponentSender<NnetworkMenuWidgetModel>,
    token: &mut WatcherToken,
) {
    let t = token.reset();
    spawn_wired_watcher(sender, t, || NnetworkMenuWidgetCommandOutput::NetworkChanged);
}

/// Run `nmcli <args…>` once, on a user action. The panel
/// refresh comes from the D-Bus watchers picking up the
/// NetworkManager state change — no re-probe here.
fn run_nmcli(args: Vec<String>, _sender: ComponentSender<NnetworkMenuWidgetModel>) {
    tokio::spawn(async move {
        match tokio::process::Command::new("nmcli")
            .args(&args)
            .status()
            .await
        {
            Ok(s) if s.success() => {}
            Ok(s) => warn!(?s, ?args, "nmcli action returned non-zero"),
            Err(e) => warn!(error = %e, ?args, "nmcli spawn failed"),
        }
    });
}

fn sync_view(model: &NnetworkMenuWidgetModel, sender: &ComponentSender<NnetworkMenuWidgetModel>) {
    let s = &model.state;

    // Hero.
    let (icon, title, subtitle) = match s.active_kind {
        LinkKind::Wifi => (
            wifi_signal_icon(s.active_signal),
            if s.active_name.is_empty() {
                "Wi-Fi".to_string()
            } else {
                s.active_name.clone()
            },
            format!("{}% signal", s.active_signal),
        ),
        LinkKind::Wired => (
            "network-wired-symbolic",
            if s.active_name.is_empty() {
                "Wired".to_string()
            } else {
                s.active_name.clone()
            },
            "connected".to_string(),
        ),
        LinkKind::None => (
            if s.wifi_enabled {
                "network-wireless-offline-symbolic"
            } else {
                "network-wireless-disabled-symbolic"
            },
            "Not connected".to_string(),
            if s.wifi_enabled {
                "Pick a network below".to_string()
            } else {
                "Wi-Fi is off".to_string()
            },
        ),
    };
    model.hero_icon.set_icon_name(Some(icon));
    model.hero_title.set_label(&title);
    model.hero_subtitle.set_label(&subtitle);

    let (badge_text, badge_class) = match s.connectivity.as_str() {
        "full" => ("Online", "nnetwork-badge-online"),
        "limited" | "portal" => ("Limited", "nnetwork-badge-limited"),
        "none" => ("Offline", "nnetwork-badge-offline"),
        other => (other, "nnetwork-badge-unknown"),
    };
    model.connectivity_badge.set_label(badge_text);
    model
        .connectivity_badge
        .set_css_classes(&["nnetwork-badge", badge_class]);

    // Wi-Fi switch — block our own handler during the
    // programmatic set so it doesn't loop back into a radio
    // toggle.
    if model.wifi_switch.state() != s.wifi_enabled {
        model.wifi_switch.block_signal(&model.wifi_switch_signal);
        model.wifi_switch.set_state(s.wifi_enabled);
        model.wifi_switch.set_active(s.wifi_enabled);
        model.wifi_switch.unblock_signal(&model.wifi_switch_signal);
    }

    // Network list.
    while let Some(child) = model.network_list.first_child() {
        model.network_list.remove(&child);
    }
    if !s.available {
        model
            .network_list
            .append(&placeholder_row("NetworkManager not available"));
    } else if !s.wifi_enabled {
        model
            .network_list
            .append(&placeholder_row("(Wi-Fi is off)"));
    } else if s.networks.is_empty() {
        model
            .network_list
            .append(&placeholder_row("(no networks found — try Rescan)"));
    } else {
        for net in &s.networks {
            model.network_list.append(&make_network_row(net, sender));
        }
    }
}

fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("label-small");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    row.set_child(Some(&label));
    row
}

fn make_network_row(
    net: &WifiNetwork,
    sender: &ComponentSender<NnetworkMenuWidgetModel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(5)
        .margin_bottom(5)
        .margin_start(8)
        .margin_end(8)
        .build();

    outer.append(&gtk::Image::from_icon_name(wifi_signal_icon(net.signal)));

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::new(Some(&net.ssid));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let detail = gtk::Label::new(Some(&format!(
        "{}%{}",
        net.signal,
        if net.secured { "  ·  secured" } else { "  ·  open" }
    )));
    detail.add_css_class("label-small");
    detail.set_xalign(0.0);
    texts.append(&detail);
    outer.append(&texts);

    if net.secured {
        let lock = gtk::Image::from_icon_name("system-lock-screen-symbolic");
        lock.add_css_class("nnetwork-lock");
        outer.append(&lock);
    }

    let connect = if net.in_use {
        let b = gtk::Button::with_label("Connected");
        b.add_css_class("ok-button-surface");
        b.add_css_class("selected");
        b.set_sensitive(false);
        b
    } else {
        let b = gtk::Button::with_label("Connect");
        b.add_css_class("ok-button-surface");
        let ssid = net.ssid.clone();
        let s = sender.clone();
        b.connect_clicked(move |_| {
            s.input(NnetworkMenuWidgetInput::Connect(ssid.clone()));
        });
        b
    };
    connect.set_valign(gtk::Align::Center);
    outer.append(&connect);

    row.set_child(Some(&outer));
    row
}
