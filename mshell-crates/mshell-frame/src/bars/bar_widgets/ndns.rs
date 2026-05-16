//! DNS / VPN bar pill — port of the noctalia `ndns` plugin's bar
//! half.
//!
//! Render-only widget. Polls DNS / VPN / Blocky state every 30 s
//! and draws an icon + tooltip from the result. Click emits
//! `NdnsOutput::Clicked`; `frame.rs` toggles the layer-shell
//! `MenuType::Ndns` menu in response (the menu UI lives in
//! `menu_widgets/ndns/ndns_menu_widget.rs`).
//!
//! Icon mapping is "most-secure-wins":
//!   * `firewall-error-symbolic`  — mullvad blocked / revoked
//!   * `shield-check-symbolic`    — VPN + Blocky both up
//!   * `vpn-symbolic`             — VPN up, Blocky down
//!   * `server-symbolic`          — Blocky up, no VPN
//!   * `globe-symbolic`           — custom preset DNS (no VPN)
//!   * `network-wired-symbolic`   — DHCP default
//!
//! Probes are unprivileged: `mullvad status`, `systemctl is-
//! active blocky.service`, `nmcli -g IP4.DNS`, `resolvectl dns`.
//! No pkexec from the bar — privileged actions live in the menu.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

/// "Mode" classification used by both the bar pill icon and the
/// menu widget's status / action highlight. Exposed pub(crate) so
/// the menu can reuse `DnsState::mode_id()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Mullvad,
    Blocky,
    Default,
    Mixed,
    Custom,
    Blocked,
    Idle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DnsState {
    pub(crate) vpn: bool,
    pub(crate) blocky: bool,
    pub(crate) blocked: bool,
    /// Pretty-printed list of nameservers (space-separated).
    pub(crate) display_dns: String,
    pub(crate) auto_dns: bool,
    /// Name of the primary NM connection (used as the link target
    /// for resolvectl / nmcli action subcommands).
    pub(crate) primary_conn: Option<String>,
    /// Device name for the primary connection (resolvectl wants
    /// the interface, not the connection name).
    pub(crate) primary_device: Option<String>,
    pub(crate) error: Option<String>,
}

impl DnsState {
    pub(crate) fn mode_id(&self) -> Mode {
        if self.blocked {
            Mode::Blocked
        } else if self.vpn && self.blocky {
            Mode::Mixed
        } else if self.vpn {
            Mode::Mullvad
        } else if self.blocky {
            Mode::Blocky
        } else if self.auto_dns {
            Mode::Default
        } else if !self.display_dns.is_empty() {
            Mode::Custom
        } else {
            Mode::Idle
        }
    }

    /// True when every IP in `preset_ips` is present in
    /// `display_dns`. Subset-match (not strict equality) because
    /// auxiliary resolvers can be auto-prepended without
    /// invalidating which *preset* the user actively chose:
    ///
    ///   * Mullvad's tunnel resolver `10.64.0.1` is added by the
    ///     VPN client to the head of the global list whenever VPN
    ///     is up. With strict equality the active preset (Google /
    ///     Quad9 / etc.) reads as inactive because the resolver
    ///     row no longer matches exactly.
    ///   * systemd-resolved's per-link DNS can interleave a
    ///     fallback line that doesn't belong to any preset.
    ///
    /// Subset semantics: preset is "active" when every one of its
    /// nameservers is currently resolving. The caller still gets
    /// at most one preset marked Active because the preset IP
    /// lists are mutually disjoint (Google's 8.8.8.8 vs Quad9's
    /// 9.9.9.9 etc.) — a subset hit on one preset can't also hit
    /// another.
    pub(crate) fn matches_preset(&self, preset_ips: &str) -> bool {
        let current: std::collections::HashSet<&str> =
            self.display_dns.split_whitespace().collect();
        let want: Vec<&str> = preset_ips.split_whitespace().collect();
        !want.is_empty() && want.iter().all(|ip| current.contains(ip))
    }
}

#[derive(Debug)]
pub(crate) struct NdnsModel {
    state: DnsState,
}

#[derive(Debug)]
pub(crate) enum NdnsInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NdnsOutput {
    Clicked,
}

pub(crate) struct NdnsInit {}

#[derive(Debug)]
pub(crate) enum NdnsCommandOutput {
    Refreshed(DnsState),
}

