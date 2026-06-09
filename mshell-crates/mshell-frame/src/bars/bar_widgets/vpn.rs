//! Mullvad VPN bar pill — native, selectable widget driving the `mvpn` binary.
//!
//! Polls `mvpn status --json` every few seconds; shows a shield icon tinted
//! with the accent (`secure` class) when the tunnel is up, with the relay +
//! location in the tooltip. Left-click opens the native, layer-shell "DNS /
//! VPN" menu (the frame toggles it via `VpnOutput::Clicked` → just like the
//! DNS pill — no separate `mvpn menu` popup); right-click toggles the tunnel
//! (`mvpn toggle`). The richer relay management (favourites, fastest,
//! anti-censorship, account/device) lives in Settings → VPN.

use relm4::gtk::prelude::{ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct VpnState {
    pub(crate) connected: bool,
    pub(crate) relay: String,
    pub(crate) location: String,
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
        apply_visual(&widgets.image, &root, &model.state);
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
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &VpnState) {
    let icon = if s.connected {
        "network-vpn-symbolic"
    } else {
        "network-vpn-disconnected-symbolic"
    };
    image.set_icon_name(Some(icon));

    let tooltip = if s.connected {
        if s.location.is_empty() {
            format!("Mullvad VPN · {}", s.relay)
        } else {
            format!("Mullvad VPN · {} · {}", s.relay, s.location)
        }
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
    let out = tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await;
    let Ok(out) = out else {
        return VpnState::default();
    };
    let body = String::from_utf8_lossy(&out.stdout);
    parse(&body)
}

fn parse(json: &str) -> VpnState {
    VpnState {
        connected: json.contains("\"connected\":true"),
        relay: json_str(json, "relay"),
        location: json_str(json, "location"),
    }
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
