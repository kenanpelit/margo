//! Managed SSH local port forwards (`ssh -L`) for the SSH Sessions menu's
//! **Forwards** tab.
//!
//! A forward profile (`network.ssh_forwards`: name, SSH destination, local
//! port → remote host:port) runs as a **transient systemd user service**,
//! `margo-ssh-forward-<slug>.service`, so it outlives a shell restart,
//! reconnects on its own after a dropped link (`Restart=on-failure`) and
//! logs to the journal (`journalctl --user -u <unit>`). The listener binds
//! `127.0.0.1` only. `BatchMode=yes`: an SSH key / agent is required — the
//! shell never asks for or stores a password, and an unknown host key must
//! be accepted once in a terminal.
//!
//! Every argv here is built from validated fields ([`validate`]), and the
//! destination follows `--`, so a profile can never smuggle in an `ssh`
//! option. Pure builders + parsers are unit-tested against `systemctl show`
//! output captured on the owner's machine.

use mshell_config::schema::config::SshForward;

/// The transient unit prefix.
pub const UNIT_PREFIX: &str = "margo-ssh-forward-";

/// What a forward is doing, from `systemctl --user show`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardState {
    /// Not running (never started, stopped, or collected after a stop).
    Stopped,
    /// ssh is up; the tunnel listens.
    Running,
    /// Starting for the first time.
    Starting,
    /// ssh exited with an error and systemd will retry (`auto-restart`).
    Retrying { restarts: u32 },
    /// Given up (failed and not restarting).
    Failed,
}

impl ForwardState {
    pub fn is_on(self) -> bool {
        !matches!(self, ForwardState::Stopped | ForwardState::Failed)
    }

    pub fn label(self) -> String {
        match self {
            ForwardState::Stopped => "Stopped".to_string(),
            ForwardState::Running => "Connected".to_string(),
            ForwardState::Starting => "Connecting…".to_string(),
            ForwardState::Retrying { restarts: 0 } => "Failed · retrying".to_string(),
            ForwardState::Retrying { restarts } => format!("Failed · retry {restarts}"),
            ForwardState::Failed => "Failed".to_string(),
        }
    }
}

/// Parse `systemctl --user show -p LoadState -p ActiveState -p SubState
/// -p NRestarts <unit>` (`Key=Value` lines, any order).
pub fn parse_state(show: &str) -> ForwardState {
    let get = |key: &str| {
        show.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
            .trim()
    };
    let restarts = get("NRestarts").parse().unwrap_or(0);
    match (get("LoadState"), get("ActiveState"), get("SubState")) {
        ("not-found", _, _) => ForwardState::Stopped,
        (_, "active", _) => ForwardState::Running,
        (_, "activating", "auto-restart") => ForwardState::Retrying { restarts },
        (_, "activating", _) | (_, "reloading", _) => ForwardState::Starting,
        (_, "failed", _) => ForwardState::Failed,
        _ => ForwardState::Stopped,
    }
}

/// The unit-name part of a profile name: lowercase letters, digits and
/// single dashes (`My DB (prod)` → `my-db-prod`). Empty when nothing is
/// left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(48).collect()
}

pub fn unit_name(f: &SshForward) -> String {
    format!("{UNIT_PREFIX}{}", slug(&f.name))
}

/// An SSH alias, host name, IP, `[v6]` or `user@host` — never something
/// `ssh` could read as an option or a second argument.
fn host_ok(host: &str, allow_user: bool) -> bool {
    let host = match (allow_user, host.rsplit_once('@')) {
        (true, Some((user, h))) => {
            let user_ok = !user.is_empty()
                && !user.starts_with('-')
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
            if !user_ok {
                return false;
            }
            h
        }
        _ => host,
    };
    if host.is_empty() || host.len() > 253 || host.starts_with('-') {
        return false;
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }
    if host.contains(':') {
        return host.parse::<std::net::Ipv6Addr>().is_ok();
    }
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Why a profile can't run, worded for the UI; `others` are the rest of
/// the saved profiles (names and local ports must be unique).
pub fn validate(f: &SshForward, others: &[SshForward]) -> Result<(), String> {
    let name = f.name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err("Name: 1–64 printable characters".to_string());
    }
    if slug(name).is_empty() {
        return Err("Name needs at least one letter or digit".to_string());
    }
    if !host_ok(f.host.trim(), true) {
        return Err("SSH host: an alias, host name, IP or user@host".to_string());
    }
    if !host_ok(f.remote_host.trim(), false) {
        return Err("Remote host: a host name or IP as seen from the SSH host".to_string());
    }
    if f.local_port == 0 || f.remote_port == 0 {
        return Err("Ports must be 1–65535".to_string());
    }
    for o in others {
        if slug(&o.name) == slug(name) {
            return Err(format!("A forward named “{}” already exists", o.name));
        }
        if o.local_port == f.local_port {
            return Err(format!(
                "Local port {} is already used by “{}”",
                f.local_port, o.name
            ));
        }
    }
    Ok(())
}