#[relm4::component(pub)]
impl Component for NdnsModel {
    type CommandOutput = NdnsCommandOutput;
    type Input = NdnsInput;
    type Output = NdnsOutput;
    type Init = NdnsInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "ndns-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NdnsInput::Clicked);
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
                    let s = probe_dns_state().await;
                    let _ = out.send(NdnsCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NdnsModel {
            state: DnsState::default(),
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
            NdnsInput::Clicked => {
                let _ = sender.output(NdnsOutput::Clicked);
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
            NdnsCommandOutput::Refreshed(state) => {
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

    // CSS class for bar pill colour. Three-state same as nufw:
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

/// Aggregate state probe. Each sub-probe is independently
/// fallible — missing tools leave their bits at default
/// (false / empty). Exposed (pub(crate)) so the menu widget can
/// trigger a fresh probe after each action.
pub(crate) async fn probe_dns_state() -> DnsState {
    let mut state = DnsState::default();

    if let Some(status) = run_capture("mullvad", &["status"]).await {
        if status.contains("Connected") {
            state.vpn = true;
        }
        let lower = status.to_lowercase();
        if lower.contains("blocked:") || lower.contains("device has been revoked") {
            state.blocked = true;
        }
    }

    if run_capture("systemctl", &["is-active", "--quiet", "blocky.service"])
        .await
        .is_some()
    {
        state.blocky = true;
    }

    if let Some((name, device)) = primary_nm_connection().await {
        state.primary_conn = Some(name.clone());
        state.primary_device = Some(device);
        if let Some(dns) =
            run_capture("nmcli", &["-g", "IP4.DNS", "connection", "show", &name]).await
        {
            let cleaned = dns
                .split_whitespace()
                .filter(|s| looks_like_ipv4(s))
                .collect::<Vec<_>>()
                .join(" ");
            if !cleaned.is_empty() {
                state.display_dns = cleaned;
            }
        }
        if let Some(ignore_auto) = run_capture(
            "nmcli",
            &[
                "-g",
                "ipv4.ignore-auto-dns",
                "connection",
                "show",
                &name,
            ],
        )
        .await
        {
            let v = ignore_auto.trim().to_lowercase();
            if v.is_empty() || v == "no" || v == "false" {
                state.auto_dns = true;
            }
        }
    }

    if let Some(resolvectl) = run_capture("resolvectl", &["dns"]).await {
        // `resolvectl dns` output looks like:
        //
        //   Global: 10.64.0.1
        //   Link 2 (wlan0): 9.9.9.9 149.112.112.112
        //   Link 5 (wg0-mullvad):
        //
        // Previous parser only kept the `Global:` line, which
        // under Mullvad VPN held just the tunnel resolver
        // `10.64.0.1`. The user's actually-chosen preset (Quad9
        // / Google / etc.) lives on the per-link line, so the
        // `matches_preset` subset check missed it and the
        // Active highlight never lit up. Now we accumulate IPs
        // from Global: AND every `Link N (iface):` line so the
        // full effective resolver list ends up in `display_dns`.
        let mut all_ips: Vec<&str> = Vec::new();
        for line in resolvectl.lines() {
            let trimmed = line.trim();
            let tail = if let Some(t) = trimmed.strip_prefix("Global:") {
                t
            } else if trimmed.starts_with("Link ") {
                // Skip past the `Link N (iface):` prefix.
                trimmed.split_once(':').map(|(_, rest)| rest).unwrap_or("")
            } else {
                continue;
            };
            for tok in tail.split_whitespace() {
                if looks_like_ipv4(tok) {
                    all_ips.push(tok);
                }
            }
        }
        if !all_ips.is_empty() {
            state.display_dns = all_ips.join(" ");
        }
    }

    if state.display_dns.is_empty() {
        if let Ok(raw) = tokio::fs::read_to_string("/etc/resolv.conf").await {
            let parsed: Vec<&str> = raw
                .lines()
                .filter_map(|l| {
                    let t = l.trim();
                    if t.starts_with('#') {
                        return None;
                    }
                    t.strip_prefix("nameserver").map(|s| s.trim())
                })
                .collect();
            state.display_dns = parsed.join(" ");
        }
    }

    if state.display_dns.is_empty() && !state.vpn && !state.blocky && !state.blocked {
        state.error = Some("no DNS probes available".to_string());
    }
    state
}

async fn primary_nm_connection() -> Option<(String, String)> {
    let out =
        run_capture("nmcli", &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"]).await?;
    for line in out.lines() {
        let mut parts = line.splitn(2, ':');
        let name = parts.next()?.to_string();
        let device = parts.next().unwrap_or("");
        if device == "lo" || device.starts_with("wg") {
            continue;
        }
        return Some((name, device.to_string()));
    }
    None
}

fn looks_like_ipv4(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 15 {
        return false;
    }
    let mut octets = 0;
    let mut digits = 0;
    for b in bytes {
        match b {
            b'0'..=b'9' => {
                digits += 1;
                if digits > 3 {
                    return false;
                }
            }
            b'.' => {
                if digits == 0 {
                    return false;
                }
                octets += 1;
                digits = 0;
            }
            _ => return false,
        }
    }
    octets == 3 && digits > 0
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
    fn matches_preset_unordered() {
        let s = DnsState {
            display_dns: "8.8.4.4 8.8.8.8".to_string(),
            ..DnsState::default()
        };
        assert!(s.matches_preset("8.8.8.8 8.8.4.4"));
        assert!(!s.matches_preset("1.1.1.1"));
    }

    /// Mullvad-on case: VPN client prepends `10.64.0.1` to the
    /// global resolver list. Google is still the user's chosen
    /// preset — strict equality used to mark it inactive, which
    /// is the bug the user reported as "Apply doesn't highlight."
    #[test]
    fn matches_preset_subset_under_vpn() {
        let s = DnsState {
            display_dns: "10.64.0.1 8.8.8.8 8.8.4.4".to_string(),
            ..DnsState::default()
        };
        assert!(s.matches_preset("8.8.8.8 8.8.4.4"));
        assert!(s.matches_preset("10.64.0.1")); // single-IP preset hypothetical
        assert!(!s.matches_preset("9.9.9.9 149.112.112.112"));
    }

    #[test]
    fn matches_preset_empty_preset_never_matches() {
        let s = DnsState {
            display_dns: "8.8.8.8".to_string(),
            ..DnsState::default()
        };
        assert!(!s.matches_preset(""));
    }
}
