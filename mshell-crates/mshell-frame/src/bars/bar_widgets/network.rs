//! Network Console bar pill — port of the noctalia `network`
//! plugin's bar half.
//!
//! Render-only widget. Reactive: link state / Wi-Fi / scanned
//! APs come from `network_service()` (NetworkManager over D-Bus)
//! — no `nmcli` polling. Click emits `NetworkOutput::Clicked`;
//! frame toggles `MenuType::Network`. The Wi-Fi list + connect /
//! disconnect / rescan / radio-toggle actions live in the menu
//! widget.
//!
//! Live throughput is the one thing NetworkManager doesn't give
//! cheaply, so the `↓ … ↑ …` figure is still sampled from
//! `/proc/net/dev` on a 1 s loop — a couple of file reads, no
//! subprocess.

use mshell_common::WatcherToken;
use mshell_services::network_service;
use mshell_utils::network::{spawn_network_watcher, spawn_wifi_watcher, spawn_wired_watcher};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureSingleExt, IsA, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use wayle_network::types::connectivity::ConnectionType;
use wayle_network::types::states::NetworkStatus;
use wayle_network::core::access_point::SecurityType;

/// Live throughput sampling cadence — 1 s gives a readable
/// KB/s figure without the number jittering too fast to read.
const SPEED_INTERVAL: Duration = Duration::from_secs(1);

/// Bar-pill display mode. Right-click toggles between the two:
///   * `Speed` — live `↓ … ↑ …` throughput text (the default).
///   * `Icon`  — the signal-strength / link-state glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayMode {
    Speed,
    Icon,
}

/// One throughput sample — bytes/s down + up, computed from the
/// delta between two `/proc/net/dev` reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SpeedSample {
    pub(crate) down_bps: u64,
    pub(crate) up_bps: u64,
}

