//! Shared DNS / VPN / Blocky state probe.
//!
//! Previously the bar-half of the noctalia `dns` plugin lived here as a pill;
//! that pill is gone (its menu is now folded into the VPN menu's DNS section),
//! but the unprivileged state probe + `Mode` classification it carried are
//! still used by the DNS menu widget, so they stay as a standalone module.
//!
//! Probes are unprivileged: `mullvad status`, `systemctl is-active
//! blocky.service`, `nmcli -g IP4.DNS`, `resolvectl dns`.

/// "Mode" classification used by the DNS menu widget's status / action
/// highlight (and formerly the bar pill icon).
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

    /// True when every IP in `preset_ips` is present in `display_dns`.
    /// Subset-match (not strict equality) because auxiliary resolvers can be
    /// auto-prepended (Mullvad's tunnel resolver `10.64.0.1`,
    /// systemd-resolved fallbacks) without invalidating which *preset* the
    /// user actively chose. The preset IP lists are mutually disjoint, so at
    /// most one preset ever reads as Active.
    pub(crate) fn matches_preset(&self, preset_ips: &str) -> bool {
        let current: std::collections::HashSet<&str> =
            self.display_dns.split_whitespace().collect();
        let want: Vec<&str> = preset_ips.split_whitespace().collect();
        !want.is_empty() && want.iter().all(|ip| current.contains(ip))
    }
}

/// Aggregate state probe. Each sub-probe is independently fallible — missing
/// tools leave their bits at default (false / empty).
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
            &["-g", "ipv4.ignore-auto-dns", "connection", "show", &name],
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
        // `resolvectl dns` lists a `Global:` resolver plus per-link
        // `Link N (iface):` lines. Accumulate IPs from all of them so the
        // user's actually-chosen preset (on a per-link line under VPN) is
        // reflected in `display_dns` and `matches_preset` lights up.
        let mut all_ips: Vec<&str> = Vec::new();
        for line in resolvectl.lines() {
            let trimmed = line.trim();
            let tail = if let Some(t) = trimmed.strip_prefix("Global:") {
                t
            } else if trimmed.starts_with("Link ") {
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

    if state.display_dns.is_empty()
        && let Ok(raw) = tokio::fs::read_to_string("/etc/resolv.conf").await
    {
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

    if state.display_dns.is_empty() && !state.vpn && !state.blocky && !state.blocked {
        state.error = Some("no DNS probes available".to_string());
    }
    state
}

async fn primary_nm_connection() -> Option<(String, String)> {
    let out = run_capture(
        "nmcli",
        &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"],
    )
    .await?;
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

    #[test]
    fn matches_preset_subset_under_vpn() {
        let s = DnsState {
            display_dns: "10.64.0.1 8.8.8.8 8.8.4.4".to_string(),
            ..DnsState::default()
        };
        assert!(s.matches_preset("8.8.8.8 8.8.4.4"));
        assert!(s.matches_preset("10.64.0.1"));
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
