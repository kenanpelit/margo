//! DNS / VPN switcher bar widget — MVP port of the `ndns` noctalia
//! plugin. Panel + control-center + settings UI are deferred to
//! follow-up work; this is the always-visible bar pill.
//!
//! State is probed every 30 s by spawning a handful of small CLI
//! tools (matches the upstream `scripts/state.sh` set of probes):
//!
//!   * `mullvad status`          → VPN connected / blocked
//!   * `systemctl is-active blocky.service` → Blocky DNS up?
//!   * `nmcli -g IP4.DNS connection show <primary>` → per-link DNS
//!   * `resolvectl dns`          → systemd-resolved global DNS
//!   * `/etc/resolv.conf` fallback when the above all come up empty
//!
//! Icon mapping is "most-secure-wins":
//!
//!   * `shield-safe-symbolic`        — VPN + Blocky both up
//!   * `security-high-symbolic`      — VPN up, Blocky down
//!   * `network-server-symbolic`     — Blocky up, no VPN
//!   * `network-wired-symbolic`      — plain DNS via DHCP / NM
//!   * `security-low-symbolic`       — VPN blocked / device revoked
//!   * `dialog-question-symbolic`    — nothing probed yet (startup)
//!
//! Click runs the user-configured switcher script (defaults to
//! `osc-mullvad`, the convention from upstream); falls back to
//! opening the Mullvad GUI if the script isn't on PATH.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

/// Default switcher subcommand. Upstream nip plugin reads this from
/// `pluginSettings.oscCommand`; once the settings UI is ported this
/// becomes user-configurable, but `osc-mullvad` is the only
/// upstream-shipped command so we hard-wire it for now.
const DEFAULT_SWITCHER: &str = "osc-mullvad";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DnsState {
    vpn: bool,
    blocky: bool,
    blocked: bool,
    /// Pretty-printed list of nameservers (space-separated). May be
    /// empty if all four probes failed.
    display_dns: String,
    /// True when the connection inherits DHCP / NM auto-DNS; used
    /// to disambiguate "DNS is auto-configured" from "no DNS" in
    /// the tooltip.
    auto_dns: bool,
    /// Filled when none of the probes are available at all
    /// (no nmcli, no resolvectl, no /etc/resolv.conf, no mullvad,
    /// no systemctl) — extremely rare, basically a fresh container.
    error: Option<String>,
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
pub(crate) enum NdnsOutput {}

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
                    let state = probe_dns_state().await;
                    let _ = out.send(NdnsCommandOutput::Refreshed(state));
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
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NdnsInput::Clicked => spawn_switcher(),
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
    let icon = if s.blocked {
        "security-low-symbolic"
    } else if s.vpn && s.blocky {
        "shield-safe-symbolic"
    } else if s.vpn {
        "security-high-symbolic"
    } else if s.blocky {
        "network-server-symbolic"
    } else if !s.display_dns.is_empty() {
        "network-wired-symbolic"
    } else {
        "dialog-question-symbolic"
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("DNS: {err}")
    } else {
        let mut lines = Vec::with_capacity(4);
        if s.blocked {
            lines.push("VPN: blocked / revoked".to_string());
        } else if s.vpn {
            lines.push("VPN: connected".to_string());
        } else {
            lines.push("VPN: off".to_string());
        }
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

    // Use the most-secure-wins state for a CSS hook too — the
    // bluetooth widget does the same thing with `.connected`.
    root.remove_css_class("secure");
    root.remove_css_class("blocked");
    if s.blocked {
        root.add_css_class("blocked");
    } else if s.vpn || s.blocky {
        root.add_css_class("secure");
    }
}

fn spawn_switcher() {
    tokio::spawn(async move {
        if let Ok(true) = which_async(DEFAULT_SWITCHER).await {
            let _ = tokio::process::Command::new(DEFAULT_SWITCHER)
                .status()
                .await;
            return;
        }
        // Mullvad GUI as a last-ditch fallback.
        for candidate in ["mullvad-vpn", "mullvad-gui"] {
            if let Ok(true) = which_async(candidate).await {
                let _ = tokio::process::Command::new(candidate).status().await;
                return;
            }
        }
        warn!(
            switcher = DEFAULT_SWITCHER,
            "ndns: switcher script not found on PATH"
        );
    });
}

async fn which_async(bin: &str) -> std::io::Result<bool> {
    let status = tokio::process::Command::new("which")
        .arg(bin)
        .status()
        .await?;
    Ok(status.success())
}

/// Aggregate state probe — matches the structure of upstream
/// `scripts/state.sh`. Each sub-probe is independently fallible;
/// missing tools just leave their bit at the default (false / "").
async fn probe_dns_state() -> DnsState {
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

    if let Some(_) = run_capture("systemctl", &["is-active", "--quiet", "blocky.service"]).await {
        // `is-active --quiet` returns 0 with empty stdout when
        // active — `run_capture` only yields Some on success, so
        // reaching this branch is the signal.
        state.blocky = true;
    }

    // Per-link DNS via NetworkManager. The upstream script walks
    // active connections and picks the first non-loopback / non-
    // wireguard device; replicate that.
    if let Some(primary) = primary_nm_connection().await {
        if let Some(dns) = run_capture("nmcli", &["-g", "IP4.DNS", "connection", "show", &primary])
            .await
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
                &primary,
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

    // systemd-resolved global DNS — usually overrides the above
    // for visible behavior.
    if let Some(resolvectl) = run_capture("resolvectl", &["dns"]).await {
        let global = resolvectl
            .lines()
            .filter_map(|l| l.trim().strip_prefix("Global:"))
            .map(|s| s.trim())
            .collect::<Vec<_>>()
            .join(" ");
        if !global.is_empty() {
            state.display_dns = global;
        }
    }

    // Final fallback: parse /etc/resolv.conf directly.
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

    if state.display_dns.is_empty()
        && !state.vpn
        && !state.blocky
        && !state.blocked
    {
        state.error = Some("no DNS probes available".to_string());
    }
    state
}

async fn primary_nm_connection() -> Option<String> {
    let out = run_capture("nmcli", &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"]).await?;
    for line in out.lines() {
        let mut parts = line.splitn(2, ':');
        let name = parts.next()?.to_string();
        let device = parts.next().unwrap_or("");
        if device == "lo" || device.starts_with("wg") {
            continue;
        }
        return Some(name);
    }
    None
}

fn looks_like_ipv4(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 15 {
        return false;
    }
    let mut octets = 0;
    let mut digits_in_octet = 0;
    for b in bytes {
        match b {
            b'0'..=b'9' => {
                digits_in_octet += 1;
                if digits_in_octet > 3 {
                    return false;
                }
            }
            b'.' => {
                if digits_in_octet == 0 {
                    return false;
                }
                octets += 1;
                digits_in_octet = 0;
            }
            _ => return false,
        }
    }
    octets == 3 && digits_in_octet > 0
}

/// Run `cmd args…`, return Some(stdout) on exit-0, None otherwise.
/// Treats ENOENT (tool not installed) the same as a non-zero exit.
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
    fn ipv4_pattern() {
        assert!(looks_like_ipv4("1.1.1.1"));
        assert!(looks_like_ipv4("192.168.0.1"));
        assert!(!looks_like_ipv4("1.1.1"));
        assert!(!looks_like_ipv4(""));
        assert!(!looks_like_ipv4("not.an.ip.addr"));
        assert!(!looks_like_ipv4("1234.1.1.1"));
    }
}
