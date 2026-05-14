//! Network Console bar pill — port of the noctalia `network`
//! plugin's bar half.
//!
//! Render-only widget. Polls NetworkManager via `nmcli` every
//! 10 s and draws a signal-strength icon + tooltip. Click emits
//! `NnetworkOutput::Clicked`; frame toggles `MenuType::Nnetwork`.
//! The Wi-Fi list + connect / disconnect / rescan / radio-toggle
//! actions live in the menu widget.
//!
//! All probes are unprivileged `nmcli` reads — NetworkManager
//! lets any session user query state + connect to a known SSID
//! without root, so no pkexec here.

use relm4::gtk::prelude::{ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const STARTUP_DELAY: Duration = Duration::from_secs(1);
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

/// One scanned Wi-Fi network row from `nmcli device wifi list`.
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
    /// `nmcli` present + the daemon reachable.
    pub(crate) available: bool,
    /// `nmcli radio wifi` → on/off.
    pub(crate) wifi_enabled: bool,
    /// "full" / "limited" / "none" / "portal" / "unknown".
    pub(crate) connectivity: String,
    /// Type of the active primary connection.
    pub(crate) active_kind: LinkKind,
    /// Active connection name (SSID for wifi, profile name for
    /// wired).
    pub(crate) active_name: String,
    /// Active device interface (wlan0 / enp3s0 …).
    pub(crate) active_device: String,
    /// Signal % of the active wifi link (0 when wired / down).
    pub(crate) active_signal: u8,
    /// IPv4 address of the active device.
    pub(crate) ipv4: String,
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
            active_device: String::new(),
            active_signal: 0,
            ipv4: String::new(),
            networks: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NnetworkModel {
    state: NetworkState,
    speed: SpeedSample,
    mode: DisplayMode,
}

#[derive(Debug)]
pub(crate) enum NnetworkInput {
    /// Left-click — open the menu.
    Clicked,
    /// Right-click — flip between Speed / Icon display.
    ToggleMode,
}

#[derive(Debug)]
pub(crate) enum NnetworkOutput {
    Clicked,
}

pub(crate) struct NnetworkInit {}

#[derive(Debug)]
pub(crate) enum NnetworkCommandOutput {
    Refreshed(NetworkState),
    SpeedSampled(SpeedSample),
}

#[relm4::component(pub)]
impl Component for NnetworkModel {
    type CommandOutput = NnetworkCommandOutput;
    type Input = NnetworkInput;
    type Output = NnetworkOutput;
    type Init = NnetworkInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "nnetwork-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NnetworkInput::Clicked);
                },

                // Speed text + signal icon share the same slot;
                // `apply_visual` toggles `set_visible` so exactly
                // one shows at a time.
                gtk::Box {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },

                    #[name="speed_label"]
                    gtk::Label {
                        add_css_class: "nnetwork-speed-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
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
        // NetworkManager state poll (icon / tooltip).
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    // Bar poll skips the (slower) wifi scan; the
                    // menu widget runs the full probe when open.
                    let s = probe_network_state(false).await;
                    let _ = out.send(NnetworkCommandOutput::Refreshed(s));
                }
            }
        });

        // Live throughput poll — independent 1 s loop over
        // `/proc/net/dev`. Keeps a `prev` reading and emits the
        // per-second delta.
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
                    let _ = out.send(NnetworkCommandOutput::SpeedSampled(sample));
                }
            }
        });

        // Right-click → ToggleMode.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            toggle_sender.input(NnetworkInput::ToggleMode);
        });
        root.add_controller(gesture);

        let model = NnetworkModel {
            state: NetworkState::default(),
            speed: SpeedSample::default(),
            mode: DisplayMode::Speed,
        };
        let widgets = view_output!();
        apply_visual(
            &widgets.image,
            &widgets.speed_label,
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
            NnetworkInput::Clicked => {
                let _ = sender.output(NnetworkOutput::Clicked);
            }
            NnetworkInput::ToggleMode => {
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
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NnetworkCommandOutput::Refreshed(state) => {
                self.state = state;
            }
            NnetworkCommandOutput::SpeedSampled(sample) => {
                self.speed = sample;
            }
        }
        apply_visual(
            &widgets.image,
            &widgets.speed_label,
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
async fn read_net_totals() -> (u64, u64) {
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
fn format_speed(bps: u64) -> String {
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
/// glyphs that ship in the bundled OkMaterial set.
pub(crate) fn wifi_signal_icon(signal: u8) -> &'static str {
    match signal {
        0 => "network-wireless-signal-none-symbolic",
        1..=25 => "network-wireless-signal-weak-symbolic",
        26..=50 => "network-wireless-signal-ok-symbolic",
        51..=75 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    }
}

fn apply_visual(
    image: &gtk::Image,
    speed_label: &gtk::Label,
    root: &gtk::Box,
    s: &NetworkState,
    speed: SpeedSample,
    mode: DisplayMode,
) {
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

    // `↓ … ↑ …` live throughput. Each figure is right-padded to
    // a fixed 5-char field (`{:>5}`) so the pill width stays
    // rock-steady as the digits tick — only the numbers move,
    // the widget never reflows the bar. Kept populated even in
    // Icon mode (hidden) so a mode-flip is instant.
    speed_label.set_label(&format!(
        "\u{2193}{:>5} \u{2191}{:>5}",
        format_speed(speed.down_bps),
        format_speed(speed.up_bps)
    ));

    // Tint the readout with the matugen accent once either
    // direction crosses 1 MB/s — a quiet "this is real traffic
    // now" signal without changing the layout. Sub-MB stays the
    // plain on-surface tone via the base `.online` rule.
    const ONE_MB: u64 = 1024 * 1024;
    if speed.down_bps >= ONE_MB || speed.up_bps >= ONE_MB {
        speed_label.add_css_class("high-rate");
    } else {
        speed_label.remove_css_class("high-rate");
    }

    match mode {
        DisplayMode::Speed => {
            speed_label.set_visible(true);
            image.set_visible(false);
        }
        DisplayMode::Icon => {
            speed_label.set_visible(false);
            image.set_visible(true);
        }
    }

    let tooltip = if let Some(err) = &s.error {
        format!("Network: {err}")
    } else {
        let mut lines = Vec::with_capacity(4);
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
        if !s.ipv4.is_empty() {
            lines.push(format!("IP: {}", s.ipv4));
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

/// Aggregate `nmcli` probe. `with_scan` controls whether the
/// (slower, ~1-2 s) Wi-Fi list scan runs — the bar poll skips
/// it, the menu widget enables it. Exposed pub(crate) so the
/// menu widget reuses the exact same parse path.
pub(crate) async fn probe_network_state(with_scan: bool) -> NetworkState {
    let mut state = NetworkState::default();

    // nmcli availability + daemon reachable.
    if run_capture("nmcli", &["-t", "-f", "RUNNING", "general"])
        .await
        .map(|o| o.trim() == "running")
        .unwrap_or(false)
    {
        state.available = true;
    } else {
        state.error = Some("NetworkManager not available".to_string());
        return state;
    }

    // Connectivity.
    if let Some(out) = run_capture("nmcli", &["-t", "-f", "STATE,CONNECTIVITY", "general"]).await {
        if let Some(line) = out.lines().next() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() > 1 {
                state.connectivity = parts[1].to_lowercase();
            }
        }
    }

    // Radio state.
    if let Some(out) = run_capture("nmcli", &["radio", "wifi"]).await {
        state.wifi_enabled = out.trim().eq_ignore_ascii_case("enabled");
    }

    // Active device — pick the first wifi or ethernet device in
    // `connected` / `connecting` state, preferring wifi.
    if let Some(out) =
        run_capture("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).await
    {
        let mut best: Option<(LinkKind, String, String)> = None;
        for line in out.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() < 4 {
                continue;
            }
            let (device, kind_str, dev_state, conn) =
                (parts[0], parts[1], parts[2], parts[3]);
            if !(dev_state == "connected" || dev_state == "connecting") {
                continue;
            }
            let kind = match kind_str {
                "wifi" => LinkKind::Wifi,
                "ethernet" => LinkKind::Wired,
                _ => continue,
            };
            let prefer = match (&best, kind) {
                (None, _) => true,
                (Some((LinkKind::Wired, _, _)), LinkKind::Wifi) => true,
                _ => false,
            };
            if prefer {
                best = Some((kind, device.to_string(), conn.to_string()));
            }
        }
        if let Some((kind, device, conn)) = best {
            state.active_kind = kind;
            state.active_device = device;
            state.active_name = conn;
        }
    }

    // IPv4 of the active device.
    if !state.active_device.is_empty() {
        if let Some(out) = run_capture(
            "nmcli",
            &["-g", "IP4.ADDRESS", "device", "show", &state.active_device],
        )
        .await
        {
            if let Some(addr) = out.lines().next() {
                // nmcli prints `192.168.1.5/24` — strip the mask.
                state.ipv4 = addr.split('/').next().unwrap_or("").trim().to_string();
            }
        }
    }

    // Wi-Fi scan list.
    if state.wifi_enabled {
        let rescan = if with_scan { "yes" } else { "no" };
        if let Some(out) = run_capture(
            "nmcli",
            &[
                "-t",
                "-f",
                "IN-USE,SIGNAL,SECURITY,SSID",
                "device",
                "wifi",
                "list",
                "--rescan",
                rescan,
            ],
        )
        .await
        {
            let mut seen: Vec<String> = Vec::new();
            for line in out.lines() {
                // nmcli -t escapes embedded ':' in fields with
                // '\:'; split on unescaped ':' only.
                let parts = split_nmcli_terse(line);
                if parts.len() < 4 {
                    continue;
                }
                let in_use = parts[0].trim() == "*";
                let signal = parts[1].trim().parse::<u8>().unwrap_or(0);
                let security = parts[2].trim();
                let ssid = parts[3].trim().to_string();
                if ssid.is_empty() || seen.contains(&ssid) {
                    continue;
                }
                seen.push(ssid.clone());
                state.networks.push(WifiNetwork {
                    ssid,
                    signal,
                    secured: !security.is_empty() && security != "--",
                    in_use,
                });
                if in_use {
                    state.active_signal = signal;
                }
            }
            // Strongest first.
            state.networks.sort_by(|a, b| b.signal.cmp(&a.signal));
        }
    }

    state
}

/// Split an `nmcli -t` line on unescaped `:` separators. nmcli
/// escapes literal colons inside a field as `\:`; a naive
/// `split(':')` would chop SSIDs / security strings that contain
/// them.
fn split_nmcli_terse(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    current.push(next);
                    chars.next();
                }
            }
            ':' => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

async fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_split_handles_escaped_colons() {
        // SSID "my:net" should survive as a single field.
        let parts = split_nmcli_terse(r"*:72:WPA2:my\:net");
        assert_eq!(parts, vec!["*", "72", "WPA2", "my:net"]);
    }

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
