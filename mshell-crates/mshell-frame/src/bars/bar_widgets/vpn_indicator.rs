//! Generic VPN-up indicator bar pill.
//!
//! A "a VPN tunnel is up" cue for *generic* tunnels — OpenVPN
//! (`tun*`) and WireGuard (`wg*`: wg-quick, NetworkManager, …). The
//! whole pill is hidden while no tunnel is up so it never clutters
//! the bar at rest; when a tunnel comes up it shows a single VPN
//! glyph tinted with the theme accent (the `.connected` class →
//! `--primary`), with the active interface(s) in the tooltip.
//!
//! Clicking the pill opens the layer-shell `MenuType::VpnIndicator`
//! detail menu (`menu_widgets/vpn_indicator/vpn_indicator_menu_widget.rs`),
//! which lists one card per active tunnel with its type, local
//! tunnel IP(s), and live RX/TX throughput. All detail is gathered
//! from **local** sources only (`/sys/class/net` + `getifaddrs`) —
//! there is no network call anywhere in this widget.
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
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
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
    Clicked,
}

#[derive(Debug)]
pub(crate) enum VpnIndicatorOutput {
    Clicked,
}

pub(crate) struct VpnIndicatorInit {
    pub(crate) orientation: Orientation,
}

impl VpnIndicatorModel {
    /// Root CSS classes; adds `.connected` while a tunnel is up so the
    /// glyph tints `--primary` (DESIGN.md §3).
    fn css_classes(&self) -> &'static [&'static str] {
        if self.up {
            &[
                "vpn-indicator-bar-widget",
                "ok-button-surface",
                "ok-bar-widget",
                "connected",
            ]
        } else {
            &[
                "vpn-indicator-bar-widget",
                "ok-button-surface",
                "ok-bar-widget",
            ]
        }
    }
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
            // §3 active-state tinting — a live tunnel adds the
            // `.connected` class so the glyph tints `--primary`
            // (SCSS in `_vpn_indicator.scss`).
            #[watch]
            set_css_classes: model.css_classes(),
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

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(VpnIndicatorInput::Clicked);
                },

                gtk::Image {
                    set_icon_name: Some("network-vpn-symbolic"),
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
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

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnIndicatorInput::Poll => {
                let (up, tooltip) = read_vpn_state();
                if up != self.up || tooltip != self.tooltip {
                    self.up = up;
                    self.tooltip = tooltip;
                }
            }
            VpnIndicatorInput::Clicked => {
                let _ = sender.output(VpnIndicatorOutput::Clicked);
            }
        }
    }
}

// ── Shared data layer ────────────────────────────────────────────────
//
// The gather logic lives here once and is re-used by the menu widget
// (`menu_widgets/vpn_indicator/`) so the pill and its detail panel
// always agree on which tunnels are up. Everything below reads only
// local sources — `/sys/class/net` + `getifaddrs(3)` — never the
// network.

/// The kind of tunnel a generic VPN interface carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VpnKind {
    WireGuard,
    OpenVpn,
}

impl VpnKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            VpnKind::WireGuard => "WireGuard",
            VpnKind::OpenVpn => "OpenVPN",
        }
    }
}

/// One active generic VPN tunnel interface — everything the menu
/// needs, all from local sysfs / `getifaddrs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VpnInterface {
    pub(crate) name: String,
    pub(crate) kind: VpnKind,
    /// Local tunnel IPv4 address(es) assigned to the interface.
    pub(crate) ipv4: Vec<String>,
    /// Local tunnel IPv6 address(es) (link-local `fe80::` excluded).
    pub(crate) ipv6: Vec<String>,
    /// Cumulative received bytes (`statistics/rx_bytes`). The menu
    /// turns successive samples into a throughput rate.
    pub(crate) rx_bytes: u64,
    /// Cumulative transmitted bytes (`statistics/tx_bytes`).
    pub(crate) tx_bytes: u64,
}

