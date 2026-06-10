//! Mullvad VPN bar pill — native, selectable widget driving the `mvpn` binary.
//!
//! Polls `mvpn status --json` every few seconds; shows a shield icon tinted
//! with the accent (`secure` class) when the tunnel is up, with the relay +
//! location in the tooltip. Left-click opens the shell's own native,
//! layer-shell VPN menu (`MenuType::Vpn` — toggled via `VpnOutput::Clicked`,
//! no separate `mvpn menu` popup), which carries the full mvpn control set:
//! connect / random / fastest, lockdown / auto-connect / quantum toggles,
//! anti-censorship, and favourites. Right-click toggles the tunnel
//! (`mvpn toggle`).

use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct VpnState {
    pub(crate) connected: bool,
    pub(crate) relay: String,
    pub(crate) location: String,
    /// Blocky DNS guard active (systemctl `is-active blocky.service`).
    pub(crate) blocky: bool,
    /// Short upstream-resolver name (Google / Cloudflare / Quad9 / …) when a
    /// known DNS is set and the tunnel is down; empty otherwise.
    pub(crate) dns_label: String,
}

#[derive(Debug)]
pub(crate) struct VpnModel {
    state: VpnState,
}

#[derive(Debug)]
pub(crate) enum VpnInput {
    OpenMenu,
    Toggle,
}

/// Emitted to the bar → frame so the native "DNS / VPN" layer-shell menu is
/// toggled (mirrors `DnsOutput::Clicked`).
#[derive(Debug)]
pub(crate) enum VpnOutput {
    Clicked,
}

pub(crate) struct VpnInit {}

#[derive(Debug)]
pub(crate) enum VpnCommandOutput {
    Refreshed(VpnState),
}

#[relm4::component(pub)]
impl Component for VpnModel {
    type CommandOutput = VpnCommandOutput;
    type Input = VpnInput;
    type Output = VpnOutput;
    type Init = VpnInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "vpn-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(VpnInput::OpenMenu);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name="image"]
                    gtk::Image {
                        set_valign: gtk::Align::Center,
                    },
                    // Country label — shown only while connected (see apply_visual).
                    #[name="label"]
                    gtk::Label {
                        add_css_class: "vpn-bar-label",
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
        // Right-click → toggle the tunnel.
        let right = gtk::GestureClick::new();
        right.set_button(3);
        {
            let sender = sender.clone();
            right.connect_pressed(move |_, _, _, _| sender.input(VpnInput::Toggle));
        }
        root.add_controller(right);

        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            loop {
                let delay = if first {
                    STARTUP_DELAY
                } else {
                    REFRESH_INTERVAL
                };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let _ = out.send(VpnCommandOutput::Refreshed(probe().await));
            }
        });

        let model = VpnModel {
            state: VpnState::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &widgets.label, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnInput::OpenMenu => {
                // Toggle the shell's native layer-shell DNS/VPN menu rather
                // than spawning the standalone `mvpn menu` popup.
                let _ = sender.output(VpnOutput::Clicked);
            }
            VpnInput::Toggle => {
                // Toggle off-thread, then refresh sooner than the poll cycle.
                sender.command(|out, _| async move {
                    let _ = tokio::process::Command::new("mvpn")
                        .arg("toggle")
                        .status()
                        .await;
                    let _ = out.send(VpnCommandOutput::Refreshed(probe().await));
                });
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
            VpnCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, &widgets.label, root, &self.state);
                }
            }
        }
    }
}

/// The country half of a Mullvad "Visible location" string
/// ("Sweden, Stockholm. IPv4: …" → "Sweden").
fn country_of(location: &str) -> &str {
    location.split(',').next().unwrap_or("").trim()
}

