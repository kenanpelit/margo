//! `mvpn health --json`: what is wrong with Mullvad right now, and the
//! fix for it, as one plain sentence plus the actions that fix it.
//!
//! mvpn gathers the facts ([`Health`]); [`diagnose`] — shared by the
//! `mvpn health` text output and the VPN menu's error header — turns
//! them into at most one [`Problem`], the most urgent one first (a
//! stopped service hides everything behind it).

use serde::{Deserialize, Serialize};

/// The Mullvad service.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Daemon {
    #[default]
    Running,
    /// `mullvad` is installed but cannot reach `mullvad-daemon`.
    Down,
    /// No `mullvad` program at all.
    Missing,
}

/// The account as `mullvad account get` reports it.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Account {
    LoggedIn {
        device: String,
        /// `YYYY-MM-DD`, empty when not reported.
        expiry: String,
        expired: bool,
    },
    LoggedOut,
    /// This device was revoked (from another device or the website).
    Revoked,
    /// Not read (daemon down, offline, an answer this parser does not know).
    #[default]
    Unknown,
}

/// One device on the account (`mvpn account devices --json`).
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub name: String,
    /// The device this machine is logged in as.
    pub current: bool,
}

/// Parse `mvpn account devices --json`; unreadable output is `None`.
pub fn parse_devices(json: &str) -> Option<Vec<Device>> {
    serde_json::from_str(json.trim()).ok()
}

/// Parse `mvpn account show --json`; unreadable output is `Unknown`.
pub fn parse_account(json: &str) -> Account {
    serde_json::from_str(json.trim()).unwrap_or_default()
}

/// The facts [`diagnose`] reads.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Health {
    pub daemon: Daemon,
    /// `systemctl is-enabled mullvad-daemon`: `Some(false)` only for a
    /// unit that is plainly disabled.
    pub boot_enabled: Option<bool>,
    /// Tunnel state (`disconnected`, `connected`, `error`, …).
    pub state: String,
    /// Disconnected with lockdown on: all traffic is blocked.
    pub locked_down: bool,
    /// `error` state cause (`set_firewall_policy_error`, `auth_failed`, …).
    pub error_reason: String,
    pub error_detail: String,
    /// `/usr/lib/modules/$(uname -r)` exists. `Some(false)` after a kernel
    /// upgrade without a reboot: the running kernel's modules are gone,
    /// so nftables cannot load what Mullvad's firewall needs.
    pub kernel_modules_present: Option<bool>,
    pub account: Account,
}

/// One button in the error header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fix {
    StartService,
    EnableService,
    RestartService,
    Disconnect,
    Connect,
    LockdownOff,
    Login,
    /// Open the menu's Account section.
    ShowAccount,
    /// Open the account page on mullvad.net.
    OpenAccountPage,
}

/// The account page `Fix::OpenAccountPage` opens.
pub const ACCOUNT_URL: &str = "https://mullvad.net/account";

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::StartService => "Start service",
            Fix::EnableService => "Start at boot",
            Fix::RestartService => "Restart service",
            Fix::Disconnect => "Disconnect",
            Fix::Connect => "Connect",
            Fix::LockdownOff => "Turn off lockdown",
            Fix::Login => "Log in",
            Fix::ShowAccount => "Account",
            Fix::OpenAccountPage => "Open account page",
        }
    }

    /// The `mvpn` argv that applies the fix; `None` for the two that open
    /// something instead (the Account section, the account page).
    pub fn mvpn_args(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Fix::StartService => &["service", "start"],
            Fix::EnableService => &["service", "enable"],
            Fix::RestartService => &["service", "restart"],
            Fix::Disconnect => &["disconnect"],
            Fix::Connect => &["connect"],
            Fix::LockdownOff => &["lockdown", "off"],
            Fix::Login => &["account", "login"],
            Fix::ShowAccount | Fix::OpenAccountPage => return None,
        })
    }
}

/// How loud the header is (DESIGN.md severity ladder).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Nothing broke; something is missing (Mullvad not installed).
    Calm,
    /// Needs attention, traffic still flows (logged out, service off).
    Warn,
    /// Traffic is blocked or the tunnel failed.
    Danger,
}

/// What is wrong, in one sentence, and what fixes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// Stable id (`daemon_down`, `kernel_updated`, `lockdown_blocking`, …).
    pub id: String,
    pub message: String,
    pub fixes: Vec<Fix>,
    pub severity: Severity,
}

fn problem(id: &str, message: &str, fixes: &[Fix], severity: Severity) -> Option<Problem> {
    Some(Problem {
        id: id.to_string(),
        message: message.to_string(),
        fixes: fixes.to_vec(),
        severity,
    })
}