/// Enumerate `/sys/class/net` for up generic VPN tunnel interfaces.
///
/// Matches OpenVPN (`tun*`) and WireGuard (`wg*`) interfaces, which
/// wg-quick / OpenVPN / NetworkManager create only while connected.
/// `tap*` is intentionally *not* matched — it's used by VM bridges,
/// not typical VPNs. Mullvad's `wg0-mullvad` is excluded so this
/// stays complementary to the dedicated Mullvad `Vpn` pill. An
/// interface whose `operstate` is `down` counts as inactive
/// (WireGuard's `unknown` still counts as up).
///
/// Returns one [`VpnInterface`] per surviving tunnel, sorted by name.
/// Local details only — no network call.
pub(crate) fn gather_interfaces() -> Vec<VpnInterface> {
    let dir = PathBuf::from("/sys/class/net");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let ip_map = interface_ips();
    let mut out: Vec<VpnInterface> = Vec::new();
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
        let path = entry.path();
        if let Ok(state) = std::fs::read_to_string(path.join("operstate"))
            && state.trim() == "down"
        {
            continue;
        }

        // WireGuard if the name says so OR the driver reports it in
        // `uevent` (covers a NetworkManager `wg`-typed interface with
        // a non-`wg*` name); everything else generic is OpenVPN.
        let uevent = std::fs::read_to_string(path.join("uevent")).unwrap_or_default();
        let kind = if n.starts_with("wg") || uevent.contains("DEVTYPE=wireguard") {
            VpnKind::WireGuard
        } else {
            VpnKind::OpenVpn
        };

        let rx_bytes = read_stat(&path, "rx_bytes");
        let tx_bytes = read_stat(&path, "tx_bytes");
        let (ipv4, ipv6) = ip_map.get(n).cloned().unwrap_or_default();

        out.push(VpnInterface {
            name: n.to_string(),
            kind,
            ipv4,
            ipv6,
            rx_bytes,
            tx_bytes,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Pill-facing summary derived from [`gather_interfaces`].
///
/// Returns `(any_up, tooltip)`; the tooltip is a concise
/// `wg0 · WireGuard · 10.2.0.2` line per active tunnel.
fn read_vpn_state() -> (bool, String) {
    let ifaces = gather_interfaces();
    if ifaces.is_empty() {
        return (false, String::new());
    }
    let lines: Vec<String> = ifaces
        .iter()
        .map(|i| {
            let mut parts = vec![i.name.clone(), i.kind.label().to_string()];
            if let Some(ip) = i.ipv4.first().or_else(|| i.ipv6.first()) {
                parts.push(ip.clone());
            }
            parts.join(" · ")
        })
        .collect();
    (true, lines.join("\n"))
}

fn read_stat(iface_path: &std::path::Path, stat: &str) -> u64 {
    std::fs::read_to_string(iface_path.join("statistics").join(stat))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Map interface name → (IPv4 addrs, IPv6 addrs) via `getifaddrs(3)`.
///
/// `libc` is already a workspace dependency, so this avoids shelling
/// out. Link-local IPv6 (`fe80::/10`) is dropped — it isn't the
/// tunnel's routable address and only adds noise to the detail panel.
fn interface_ips() -> HashMap<String, (Vec<String>, Vec<String>)> {
    let mut map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    // SAFETY: standard `getifaddrs`/`freeifaddrs` pairing. We only read
    // through the linked list while it is live and free it once at the end.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return map;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                .to_string_lossy()
                .into_owned();
            let family = i32::from((*ifa.ifa_addr).sa_family);
            match family {
                libc::AF_INET => {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                    map.entry(name).or_default().0.push(ip.to_string());
                }
                libc::AF_INET6 => {
                    let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                    // Skip link-local `fe80::/10` — not the tunnel IP.
                    if (ip.segments()[0] & 0xffc0) == 0xfe80 {
                        continue;
                    }
                    map.entry(name).or_default().1.push(ip.to_string());
                }
                _ => {}
            }
        }
        libc::freeifaddrs(ifap);
    }
    map
}