/// One scanned Wi-Fi network row from the NetworkManager AP list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WifiNetwork {
    pub(crate) ssid: String,
    /// 0..=100 signal percentage.
    pub(crate) signal: u8,
    /// True when the AP advertises any security (WPA/WEP/etc).
    pub(crate) secured: bool,
    /// True when this is the currently-connected network.
    pub(crate) in_use: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkKind {
    Wifi,
    Wired,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkState {
    /// A NetworkManager device (wifi or wired) is present.
    pub(crate) available: bool,
    /// Wi-Fi radio on/off.
    pub(crate) wifi_enabled: bool,
    /// "full" / "limited" / "none" / "unknown".
    pub(crate) connectivity: String,
    /// Type of the active primary connection.
    pub(crate) active_kind: LinkKind,
    /// Active connection name (SSID for wifi, "Wired" for wired).
    pub(crate) active_name: String,
    /// Signal % of the active wifi link (0 when wired / down).
    pub(crate) active_signal: u8,
    /// Scanned networks (only meaningful when wifi_enabled).
    pub(crate) networks: Vec<WifiNetwork>,
    pub(crate) error: Option<String>,
}

impl Default for NetworkState {
    fn default() -> Self {
        Self {
            available: false,
            wifi_enabled: false,
            connectivity: "unknown".to_string(),
            active_kind: LinkKind::None,
            active_name: String::new(),
            active_signal: 0,
            networks: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NetworkModel {
    state: NetworkState,
    speed: SpeedSample,
    mode: DisplayMode,
    wifi_watcher_token: WatcherToken,
    wired_watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum NetworkInput {
    /// Left-click — open the menu.
    Clicked,
    /// Right-click — flip between Speed / Icon display.
    ToggleMode,
}

#[derive(Debug)]
pub(crate) enum NetworkOutput {
    Clicked,
}

pub(crate) struct NetworkInit {}

#[derive(Debug)]
pub(crate) enum NetworkCommandOutput {
    /// Link / Wi-Fi / AP state changed (a D-Bus watcher fired).
    NetworkChanged,
    /// The Wi-Fi device was (un)plugged — re-arm its sub-watcher.
    WifiChanged,
    /// The wired device was (un)plugged — re-arm its sub-watcher.
    WiredChanged,
    /// A `/proc/net/dev` throughput sample.
    SpeedSampled(SpeedSample),
}

#[relm4::component(pub)]
impl Component for NetworkModel {
    type CommandOutput = NetworkCommandOutput;
    type Input = NetworkInput;
    type Output = NetworkOutput;
    type Init = NetworkInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "network-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NetworkInput::Clicked);
                },

                // Two display modes share the slot; `apply_visual`
                // toggles `set_visible` so exactly one shows. Icon
                // mode is the signal/wired glyph plus live ↓↑
                // activity arrows that brighten with traffic; Speed
                // mode is the `↓ … ↑ …` readout.
                gtk::Box {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name="icon_group"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 3,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,

                        #[name="image"]
                        gtk::Image {
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },
                        #[name="rx_arrow"]
                        gtk::Image {
                            set_css_classes: &["network-arrow", "network-rx"],
                            set_icon_name: Some("go-down-symbolic"),
                            set_pixel_size: 11,
                            set_valign: gtk::Align::Center,
                        },
                        #[name="tx_arrow"]
                        gtk::Image {
                            set_css_classes: &["network-arrow", "network-tx"],
                            set_icon_name: Some("go-up-symbolic"),
                            set_pixel_size: 11,
                            set_valign: gtk::Align::Center,
                        },
                    },

                    // Speed mode: per-direction coloured readout —
                    // ↓ rides the primary accent, ↑ the secondary,
                    // each dimming when its direction is idle.
                    #[name="speed_group"]
                    gtk::Box {
                        add_css_class: "network-speed-group",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,

                        #[name="rx_speed"]
                        gtk::Label {
                            set_css_classes: &["network-speed-label", "network-rx"],
                        },
                        #[name="tx_speed"]
                        gtk::Label {
                            set_css_classes: &["network-speed-label", "network-tx"],
                        },
                    },
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Reactive link / Wi-Fi state — NetworkManager over D-Bus,
        // no polling.
        spawn_network_watcher(
            &sender,
            || NetworkCommandOutput::NetworkChanged,
            || NetworkCommandOutput::WifiChanged,
            || NetworkCommandOutput::WiredChanged,
        );

        // Live throughput poll — independent 1 s loop over
        // `/proc/net/dev`. Keeps a `prev` reading and emits the
        // per-second delta. File reads only, no subprocess.
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut prev = read_net_totals().await;
                loop {
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(SPEED_INTERVAL) => {}
                    }
                    let now = read_net_totals().await;
                    let sample = SpeedSample {
                        down_bps: now.0.saturating_sub(prev.0),
                        up_bps: now.1.saturating_sub(prev.1),
                    };
                    prev = now;
                    let _ = out.send(NetworkCommandOutput::SpeedSampled(sample));
                }
            }
        });

        // Right-click → ToggleMode.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            toggle_sender.input(NetworkInput::ToggleMode);
        });
        root.add_controller(gesture);

        let mut model = NetworkModel {
            state: read_network_state(),
            speed: SpeedSample::default(),
            mode: DisplayMode::Speed,
            wifi_watcher_token: WatcherToken::new(),
            wired_watcher_token: WatcherToken::new(),
        };
        // Arm the per-device sub-watchers for whatever's already
        // present (the top-level watcher only re-fires on
        // hot-plug).
        let wifi_token = model.wifi_watcher_token.reset();
        spawn_wifi_watcher(&sender, wifi_token, || NetworkCommandOutput::NetworkChanged);
        let wired_token = model.wired_watcher_token.reset();
        spawn_wired_watcher(&sender, wired_token, || NetworkCommandOutput::NetworkChanged);

        let widgets = view_output!();
        apply_visual(
            &widgets.image,
            &widgets.icon_group,
            &widgets.rx_arrow,
            &widgets.tx_arrow,
            &widgets.speed_group,
            &widgets.rx_speed,
            &widgets.tx_speed,
            &root,
            &model.state,
            model.speed,
            model.mode,
        );
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkInput::Clicked => {
                let _ = sender.output(NetworkOutput::Clicked);
            }
            NetworkInput::ToggleMode => {
                self.mode = match self.mode {
                    DisplayMode::Speed => DisplayMode::Icon,
                    DisplayMode::Icon => DisplayMode::Speed,
                };
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NetworkCommandOutput::NetworkChanged => {
                self.state = read_network_state();
            }
            NetworkCommandOutput::WifiChanged => {
                let token = self.wifi_watcher_token.reset();
                spawn_wifi_watcher(&sender, token, || NetworkCommandOutput::NetworkChanged);
                self.state = read_network_state();
            }
            NetworkCommandOutput::WiredChanged => {
                let token = self.wired_watcher_token.reset();
                spawn_wired_watcher(&sender, token, || NetworkCommandOutput::NetworkChanged);
                self.state = read_network_state();
            }
            NetworkCommandOutput::SpeedSampled(sample) => {
                self.speed = sample;
            }
        }
        apply_visual(
            &widgets.image,
            &widgets.icon_group,
            &widgets.rx_arrow,
            &widgets.tx_arrow,
            &widgets.speed_group,
            &widgets.rx_speed,
            &widgets.tx_speed,
            root,
            &self.state,
            self.speed,
            self.mode,
        );
    }
}

