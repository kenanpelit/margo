//! Network OSD — flashes briefly when the primary connection
//! type or active Wi-Fi SSID changes.
//!
//! Triggers:
//!   * `None` → `Wifi` / `Wired`  — "Connected: <name>"
//!   * `Wifi` / `Wired` → `None`  — "Disconnected"
//!   * Wifi SSID change (roam)    — "Connected: <new SSID>"
//!
//! Stays gated by `general.network_osd_enabled` so users on
//! systems with NetworkManager notifications can keep it off.
//! The first event after init is suppressed (it's just the
//! existing state being observed, not a real change), so opening
//! a session doesn't pop a spurious "Connected" right at start.
//!
//! Layout-shell window pattern + 2 s auto-hide mirror the
//! volume / brightness OSDs so visually all three feel like the
//! same surface.

use gtk4::gdk;
use gtk4::prelude::{BoxExt, GtkWindowExt, OrientableExt, WidgetExt};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_common::{WatcherToken, watch};
use mshell_services::network_service;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use wayle_network::types::connectivity::ConnectionType;

const HIDE_AFTER: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetState {
    None,
    Wifi(Option<String>),
    Wired,
}

#[derive(Debug)]
pub struct NetworkOsdModel {
    hide_token: WatcherToken,
    icon_name: String,
    title: String,
    subtitle: String,
    /// Last rendered state. Initial network reading lands here at
    /// init time without showing the OSD — the surface only
    /// appears on subsequent *changes*.
    last_state: NetState,
}

#[derive(Debug)]
pub enum NetworkOsdInput {
    Show,
    Hide,
}

#[derive(Debug)]
pub enum NetworkOsdOutput {}

pub struct NetworkOsdInit {
    pub monitor: gdk::Monitor,
}

#[derive(Debug)]
pub enum NetworkOsdCommandOutput {
    /// Network state changed. The payload is the new snapshot,
    /// already classified so `update_cmd` can decide whether to
    /// show + with what label.
    StateChanged(NetState),
    Hide,
}

#[relm4::component(pub)]
impl Component for NetworkOsdModel {
    type CommandOutput = NetworkOsdCommandOutput;
    type Input = NetworkOsdInput;
    type Output = NetworkOsdOutput;
    type Init = NetworkOsdInit;

    view! {
        #[root]
        gtk::Window {
            set_css_classes: &["osd-window", "window-opacity"],
            set_decorated: false,
            set_visible: false,
            set_default_height: 1,
            set_margin_bottom: 48,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_width_request: 300,
                set_spacing: 16,

                gtk::Image {
                    add_css_class: "osd-icon",
                    #[watch]
                    set_icon_name: Some(model.icon_name.as_str()),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_label: model.title.as_str(),
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_label: model.subtitle.as_str(),
                        #[watch]
                        set_visible: !model.subtitle.is_empty(),
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_namespace(Some("mshell-osd"));
        root.set_layer(Layer::Overlay);
        root.set_exclusive_zone(0);
        root.set_anchor(Edge::Bottom, true);

        // Subscribe to the relevant network properties. Three
        // streams are merged: primary-connection-type, wifi SSID,
        // and the wifi-Option itself (so we re-bind when the
        // wifi adapter appears or disappears). On any tick the
        // handler re-reads everything and emits the new
        // classified snapshot.
        let svc = network_service();
        let primary = svc.primary.clone();
        let wifi = svc.wifi.clone();
        watch!(sender, [primary.watch(), wifi.watch()], |out| {
            let snap = snapshot_state();
            let _ = out.send(NetworkOsdCommandOutput::StateChanged(snap));
        });

        let model = NetworkOsdModel {
            hide_token: WatcherToken::new(),
            icon_name: "network-wired-symbolic".to_string(),
            title: String::new(),
            subtitle: String::new(),
            last_state: snapshot_state(),
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NetworkOsdInput::Show => {
                if !is_enabled() {
                    return;
                }
                root.set_visible(true);
                let token = self.hide_token.reset();
                sender.command(|out, shutdown| {
                    shutdown
                        .register(async move {
                            tokio::time::sleep(HIDE_AFTER).await;
                            if !token.is_cancelled() {
                                out.send(NetworkOsdCommandOutput::Hide).ok();
                            }
                        })
                        .drop_on_shutdown()
                });
            }
            NetworkOsdInput::Hide => {
                root.set_visible(false);
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkOsdCommandOutput::StateChanged(new) => {
                if new == self.last_state {
                    // Subscriber re-fired with no meaningful diff
                    // (a property update we don't care about,
                    // e.g. signal-strength wobble on stable wifi).
                    return;
                }
                let previous = std::mem::replace(&mut self.last_state, new.clone());
                self.icon_name = icon_for(&new);
                self.title = title_for(&previous, &new);
                self.subtitle = subtitle_for(&new);
                sender.input(NetworkOsdInput::Show);
            }
            NetworkOsdCommandOutput::Hide => {
                sender.input(NetworkOsdInput::Hide);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn is_enabled() -> bool {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
    use reactive_graph::traits::GetUntracked;
    config_manager()
        .config()
        .general()
        .network_osd_enabled()
        .get_untracked()
}

fn snapshot_state() -> NetState {
    let svc = network_service();
    match svc.primary.get() {
        ConnectionType::Wifi => {
            let ssid = svc.wifi.get().and_then(|w| w.ssid.get());
            NetState::Wifi(ssid)
        }
        ConnectionType::Wired => NetState::Wired,
        // `ConnectionType` is `#[non_exhaustive]`; treat unknown
        // future variants as no-connection so we don't flash a
        // wrong-icon OSD on a kind we haven't seen yet.
        _ => NetState::None,
    }
}

fn icon_for(state: &NetState) -> String {
    match state {
        NetState::Wifi(_) => "network-wireless-signal-good-symbolic".to_string(),
        NetState::Wired => "network-wired-symbolic".to_string(),
        NetState::None => "network-wired-disconnected-symbolic".to_string(),
    }
}

fn title_for(prev: &NetState, new: &NetState) -> String {
    // Connection went down → "Disconnected".
    if matches!(new, NetState::None) && !matches!(prev, NetState::None) {
        return "Disconnected".to_string();
    }
    match new {
        NetState::Wifi(_) => "Wi-Fi connected".to_string(),
        NetState::Wired => "Ethernet connected".to_string(),
        NetState::None => "Disconnected".to_string(),
    }
}

fn subtitle_for(state: &NetState) -> String {
    match state {
        NetState::Wifi(Some(ssid)) if !ssid.is_empty() => ssid.clone(),
        NetState::Wifi(_) => "Unknown network".to_string(),
        NetState::Wired | NetState::None => String::new(),
    }
}