/// The most urgent problem `h` shows, or `None` when all is well.
pub fn diagnose(h: &Health) -> Option<Problem> {
    use Fix::*;
    use Severity::*;
    match h.daemon {
        Daemon::Missing => {
            return problem(
                "not_installed",
                "Mullvad is not installed (no mullvad command).",
                &[],
                Calm,
            );
        }
        Daemon::Down => {
            let fixes: &[Fix] = if h.boot_enabled == Some(false) {
                &[StartService, EnableService]
            } else {
                &[StartService]
            };
            return problem(
                "daemon_down",
                "The Mullvad service is not running.",
                fixes,
                Warn,
            );
        }
        Daemon::Running => {}
    }
    if h.state == "error" {
        return tunnel_error(h);
    }
    if h.state == "disconnected" && h.locked_down {
        return problem(
            "lockdown_blocking",
            "Lockdown mode is blocking all traffic while you are disconnected.",
            &[Connect, LockdownOff],
            Danger,
        );
    }
    match &h.account {
        Account::LoggedIn { expired: true, .. } => problem(
            "expired",
            "Your Mullvad account has expired. Add time on mullvad.net.",
            &[OpenAccountPage],
            Warn,
        ),
        Account::LoggedOut => problem(
            "logged_out",
            "You are logged out of Mullvad.",
            &[Login],
            Warn,
        ),
        Account::Revoked => problem(
            "revoked",
            "This device was removed from your Mullvad account. Log in again.",
            &[Login],
            Warn,
        ),
        _ => None,
    }
}

/// The tunnel's `error` state, by cause (talpid-types `ErrorStateCause`).
fn tunnel_error(h: &Health) -> Option<Problem> {
    use Fix::*;
    use Severity::Danger;
    match h.error_reason.as_str() {
        "set_firewall_policy_error" if h.kernel_modules_present == Some(false) => problem(
            "kernel_updated",
            "Your kernel was updated. Reboot so Mullvad can set its firewall.",
            &[Disconnect],
            Danger,
        ),
        "set_firewall_policy_error" => problem(
            "firewall",
            "Mullvad could not set its firewall. Restarting the service may help.",
            &[RestartService, Disconnect],
            Danger,
        ),
        "auth_failed" if h.error_detail.contains("EXPIRED") => problem(
            "expired",
            "Your Mullvad account has expired. Add time on mullvad.net.",
            &[OpenAccountPage, Disconnect],
            Danger,
        ),
        "auth_failed" => problem(
            "auth_failed",
            "Mullvad rejected this device. Check your account.",
            &[ShowAccount, Disconnect],
            Danger,
        ),
        "ipv6_unavailable" => problem(
            "ipv6_unavailable",
            "IPv6 is turned off on this system, so the tunnel cannot start.",
            &[Disconnect],
            Danger,
        ),
        "set_dns_error" => problem(
            "set_dns",
            "Mullvad could not set the system DNS server.",
            &[RestartService, Disconnect],
            Danger,
        ),
        "start_tunnel_error" => problem(
            "start_tunnel",
            "Mullvad could not start the tunnel.",
            &[Disconnect],
            Danger,
        ),
        "tunnel_parameter_error" => problem(
            "tunnel_parameter",
            "Mullvad found no server for your settings. Check the location and the multihop entry.",
            &[Disconnect],
            Danger,
        ),
        "is_offline" => problem(
            "offline",
            "This device is offline, so Mullvad is blocking traffic.",
            &[Disconnect],
            Danger,
        ),
        "" => problem(
            "unknown",
            "Mullvad is blocking traffic after an error.",
            &[Disconnect],
            Danger,
        ),
        other => Some(Problem {
            id: "unknown".into(),
            message: format!(
                "Mullvad is blocking traffic after an error ({}).",
                other.replace('_', " ")
            ),
            fixes: vec![Disconnect],
            severity: Danger,
        }),
    }
}

