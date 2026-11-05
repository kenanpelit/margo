//! Safety net for IP / DNS / gateway edits on a connection that is carrying
//! traffic: after the change is applied, a **transient systemd user timer**
//! restores the saved profile and re-activates it unless the user presses
//! *Keep* in time — the Monitors "keep this layout?" countdown, for the
//! network.
//!
//! The rollback is armed in systemd rather than in the shell on purpose: a
//! static address that takes the machine offline is exactly the moment the
//! shell may be restarted, and a timer inside mshell would die with it.
//! `systemd-run --user --on-active=<n>s` runs `nmcli connection modify
//! <uuid> <saved values…>` with an `ExecStartPost=nmcli connection up
//! <uuid>`; *Keep* stops the timer, *Revert now* starts the service at once.
//!
//! Only IP settings travel on these argvs — never a secret. Every value is
//! validated first ([`check_value`]) so nothing systemd would expand (`$`,
//! `%`) or a quote can reach the unit.

/// The IP part of a connection profile, as `nmcli -t -f <field>` reads it
/// (empty = unset).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpSettings {
    pub ipv4_method: String,
    pub ipv4_addresses: String,
    pub ipv4_gateway: String,
    pub ipv4_dns: String,
    pub ipv4_dns_search: String,
    pub ipv4_routes: String,
    pub ipv6_method: String,
    pub ipv6_addresses: String,
    pub ipv6_gateway: String,
    pub ipv6_dns: String,
    pub ipv6_dns_search: String,
    pub ipv6_routes: String,
}

impl IpSettings {
    /// `(nmcli property, value)` for every field, in a fixed order.
    pub fn pairs(&self) -> [(&'static str, &str); 12] {
        [
            ("ipv4.method", &self.ipv4_method),
            ("ipv4.addresses", &self.ipv4_addresses),
            ("ipv4.gateway", &self.ipv4_gateway),
            ("ipv4.dns", &self.ipv4_dns),
            ("ipv4.dns-search", &self.ipv4_dns_search),
            ("ipv4.routes", &self.ipv4_routes),
            ("ipv6.method", &self.ipv6_method),
            ("ipv6.addresses", &self.ipv6_addresses),
            ("ipv6.gateway", &self.ipv6_gateway),
            ("ipv6.dns", &self.ipv6_dns),
            ("ipv6.dns-search", &self.ipv6_dns_search),
            ("ipv6.routes", &self.ipv6_routes),
        ]
    }

    /// What an edit writes over this profile: `(property, value)` pairs from
    /// the editor. `true` when any IP property would change.
    pub fn changed_by(&self, writes: &[(&str, &str)]) -> bool {
        writes.iter().any(|(k, v)| {
            self.pairs()
                .iter()
                .any(|(p, old)| p == k && norm(old) != norm(v))
        })
    }
}

/// nmcli prints list values as `a,b` or `a, b` depending on the field;
/// compare them as token sets in order.
fn norm(v: &str) -> Vec<&str> {
    v.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect()
}

/// An IP value safe to place on an argv systemd stores in a unit: IP
/// addresses, CIDRs, route triples, domain names. Rejects what a unit
/// would expand or mis-split (`$`, `%`, quotes, backslash, `;`, control
/// characters).
pub fn check_value(v: &str) -> Result<(), String> {
    let ok = v.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '/' | ',' | ' ' | '-' | '_' | '~')
    });
    if ok {
        Ok(())
    } else {
        Err(format!("unexpected character in network setting `{v}`"))
    }
}

/// A NetworkManager UUID (hex + dashes), the only identifier these argvs
/// carry.
pub fn check_uuid(uuid: &str) -> Result<(), String> {
    let ok = uuid.len() >= 8 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!("not a connection UUID: `{uuid}`"))
    }
}

/// The transient unit that holds the rollback for `uuid`.
pub fn unit_name(uuid: &str) -> String {
    let short: String = uuid.chars().filter(|c| *c != '-').take(12).collect();
    format!("margo-net-rollback-{short}")
}

