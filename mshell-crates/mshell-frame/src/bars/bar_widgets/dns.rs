//! DNS bar pill — opens the standalone DNS menu (`mshellctl menu dns`).
//!
//! Render-only widget. Polls DNS / VPN / Blocky state every 30 s (via the
//! shared probe in [`crate::menus::menu_widgets::dns::state`]) and draws an
//! icon + tooltip from the result. Click emits `DnsOutput::Clicked`;
//! `frame.rs` toggles the layer-shell `MenuType::Dns` menu in response.
//!
//! Distinct from the combined **VPN** pill (`bar_widgets/vpn.rs`), whose menu
//! folds these DNS controls into a collapsible section — this pill is for
//! users who want a dedicated DNS / Blocky entry on the bar.
//!
//! Icon mapping is "most-secure-wins":
//!   * `firewall-error-symbolic`  — mullvad blocked / revoked
//!   * `shield-check-symbolic`    — VPN + Blocky both up
//!   * `vpn-symbolic`             — VPN up, Blocky down
//!   * `server-symbolic`          — Blocky up, no VPN
//!   * `globe-symbolic`           — custom preset DNS (no VPN)
//!   * `network-wired-symbolic`   — DHCP default

use crate::menus::menu_widgets::dns::state::{DnsState, Mode, probe_dns_state};
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(crate) struct DnsModel {
    state: DnsState,
}

#[derive(Debug)]
pub(crate) enum DnsInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum DnsOutput {
    Clicked,
}

pub(crate) struct DnsInit {}

#[derive(Debug)]
pub(crate) enum DnsCommandOutput {
    Refreshed(DnsState),
}

#[relm4::component(pub)]
impl Component for DnsModel {
    type CommandOutput = DnsCommandOutput;
    type Input = DnsInput;
    type Output = DnsOutput;
    type Init = DnsInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "dns-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(DnsInput::Clicked);
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
                let s = probe_dns_state().await;
                let _ = out.send(DnsCommandOutput::Refreshed(s));
            }
        });

        let model = DnsModel {
            state: DnsState::default(),
        };

        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DnsInput::Clicked => {
                let _ = sender.output(DnsOutput::Clicked);
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
            DnsCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &DnsState) {
    let icon = match s.mode_id() {
        Mode::Blocked => "firewall-error-symbolic",
        Mode::Mixed => "shield-check-symbolic",
        Mode::Mullvad => "vpn-symbolic",
        Mode::Blocky => "server-symbolic",
        Mode::Custom => "globe-symbolic",
        Mode::Default => "network-wired-symbolic",
        Mode::Idle => "globe-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("DNS: {err}")
    } else {
        let mut lines = Vec::with_capacity(4);
        lines.push(format!(
            "VPN: {}",
            if s.blocked {
                "blocked / revoked"
            } else if s.vpn {
                "connected"
            } else {
                "off"
            }
        ));
        lines.push(format!(
            "Blocky: {}",
            if s.blocky { "active" } else { "inactive" }
        ));
        if s.display_dns.is_empty() {
            lines.push("DNS: (none)".to_string());
        } else {
            lines.push(format!(
                "DNS: {}{}",
                s.display_dns,
                if s.auto_dns { " (auto)" } else { "" }
            ));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    // CSS class for bar pill colour. Three-state same as ufw:
    // secure (VPN/Blocky/Mixed) = primary, blocked = red, idle/
    // default/custom = neutral.
    root.remove_css_class("secure");
    root.remove_css_class("blocked");
    root.remove_css_class("custom");
    match s.mode_id() {
        Mode::Blocked => root.add_css_class("blocked"),
        Mode::Mullvad | Mode::Blocky | Mode::Mixed => root.add_css_class("secure"),
        Mode::Custom => root.add_css_class("custom"),
        _ => {}
    }
}
