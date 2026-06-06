//! Native home-network login automation — replaces the external `home-net-vpn`
//! login script. At login (and on demand from the Network Console menu) bring
//! up a saved Wi-Fi connection, then connect Mullvad, and — coupled — stop
//! Blocky while the VPN is up (Blocky is the no-VPN DNS fallback).
//!
//! All privileged escalation is **non-interactive** (`sudo -n` only, never
//! pkexec) so it can't hang behind a focused-menu grab; the Blocky step is
//! simply skipped when passwordless sudo isn't set up. Runs on the shared
//! [`tokio_rt`](crate::tokio_rt).

use crate::tokio_rt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, LoginNetworkConfig};
use reactive_graph::traits::GetUntracked;
use std::time::Duration;

/// Snapshot the config (untracked — off the reactive graph here).
fn config() -> LoginNetworkConfig {
    config_manager().config().login_network().get_untracked()
}

fn notify(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    tokio_rt().spawn(async move {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                "network-vpn-symbolic",
                &summary,
                &body,
            ])
            .status()
            .await;
    });
}

/// `nmcli connection up <name>` — rootless (NM lets the active session activate
/// a saved connection). No-op if it's already active.
async fn wifi_up(name: &str) -> Result<(), String> {
    let already = tokio::process::Command::new("nmcli")
        .args(["-t", "-g", "NAME", "connection", "show", "--active"])
        .output()
        .await
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l == name)
        })
        .unwrap_or(false);
    if already {
        return Ok(());
    }
    let s = tokio::process::Command::new("nmcli")
        .args(["connection", "up", name])
        .status()
        .await
        .map_err(|e| format!("nmcli spawn: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("nmcli up {name} exit {s}"))
    }
}

/// `mullvad connect` — rootless.
async fn mullvad_connect() -> Result<(), String> {
    let s = tokio::process::Command::new("mullvad")
        .arg("connect")
        .status()
        .await
        .map_err(|e| format!("mullvad spawn: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("mullvad connect exit {s}"))
    }
}

/// Stop Blocky — passwordless `sudo -n` only (never interactive). Errors (incl.
/// no NOPASSWD sudo) are non-fatal: the VPN is already up.
async fn blocky_stop() -> Result<(), String> {
    let nopass = tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !nopass {
        return Err("no passwordless sudo".into());
    }
    let s = tokio::process::Command::new("sudo")
        .args(["-n", "systemctl", "stop", "blocky.service"])
        .status()
        .await
        .map_err(|e| format!("sudo spawn: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("blocky stop exit {s}"))
    }
}

/// Run the full reconcile once. `loud` → desktop notifications.
pub async fn run_reconcile(loud: bool) {
    let cfg = config();

    if !cfg.wifi_connection.is_empty() {
        match wifi_up(&cfg.wifi_connection).await {
            Ok(()) => {
                if loud {
                    notify(
                        "🌐 Home network",
                        &format!("Wi-Fi up: {}", cfg.wifi_connection),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "login-net: wifi");
                if loud {
                    notify("⚠️ Home network", &format!("Wi-Fi failed: {e}"));
                }
                return;
            }
        }
    }

    if cfg.connect_vpn {
        match mullvad_connect().await {
            Ok(()) => {
                if loud {
                    notify("🔒 Mullvad VPN", "Connected");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "login-net: vpn");
                if loud {
                    notify("⚠️ Mullvad VPN", &format!("Connect failed: {e}"));
                }
            }
        }
        if cfg.couple_blocky
            && let Err(e) = blocky_stop().await
        {
            tracing::info!(error = %e, "login-net: blocky stop skipped");
        }
    }
}

/// At login: if enabled, wait the configured delay then reconcile (loud).
pub fn spawn_login_net_startup() {
    let cfg = config();
    if !cfg.enabled {
        return;
    }
    tokio_rt().spawn(async move {
        tokio::time::sleep(Duration::from_secs(cfg.delay_secs as u64)).await;
        run_reconcile(true).await;
    });
}

/// Manual trigger (Network Console menu button / keybind) — ignores `enabled`.
pub fn run_now() {
    tokio_rt().spawn(async move {
        run_reconcile(true).await;
    });
}