/// Read the cumulative rx / tx byte counters across all real
/// network interfaces from `/proc/net/dev`. Skips `lo` and the
/// usual virtual prefixes so VPN tunnels / bridges / docker
/// veths don't double-count the physical link's traffic.
pub(crate) async fn read_net_totals() -> (u64, u64) {
    let raw = match tokio::fs::read_to_string("/proc/net/dev").await {
        Ok(r) => r,
        Err(_) => return (0, 0),
    };
    let mut rx_total: u64 = 0;
    let mut tx_total: u64 = 0;
    for line in raw.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo"
            || iface.starts_with("veth")
            || iface.starts_with("br-")
            || iface.starts_with("docker")
            || iface.starts_with("virbr")
        {
            continue;
        }
        let cols: Vec<&str> = rest.split_whitespace().collect();
        // /proc/net/dev column layout: rx bytes is col 0, tx
        // bytes is col 8 (after the 8 rx-side counters).
        if cols.len() >= 9 {
            rx_total += cols[0].parse::<u64>().unwrap_or(0);
            tx_total += cols[8].parse::<u64>().unwrap_or(0);
        }
    }
    (rx_total, tx_total)
}

/// Format a bytes/sec figure into a compact bar-friendly string:
/// `0`, `512B`, `4.2K`, `1.5M`. Whole numbers below 10 of each
/// unit get one decimal; above that they're rounded so the
/// label width stays stable.
pub(crate) fn format_speed(bps: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * 1024;
    if bps >= M {
        let v = bps as f64 / M as f64;
        if v < 10.0 {
            format!("{v:.1}M")
        } else {
            format!("{:.0}M", v)
        }
    } else if bps >= K {
        let v = bps as f64 / K as f64;
        if v < 10.0 {
            format!("{v:.1}K")
        } else {
            format!("{:.0}K", v)
        }
    } else {
        format!("{bps}B")
    }
}

/// Map a signal % to one of the 5 `network-wireless-signal-*`
/// glyphs that ship in the bundled MargoMaterial set.
pub(crate) fn wifi_signal_icon(signal: u8) -> &'static str {
    match signal {
        0 => "network-wireless-signal-none-symbolic",
        1..=25 => "network-wireless-signal-weak-symbolic",
        26..=50 => "network-wireless-signal-ok-symbolic",
        51..=75 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    }
}

/// Snapshot the network state from the D-Bus-backed
/// `network_service()`. Exposed `pub(crate)` so the menu widget
/// reuses it.
pub(crate) fn read_network_state() -> NetworkState {
    let net = network_service();
    let mut state = NetworkState::default();

    let wifi = net.wifi.get();
    let wired = net.wired.get();
    state.available = wifi.is_some() || wired.is_some();

    if let Some(w) = &wifi {
        state.wifi_enabled = w.enabled.get();
    }

    let conn_str = |s: NetworkStatus| match s {
        NetworkStatus::Connected => "full",
        NetworkStatus::Connecting => "limited",
        NetworkStatus::Disconnected => "none",
    };

    let wired_connected = wired
        .as_ref()
        .map(|w| w.connectivity.get() == NetworkStatus::Connected)
        .unwrap_or(false);
    let wifi_connected = wifi
        .as_ref()
        .map(|w| w.connectivity.get() == NetworkStatus::Connected)
        .unwrap_or(false);

    state.active_kind = match net.primary.get() {
        ConnectionType::Wired => LinkKind::Wired,
        ConnectionType::Wifi => LinkKind::Wifi,
        _ => {
            if wired_connected {
                LinkKind::Wired
            } else if wifi_connected {
                LinkKind::Wifi
            } else {
                LinkKind::None
            }
        }
    };

    match state.active_kind {
        LinkKind::Wifi => {
            if let Some(w) = &wifi {
                state.active_name = w.ssid.get().unwrap_or_default();
                state.active_signal = w.strength.get().unwrap_or(0);
                state.connectivity = conn_str(w.connectivity.get()).to_string();
            }
        }
        LinkKind::Wired => {
            state.active_name = "Wired".to_string();
            if let Some(w) = &wired {
                state.connectivity = conn_str(w.connectivity.get()).to_string();
            }
        }
        LinkKind::None => {
            state.connectivity = "none".to_string();
        }
    }

    // Scanned APs — dedup by SSID, strongest first.
    if let Some(w) = &wifi {
        let current = w.ssid.get();
        let mut seen: Vec<String> = Vec::new();
        for ap in w.access_points.get() {
            let ssid = ap.ssid.get().to_string_lossy();
            if ssid.is_empty() || seen.contains(&ssid) {
                continue;
            }
            seen.push(ssid.clone());
            state.networks.push(WifiNetwork {
                in_use: current.as_deref() == Some(ssid.as_str()),
                ssid,
                signal: ap.strength.get(),
                secured: !matches!(ap.security.get(), SecurityType::None),
            });
        }
        state
            .networks
            .sort_by_key(|n| std::cmp::Reverse(n.signal));
    }

    state
}