/// The `-L` spec: `127.0.0.1:<local>:<remote host>:<remote port>`.
pub fn forward_spec(f: &SshForward) -> String {
    let host = f.remote_host.trim();
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("127.0.0.1:{}:{host}:{}", f.local_port, f.remote_port)
}

/// `systemd-run` argv starting the forward as a transient user service.
pub fn start_args(f: &SshForward) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--user".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit={}", unit_name(f)),
        format!("--description=margo SSH forward {}", slug(&f.name)),
        "--property=Restart=on-failure".into(),
        "--property=RestartSec=10".into(),
        "--property=StartLimitIntervalSec=0".into(),
        "--".into(),
        "/usr/bin/env".into(),
        "ssh".into(),
        "-N".into(),
        "-T".into(),
    ];
    for opt in [
        // No password / passphrase prompt nobody could answer.
        "BatchMode=yes",
        // A port that can't be bound is an error, not a silent no-op.
        "ExitOnForwardFailure=yes",
        // Notice a dead link and let systemd reconnect.
        "ServerAliveInterval=15",
        "ServerAliveCountMax=3",
        "ConnectTimeout=10",
        // Never hand the forward to (or become) a shared control master.
        "ControlMaster=no",
        "ControlPath=none",
        "PermitLocalCommand=no",
        "RequestTTY=no",
    ] {
        args.push("-o".into());
        args.push(opt.into());
    }
    args.push("-L".into());
    args.push(forward_spec(f));
    args.push("--".into());
    args.push(f.host.trim().to_string());
    args
}

pub fn stop_args(f: &SshForward) -> Vec<String> {
    vec![
        "--user".into(),
        "stop".into(),
        format!("{}.service", unit_name(f)),
    ]
}

pub fn status_args(f: &SshForward) -> Vec<String> {
    vec![
        "--user".into(),
        "show".into(),
        "-p".into(),
        "LoadState".into(),
        "-p".into(),
        "ActiveState".into(),
        "-p".into(),
        "SubState".into(),
        "-p".into(),
        "NRestarts".into(),
        format!("{}.service", unit_name(f)),
    ]
}

/// The terminal command following the forward's journal.
pub fn logs_command(f: &SshForward) -> Vec<String> {
    vec![
        "journalctl".into(),
        "--user".into(),
        "-u".into(),
        format!("{}.service", unit_name(f)),
        "-n".into(),
        "200".into(),
        "-f".into(),
    ]
}

/// The browser link of the local end.
pub fn url(f: &SshForward) -> String {
    let scheme = if f.https { "https" } else { "http" };
    format!("{scheme}://localhost:{}", f.local_port)
}

// ── Async wrappers ─────────────────────────────────────────────────────────

async fn run(bin: &str, args: &[String]) -> Result<String, String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::process::Command::new(bin)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{bin} timed out"))?
    .map_err(|e| format!("{bin}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("{bin} exited with {}", out.status)
        } else {
            err
        })
    }
}

/// Why forwards can't run at all here, if they can't.
pub fn unavailable() -> Option<&'static str> {
    if !margo_fs::path::on_path("ssh") {
        Some("ssh is not installed (package openssh)")
    } else if !margo_fs::path::on_path("systemd-run") || !margo_fs::path::on_path("systemctl") {
        Some("Forwards need systemd user services")
    } else {
        None
    }
}

pub async fn start(f: &SshForward) -> Result<(), String> {
    if let Some(why) = unavailable() {
        return Err(why.to_string());
    }
    // A failed run of the same profile is still loaded; clear it first.
    let _ = run("systemctl", &stop_args(f)).await;
    let _ = run(
        "systemctl",
        &[
            "--user".into(),
            "reset-failed".into(),
            format!("{}.service", unit_name(f)),
        ],
    )
    .await;
    run("systemd-run", &start_args(f)).await.map(|_| ())
}

pub async fn stop(f: &SshForward) -> Result<(), String> {
    run("systemctl", &stop_args(f)).await.map(|_| ())
}

