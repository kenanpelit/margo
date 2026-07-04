//! Generic VPN-up indicator bar pill.
//!
//! A minimal "a VPN tunnel is up" cue for *generic* tunnels —
//! OpenVPN (`tun*`) and WireGuard (`wg*`: wg-quick, NetworkManager,
//! …). The whole pill is hidden while no tunnel is up so it never
//! clutters the bar at rest; when a tunnel comes up it shows a
//! single VPN glyph, with the active interface name(s) in the
//! tooltip.
//!
//! Mullvad is deliberately excluded (its interface is
//! `wg0-mullvad`) — the dedicated `Vpn` (Mullvad) pill already
//! covers it, so this indicator stays complementary and lights up
//! only for the *other* tunnels.
//!
//! State source: `/sys/class/net/*`. wg-quick / OpenVPN /
//! NetworkManager create the tunnel interface on connect and
//! destroy it on disconnect, so the interface's mere presence is
//! the signal (an admin-`down` leftover is filtered out).
//! WireGuard interfaces report `operstate = unknown` — that still
//! counts as up. Polled every 3 s; VPN state changes rarely, so a
//! calm tick keeps it responsive at negligible cost (one
//! `/sys/class/net` readdir per tick).

use relm4::gtk::Orientation;
use relm4::gtk::prelude::WidgetExt;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct VpnIndicatorModel {
    up: bool,
    tooltip: String,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum VpnIndicatorInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum VpnIndicatorOutput {}

pub(crate) struct VpnIndicatorInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for VpnIndicatorModel {
    type CommandOutput = ();
    type Input = VpnIndicatorInput;
    type Output = VpnIndicatorOutput;
    type Init = VpnIndicatorInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "vpn-indicator-bar-widget",
            add_css_class: "ok-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            // Hide the whole pill when no tunnel is up so a
            // disconnected session leaves no dead glyph behind.
            #[watch]
            set_visible: model.up,
            #[watch]
            set_tooltip_text: Some(model.tooltip.as_str()),

            gtk::Image {
                set_icon_name: Some("network-vpn-symbolic"),
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (up, tooltip) = read_vpn_state();

        // Glib main-loop poll. VPN tunnels come and go rarely, so a
        // 3 s tick is plenty responsive.
        //
        // `input_sender().send()` (Result) instead of `s.input()`
        // (panics on a closed channel) so the timer self-cancels
        // when a bar rebuild drops this controller — otherwise a
        // config-driven widget reorder leaves the closure ticking
        // against a dead receiver and the next send aborts mshell.
        {
            let s = sender.clone();
            relm4::gtk::glib::timeout_add_local(Duration::from_secs(3), move || {
                if s.input_sender().send(VpnIndicatorInput::Poll).is_err() {
                    return relm4::gtk::glib::ControlFlow::Break;
                }
                relm4::gtk::glib::ControlFlow::Continue
            });
        }

        let model = VpnIndicatorModel {
            up,
            tooltip,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnIndicatorInput::Poll => {
                let (up, tooltip) = read_vpn_state();
                if up != self.up || tooltip != self.tooltip {
                    self.up = up;
                    self.tooltip = tooltip;
                }
            }
        }
    }
}

/// Scan `/sys/class/net` for an up VPN tunnel interface.
///
/// Matches OpenVPN (`tun*`) and WireGuard (`wg*`) interfaces, which
/// wg-quick / OpenVPN / NetworkManager create only while connected.
/// `tap*` is intentionally *not* matched — it's used by VM bridges,
/// not typical VPNs. Mullvad's `wg0-mullvad` is excluded so this
/// stays complementary to the dedicated Mullvad `Vpn` pill. An
/// interface whose `operstate` is `down` counts as inactive
/// (WireGuard's `unknown` still counts as up).
///
/// Returns `(any_up, tooltip)`, the tooltip naming the active
/// interface(s).
fn read_vpn_state() -> (bool, String) {
    let dir = PathBuf::from("/sys/class/net");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (false, String::new());
    };
    let mut active: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(n) = name.to_str() else { continue };
        let is_tunnel = n.starts_with("tun") || n.starts_with("wg");
        if !is_tunnel {
            continue;
        }
        // Mullvad is surfaced by the dedicated `Vpn` pill.
        if n.to_ascii_lowercase().contains("mullvad") {
            continue;
        }
        if let Ok(state) = std::fs::read_to_string(entry.path().join("operstate"))
            && state.trim() == "down"
        {
            continue;
        }
        active.push(n.to_string());
    }
    active.sort();
    if active.is_empty() {
        (false, String::new())
    } else {
        (true, format!("VPN active: {}", active.join(", ")))
    }
}