/// `nmcli` argv restoring `saved` on `uuid` (every IP property set
/// explicitly, an empty value clearing what the edit added).
pub fn restore_args(uuid: &str, saved: &IpSettings) -> Result<Vec<String>, String> {
    check_uuid(uuid)?;
    let mut args = vec!["connection".to_string(), "modify".into(), uuid.into()];
    for (prop, value) in saved.pairs() {
        // NetworkManager refuses an empty method; an unreadable one stays.
        if prop.ends_with(".method") && value.is_empty() {
            continue;
        }
        check_value(value)?;
        args.push(prop.to_string());
        args.push(value.to_string());
    }
    Ok(args)
}

/// `systemd-run` argv arming the rollback `secs` seconds from now.
pub fn arm_args(uuid: &str, saved: &IpSettings, secs: u32) -> Result<Vec<String>, String> {
    let restore = restore_args(uuid, saved)?;
    let mut args: Vec<String> = [
        "--user",
        "--quiet",
        "--collect",
        &format!("--unit={}", unit_name(uuid)),
        &format!("--on-active={}s", secs.max(1)),
        "--timer-property=AccuracySec=1s",
        "--description=margo: roll back unconfirmed network settings",
        // oneshot: ExecStartPost (the re-activation) runs only after the
        // modify finished and succeeded — with the default `simple` type it
        // would race it.
        "--property=Type=oneshot",
        &format!("--property=ExecStartPost=/usr/bin/env nmcli connection up {uuid}"),
        "--",
        "/usr/bin/env",
        "nmcli",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(restore);
    Ok(args)
}

/// `systemctl` argv cancelling the rollback (*Keep*).
pub fn keep_args(uuid: &str) -> Vec<String> {
    ["--user", "stop", &format!("{}.timer", unit_name(uuid))]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `systemctl` argv running the rollback now (*Revert now* / countdown
/// shown at zero while the shell still runs). Starting the service is the
/// rollback; the timer is stopped after.
pub fn revert_now_args(uuid: &str) -> Vec<String> {
    ["--user", "start", &format!("{}.service", unit_name(uuid))]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ── Async wrappers ─────────────────────────────────────────────────────────

async fn run(bin: &str, args: &[String]) -> Result<(), String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new(bin)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{bin} timed out"))?
    .map_err(|e| format!("{bin}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("{bin} exited with {}", out.status)
        } else {
            err
        })
    }
}

/// Arm the rollback (before the edit is applied, so a failure to arm means
/// the edit is never made).
pub async fn arm(uuid: &str, saved: &IpSettings, secs: u32) -> Result<(), String> {
    if !margo_fs::path::on_path("systemd-run") {
        return Err("systemd-run is not available, so the change can't be guarded".into());
    }
    // A leftover rollback for this connection (from a crash) would collide.
    let _ = run("systemctl", &keep_args(uuid)).await;
    run("systemd-run", &arm_args(uuid, saved, secs)?).await
}

/// Keep the new settings: cancel the pending rollback.
pub async fn keep(uuid: &str) -> Result<(), String> {
    run("systemctl", &keep_args(uuid)).await
}

/// Roll back now.
pub async fn revert_now(uuid: &str) -> Result<(), String> {
    let r = run("systemctl", &revert_now_args(uuid)).await;
    let _ = run("systemctl", &keep_args(uuid)).await;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "7d1c3b0e-5a39-4c2f-9a51-2f1f0e2d9a10";

    /// The editor's read of the owner's static Wi-Fi profile (`nmcli -t -f
    /// <field> connection show <id>` per field, captured on his machine:
    /// unset fields read empty, lists as `a,b`).
    fn captured() -> IpSettings {
        IpSettings {
            ipv4_method: "manual".into(),
            ipv4_addresses: "192.168.0.100/24".into(),
            ipv4_gateway: "192.168.0.1".into(),
            ipv4_dns: "9.9.9.9,149.112.112.112".into(),
            ipv6_method: "disabled".into(),
            ..IpSettings::default()
        }
    }

    fn dhcp() -> IpSettings {
        IpSettings {
            ipv4_method: "auto".into(),
            ipv6_method: "auto".into(),
            ..IpSettings::default()
        }
    }

    #[test]
    fn only_ip_writes_count_as_a_change() {
        let saved = dhcp();
        assert!(!saved.changed_by(&[("ipv4.method", "auto"), ("connection.metered", "yes")]));
        assert!(saved.changed_by(&[("ipv4.method", "manual")]));
        assert!(saved.changed_by(&[("ipv4.dns", "9.9.9.9")]));
        // nmcli's `a,b` vs the editor's `a, b`: same list, no change.
        let real = captured();
        assert!(!real.changed_by(&[("ipv4.dns", "9.9.9.9, 149.112.112.112")]));
        assert!(real.changed_by(&[("ipv4.gateway", "192.168.0.254")]));
        assert!(real.changed_by(&[("ipv4.method", "auto")]));
        let back = restore_args(UUID, &real).unwrap();
        assert!(
            back.windows(2)
                .any(|w| w == ["ipv4.addresses", "192.168.0.100/24"])
        );
        assert!(back.windows(2).any(|w| w == ["ipv6.method", "disabled"]));
    }

    #[test]
    fn restore_sets_every_property_and_clears_additions() {
        let args = restore_args(UUID, &dhcp()).unwrap();
        assert_eq!(&args[..3], ["connection", "modify", UUID]);
        let kv: Vec<(&str, &str)> = args[3..]
            .chunks(2)
            .map(|c| (c[0].as_str(), c[1].as_str()))
            .collect();
        assert!(kv.contains(&("ipv4.method", "auto")));
        // A static address added by the edit is cleared on rollback.
        assert!(kv.contains(&("ipv4.addresses", "")));
        assert!(kv.contains(&("ipv4.gateway", "")));
        assert_eq!(kv.len(), 12);
        // An unreadable method is left alone rather than written empty.
        let unknown = IpSettings::default();
        assert_eq!(restore_args(UUID, &unknown).unwrap().len(), 3 + 10 * 2);
    }

    #[test]
    fn the_timer_argv_is_exact() {
        let saved = IpSettings {
            ipv4_method: "manual".into(),
            ipv4_addresses: "192.168.1.50/24".into(),
            ipv4_gateway: "192.168.1.1".into(),
            ipv4_dns_search: "lan home.arpa".into(),
            ipv6_method: "auto".into(),
            ..IpSettings::default()
        };
        let args = arm_args(UUID, &saved, 30).unwrap();
        let unit = "margo-net-rollback-7d1c3b0e5a39";
        assert_eq!(unit_name(UUID), unit);
        assert_eq!(
            &args[..12],
            [
                "--user",
                "--quiet",
                "--collect",
                &format!("--unit={unit}"),
                "--on-active=30s",
                "--timer-property=AccuracySec=1s",
                "--description=margo: roll back unconfirmed network settings",
                "--property=Type=oneshot",
                &format!("--property=ExecStartPost=/usr/bin/env nmcli connection up {UUID}"),
                "--",
                "/usr/bin/env",
                "nmcli",
            ]
        );
        // A multi-word value stays one argv element.
        assert!(args.iter().any(|a| a == "lan home.arpa"));
        assert_eq!(
            keep_args(UUID),
            ["--user", "stop", &format!("{unit}.timer")]
        );
        assert_eq!(
            revert_now_args(UUID),
            ["--user", "start", &format!("{unit}.service")]
        );
    }

    #[test]
    fn nothing_systemd_would_expand_reaches_the_unit() {
        for bad in ["$HOME", "100%", "a;b", "a\"b", "a'b", "a\\b", "a\nb"] {
            let saved = IpSettings {
                ipv4_dns_search: bad.into(),
                ..dhcp()
            };
            assert!(arm_args(UUID, &saved, 30).is_err(), "{bad:?}");
        }
        assert!(arm_args("x; rm", &dhcp(), 30).is_err());
        assert!(check_value("10.0.0.0/8 192.168.1.1 100, fd00::/64 fe80::1").is_ok());
    }
}