pub async fn state(f: &SshForward) -> ForwardState {
    run("systemctl", &status_args(f))
        .await
        .map(|o| parse_state(&o))
        .unwrap_or(ForwardState::Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fwd(name: &str, host: &str, lp: u16, rh: &str, rp: u16) -> SshForward {
        SshForward {
            name: name.into(),
            host: host.into(),
            local_port: lp,
            remote_host: rh.into(),
            remote_port: rp,
            https: false,
        }
    }

    // `systemctl --user show -p … <unit>` captured on the owner's machine
    // for a running unit, one waiting to restart after a failure, and an
    // unknown one.
    const RUNNING: &str =
        "LoadState=loaded\nActiveState=active\nSubState=running\nNRestarts=0\nExecMainStatus=0\n";
    const AUTO_RESTART: &str = "LoadState=loaded\nActiveState=activating\nSubState=auto-restart\nNRestarts=0\nExecMainStatus=1\n";
    const NOT_FOUND: &str =
        "LoadState=not-found\nActiveState=inactive\nSubState=dead\nNRestarts=0\nExecMainStatus=0\n";

    #[test]
    fn states_from_systemctl_show() {
        assert_eq!(parse_state(RUNNING), ForwardState::Running);
        assert_eq!(
            parse_state(AUTO_RESTART),
            ForwardState::Retrying { restarts: 0 }
        );
        assert_eq!(
            parse_state(&AUTO_RESTART.replace("NRestarts=0", "NRestarts=4")),
            ForwardState::Retrying { restarts: 4 }
        );
        assert_eq!(parse_state(NOT_FOUND), ForwardState::Stopped);
        assert_eq!(
            parse_state("LoadState=loaded\nActiveState=failed\nSubState=failed\n"),
            ForwardState::Failed
        );
        assert_eq!(
            parse_state("LoadState=loaded\nActiveState=activating\nSubState=start\n"),
            ForwardState::Starting
        );
        assert_eq!(parse_state(""), ForwardState::Stopped);
        assert!(ForwardState::Retrying { restarts: 1 }.is_on());
        assert!(!ForwardState::Failed.is_on());
    }

    #[test]
    fn slugs_name_the_unit() {
        assert_eq!(slug("My DB (prod)"), "my-db-prod");
        assert_eq!(slug("  --grafana--  "), "grafana");
        assert_eq!(slug("çğ"), "");
        let f = fwd("Grafana", "box", 3000, "localhost", 3000);
        assert_eq!(unit_name(&f), "margo-ssh-forward-grafana");
    }

    #[test]
    fn validation_keeps_options_out() {
        let ok = fwd("db", "kenan@homelab", 5433, "127.0.0.1", 5432);
        assert!(validate(&ok, &[]).is_ok());
        assert!(validate(&fwd("v6", "[2001:db8::1]", 1, "::1", 2), &[]).is_ok());
        for bad in [
            fwd("x", "-oProxyCommand=evil", 1, "h", 1),
            fwd("x", "host extra", 1, "h", 1),
            fwd("x", "-u@host", 1, "h", 1),
            fwd("x", "h", 1, "-L1:2:3", 1),
            fwd("x", "h", 1, "a b", 1),
            fwd("x", "h", 0, "h", 1),
            fwd("", "h", 1, "h", 1),
            fwd("--", "h", 1, "h", 1),
        ] {
            assert!(validate(&bad, &[]).is_err(), "{bad:?}");
        }
        // Unique names (by unit) and local ports.
        assert!(validate(&fwd("DB", "h", 1, "h", 1), std::slice::from_ref(&ok)).is_err());
        assert!(validate(&fwd("web", "h", 5433, "h", 1), &[ok]).is_err());
    }

    #[test]
    fn the_start_argv_is_exact() {
        let f = fwd("db", "kenan@homelab", 5433, "db.internal", 5432);
        let args = start_args(&f);
        assert_eq!(
            &args[..9],
            [
                "--user",
                "--quiet",
                "--collect",
                "--unit=margo-ssh-forward-db",
                "--description=margo SSH forward db",
                "--property=Restart=on-failure",
                "--property=RestartSec=10",
                "--property=StartLimitIntervalSec=0",
                "--",
            ]
        );
        assert_eq!(&args[9..13], ["/usr/bin/env", "ssh", "-N", "-T"]);
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(args.windows(2).any(|w| w == ["-o", "ControlPath=none"]));
        let n = args.len();
        assert_eq!(
            &args[n - 4..],
            [
                "-L",
                "127.0.0.1:5433:db.internal:5432",
                "--",
                "kenan@homelab"
            ]
        );
        assert_eq!(
            forward_spec(&fwd("v6", "h", 8080, "fd00::5", 80)),
            "127.0.0.1:8080:[fd00::5]:80"
        );
        assert_eq!(
            stop_args(&f),
            ["--user", "stop", "margo-ssh-forward-db.service"]
        );
        assert_eq!(
            logs_command(&f),
            [
                "journalctl",
                "--user",
                "-u",
                "margo-ssh-forward-db.service",
                "-n",
                "200",
                "-f"
            ]
        );
        assert_eq!(url(&f), "http://localhost:5433");
        assert_eq!(
            url(&SshForward { https: true, ..f }),
            "https://localhost:5433"
        );
    }
}