/// Parse `mvpn health --json`; unreadable output is "unknown" (no problem).
pub fn parse(json: &str) -> Option<Health> {
    serde_json::from_str(json.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Health {
        Health {
            state: "connected".into(),
            account: Account::LoggedIn {
                device: "Fast Goat".into(),
                expiry: "2027-02-06".into(),
                expired: false,
            },
            ..Health::default()
        }
    }

    fn id(h: &Health) -> Option<String> {
        diagnose(h).map(|p| p.id)
    }

    #[test]
    fn healthy_is_no_problem() {
        assert_eq!(diagnose(&ok()), None);
        // Unknown account (offline read) is not a problem by itself.
        let h = Health {
            account: Account::Unknown,
            ..ok()
        };
        assert_eq!(diagnose(&h), None);
    }

    #[test]
    fn a_stopped_service_hides_everything_else() {
        let h = Health {
            daemon: Daemon::Down,
            state: "error".into(),
            account: Account::LoggedOut,
            ..ok()
        };
        let p = diagnose(&h).expect("problem");
        assert_eq!(p.id, "daemon_down");
        assert_eq!(p.fixes, vec![Fix::StartService]);
        assert_eq!(p.severity, Severity::Warn);
        // Disabled at boot: also offer to enable it.
        let h = Health {
            boot_enabled: Some(false),
            ..h
        };
        assert_eq!(
            diagnose(&h).expect("problem").fixes,
            vec![Fix::StartService, Fix::EnableService]
        );
        let h = Health {
            daemon: Daemon::Missing,
            ..ok()
        };
        let p = diagnose(&h).expect("problem");
        assert_eq!(
            (p.id.as_str(), p.severity),
            ("not_installed", Severity::Calm)
        );
        assert!(p.fixes.is_empty());
    }

    #[test]
    fn a_firewall_error_after_a_kernel_update_says_reboot() {
        let h = Health {
            state: "error".into(),
            error_reason: "set_firewall_policy_error".into(),
            error_detail: "generic".into(),
            kernel_modules_present: Some(false),
            ..ok()
        };
        let p = diagnose(&h).expect("problem");
        assert_eq!(p.id, "kernel_updated");
        assert!(p.message.contains("Reboot"));
        // Modules present (or unknown): a plain firewall error.
        for present in [Some(true), None] {
            let h = Health {
                kernel_modules_present: present,
                ..h.clone()
            };
            let p = diagnose(&h).expect("problem");
            assert_eq!(p.id, "firewall");
            assert_eq!(p.fixes, vec![Fix::RestartService, Fix::Disconnect]);
        }
    }

    #[test]
    fn each_failure_maps_to_its_fix() {
        let err = |reason: &str, detail: &str| Health {
            state: "error".into(),
            error_reason: reason.into(),
            error_detail: detail.into(),
            ..ok()
        };
        let fixes = |h: &Health| diagnose(h).expect("problem").fixes;
        assert_eq!(
            fixes(&err("auth_failed", "EXPIRED_ACCOUNT")),
            vec![Fix::OpenAccountPage, Fix::Disconnect]
        );
        assert_eq!(
            id(&err("auth_failed", "INVALID_ACCOUNT")).as_deref(),
            Some("auth_failed")
        );
        assert_eq!(id(&err("is_offline", "")).as_deref(), Some("offline"));
        assert_eq!(
            id(&err("ipv6_unavailable", "")).as_deref(),
            Some("ipv6_unavailable")
        );
        assert_eq!(id(&err("set_dns_error", "")).as_deref(), Some("set_dns"));
        assert_eq!(
            id(&err("start_tunnel_error", "")).as_deref(),
            Some("start_tunnel")
        );
        assert_eq!(
            id(&err("tunnel_parameter_error", "no_matching_relay")).as_deref(),
            Some("tunnel_parameter")
        );
        let p = diagnose(&err("foo_bar", "")).expect("problem");
        assert_eq!(
            p.message,
            "Mullvad is blocking traffic after an error (foo bar)."
        );
        assert_eq!(id(&err("", "")).as_deref(), Some("unknown"));

        let blocking = Health {
            state: "disconnected".into(),
            locked_down: true,
            ..ok()
        };
        assert_eq!(fixes(&blocking), vec![Fix::Connect, Fix::LockdownOff]);
        let expired = Health {
            account: Account::LoggedIn {
                device: String::new(),
                expiry: "2020-01-01".into(),
                expired: true,
            },
            ..ok()
        };
        assert_eq!(fixes(&expired), vec![Fix::OpenAccountPage]);
        let out = Health {
            account: Account::LoggedOut,
            ..ok()
        };
        assert_eq!(fixes(&out), vec![Fix::Login]);
        let revoked = Health {
            account: Account::Revoked,
            ..ok()
        };
        assert_eq!(id(&revoked).as_deref(), Some("revoked"));
    }

    #[test]
    fn fixes_run_fixed_mvpn_commands() {
        assert_eq!(
            Fix::StartService.mvpn_args(),
            Some(&["service", "start"][..])
        );
        assert_eq!(Fix::LockdownOff.mvpn_args(), Some(&["lockdown", "off"][..]));
        assert_eq!(Fix::OpenAccountPage.mvpn_args(), None);
        assert_eq!(Fix::ShowAccount.mvpn_args(), None);
        assert_eq!(Fix::EnableService.label(), "Start at boot");
    }

    #[test]
    fn account_and_devices_json() {
        let a = ok().account;
        assert_eq!(parse_account(&serde_json::to_string(&a).expect("json")), a);
        assert_eq!(
            parse_account(r#"{"state":"logged_out"}"#),
            Account::LoggedOut
        );
        assert_eq!(parse_account(""), Account::Unknown);
        let d = vec![
            Device {
                name: "Fast Goat".into(),
                current: true,
            },
            Device {
                name: "Dreamy Hen".into(),
                current: false,
            },
        ];
        assert_eq!(
            parse_devices(&serde_json::to_string(&d).expect("json")),
            Some(d)
        );
        assert_eq!(parse_devices("Error"), None);
    }

    #[test]
    fn json_round_trip() {
        let h = Health {
            boot_enabled: Some(false),
            kernel_modules_present: Some(true),
            ..ok()
        };
        let json = serde_json::to_string(&h).expect("json");
        assert_eq!(parse(&json), Some(h));
        assert_eq!(parse("nope"), None);
        // Fields from a newer / older mvpn default.
        let old = parse(r#"{"daemon":"down"}"#).expect("parses");
        assert_eq!(old.daemon, Daemon::Down);
        assert_eq!(old.account, Account::Unknown);
    }
}
