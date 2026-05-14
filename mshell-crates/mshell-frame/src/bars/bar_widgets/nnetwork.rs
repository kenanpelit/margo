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

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

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
}

#[derive(Debug)]
pub(crate) enum NnetworkInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NnetworkOutput {
    Clicked,
}

pub(crate) struct NnetworkInit {}

#[derive(Debug)]
pub(crate) enum NnetworkCommandOutput {
    Refreshed(NetworkState),
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

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
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

        let model = NnetworkModel {
            state: NetworkState::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);
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
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
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

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &NetworkState) {
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