/// Below this bytes/s a direction reads as quiet — its arrow + speed
/// label gain the `.idle` class (dimmed in SCSS).
const IDLE_THRESHOLD: u64 = 3000;

fn set_idle<W: IsA<gtk::Widget>>(w: &W, bps: u64) {
    if bps >= IDLE_THRESHOLD {
        w.remove_css_class("idle");
    } else {
        w.add_css_class("idle");
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_visual(
    image: &gtk::Image,
    icon_group: &gtk::Box,
    rx_arrow: &gtk::Image,
    tx_arrow: &gtk::Image,
    speed_group: &gtk::Box,
    rx_speed: &gtk::Label,
    tx_speed: &gtk::Label,
    root: &gtk::Box,
    s: &NetworkState,
    speed: SpeedSample,
    mode: DisplayMode,
) {
    // Arrows + speed labels dim when their direction is idle.
    set_idle(rx_arrow, speed.down_bps);
    set_idle(tx_arrow, speed.up_bps);
    set_idle(rx_speed, speed.down_bps);
    set_idle(tx_speed, speed.up_bps);

    let icon = if !s.available {
        "network-wired-disconnected-symbolic"
    } else {
        match s.active_kind {
            LinkKind::Wired => "network-wired-symbolic",
            LinkKind::Wifi => wifi_signal_icon(s.active_signal),
            LinkKind::None => {
                if s.wifi_enabled {
                    "network-wireless-offline-symbolic"
                } else {
                    "network-wireless-disabled-symbolic"
                }
            }
        }
    };
    image.set_icon_name(Some(icon));

    // `↓ …` / `↑ …` live throughput, one coloured label each. Each
    // figure is right-padded to a fixed 5-char field (`{:>5}`) so the
    // pill width stays rock-steady as the digits tick. Kept populated
    // even in Icon mode (hidden) so a mode-flip is instant.
    rx_speed.set_label(&format!("\u{2193}{:>5}", format_speed(speed.down_bps)));
    tx_speed.set_label(&format!("\u{2191}{:>5}", format_speed(speed.up_bps)));

    match mode {
        DisplayMode::Speed => {
            speed_group.set_visible(true);
            icon_group.set_visible(false);
        }
        DisplayMode::Icon => {
            speed_group.set_visible(false);
            icon_group.set_visible(true);
        }
    }

    let tooltip = if let Some(err) = &s.error {
        format!("Network: {err}")
    } else {
        let mut lines = Vec::with_capacity(3);
        match s.active_kind {
            LinkKind::Wifi => lines.push(format!(
                "Wi-Fi: {} ({}%)",
                if s.active_name.is_empty() {
                    "connected"
                } else {
                    &s.active_name
                },
                s.active_signal
            )),
            LinkKind::Wired => lines.push(format!(
                "Wired: {}",
                if s.active_name.is_empty() {
                    "connected"
                } else {
                    &s.active_name
                }
            )),
            LinkKind::None => lines.push(if s.wifi_enabled {
                "Network: not connected".to_string()
            } else {
                "Wi-Fi: off".to_string()
            }),
        }
        lines.push(format!("Connectivity: {}", s.connectivity));
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    root.remove_css_class("online");
    root.remove_css_class("offline");
    match s.active_kind {
        LinkKind::Wifi | LinkKind::Wired => root.add_css_class("online"),
        LinkKind::None => root.add_css_class("offline"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_icon_buckets() {
        assert_eq!(wifi_signal_icon(0), "network-wireless-signal-none-symbolic");
        assert_eq!(wifi_signal_icon(10), "network-wireless-signal-weak-symbolic");
        assert_eq!(wifi_signal_icon(40), "network-wireless-signal-ok-symbolic");
        assert_eq!(wifi_signal_icon(60), "network-wireless-signal-good-symbolic");
        assert_eq!(
            wifi_signal_icon(90),
            "network-wireless-signal-excellent-symbolic"
        );
    }
}