fn apply_visual(image: &gtk::Image, label: &gtk::Label, root: &gtk::Box, s: &VpnState) {
    let icon = if s.connected {
        "network-vpn-symbolic"
    } else {
        "network-vpn-disconnected-symbolic"
    };
    image.set_icon_name(Some(icon));

    // Beside-icon label: country while on Mullvad; "Blocky" when the DNS guard
    // is up; the upstream resolver's short name when a known DNS is set; else
    // hidden.
    let label_text = if s.connected {
        country_of(&s.location).to_string()
    } else if s.blocky {
        "Blocky".to_string()
    } else {
        s.dns_label.clone()
    };
    label.set_label(&label_text);
    label.set_visible(!label_text.is_empty());

    let tooltip = if s.connected {
        if s.location.is_empty() {
            format!("Mullvad VPN · {}", s.relay)
        } else {
            format!("Mullvad VPN · {} · {}", s.relay, s.location)
        }
    } else if s.blocky {
        "Blocky DNS guard active · VPN disconnected".to_string()
    } else if !s.dns_label.is_empty() {
        format!("DNS: {} · VPN disconnected", s.dns_label)
    } else {
        "Mullvad VPN · disconnected".to_string()
    };
    root.set_tooltip_text(Some(&tooltip));

    // Accent tint when the tunnel is up (same `secure` class the dns pill uses).
    root.remove_css_class("secure");
    if s.connected {
        root.add_css_class("secure");
    }
}

/// `mvpn status --json` → VpnState. Field scan avoids a serde dependency for
/// three values; missing `mvpn` leaves the state disconnected.
async fn probe() -> VpnState {
    let mut state = match tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await
    {
        Ok(out) => parse(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => VpnState::default(),
    };
    // Blocky DNS guard state — cheap, unprivileged.
    state.blocky = tokio::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "blocky.service"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    // Only surface a plain-DNS label when neither Mullvad nor blocky owns DNS.
    if !state.connected && !state.blocky {
        state.dns_label = dns_short(&resolv_nameservers());
    }
    state
}

fn parse(json: &str) -> VpnState {
    VpnState {
        connected: json.contains("\"connected\":true"),
        relay: json_str(json, "relay"),
        location: json_str(json, "location"),
        blocky: false,
        dns_label: String::new(),
    }
}

/// Nameserver IPs from `/etc/resolv.conf`, space-joined. Empty on any error.
fn resolv_nameservers() -> String {
    std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .map(|ip| ip.trim().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a resolver IP set to a well-known provider's short name (empty if none).
fn dns_short(ips: &str) -> String {
    const TABLE: &[(&str, &str)] = &[
        ("Cloudflare", "1.1.1.1 1.0.0.1 2606:4700:4700::1111"),
        ("Google", "8.8.8.8 8.8.4.4 2001:4860:4860::8888"),
        ("Quad9", "9.9.9.9 149.112.112.112 2620:fe::fe"),
        ("AdGuard", "94.140.14.14 94.140.15.15"),
        ("OpenDNS", "208.67.222.222 208.67.220.220"),
        ("NextDNS", "45.90.28.0 45.90.30.0"),
    ];
    let have: std::collections::HashSet<&str> = ips.split_whitespace().collect();
    for (name, list) in TABLE {
        if list.split_whitespace().any(|ip| have.contains(ip)) {
            return (*name).to_string();
        }
    }
    String::new()
}

/// Extract `"<key>":"<value>"` from a flat JSON object.
fn json_str(json: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let Some(i) = json.find(&needle) else {
        return String::new();
    };
    let rest = &json[i + needle.len()..];
    match rest.find('"') {
        Some(end) => rest[..end].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connected() {
        let s = parse(
            r#"{"connected":true,"relay":"de-fra-wg-002","location":"Germany, Frankfurt. IPv4: 1.2.3.4"}"#,
        );
        assert!(s.connected);
        assert_eq!(s.relay, "de-fra-wg-002");
        assert!(s.location.starts_with("Germany"));
    }

    #[test]
    fn parses_disconnected() {
        let s = parse(r#"{"connected":false,"relay":"","location":""}"#);
        assert!(!s.connected);
        assert_eq!(s.relay, "");
    }
}
