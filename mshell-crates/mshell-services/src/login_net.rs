//! Native home-network login automation — the in-shell port of the external
//! `home-net-vpn` login script (retire the script + its systemd timer once
//! `login_network.enabled` is on). At login bring up a saved Wi-Fi
//! connection, then reconcile Mullvad with the Blocky DNS fallback:
//!
//! * Mullvad healthy (tunnel up **and** the Mullvad connectivity endpoint
//!   confirms) → keep/stop Blocky, VPN owns DNS.
//! * Mullvad can't connect, is unhealthy, has no account, or is in a
//!   blocked/revoked state → soft-disable it (disconnect + auto-connect off +
//!   lockdown off, so routing is never left blackholed) and start Blocky as
//!   the local ad-blocking resolver.
//!
//! All privileged escalation is **non-interactive** (`sudo -n` only, never
//! pkexec) so it can't hang behind a focused-menu grab; every Blocky step is
//! simply skipped when passwordless sudo isn't set up. Runs on the shared
//! [`tokio_rt`](crate::tokio_rt).

use crate::tokio_rt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, LoginNetworkConfig};
use reactive_graph::traits::GetUntracked;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long to wait out a Connecting/Disconnecting transition before acting.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(12);
const SETTLE_POLL: Duration = Duration::from_secs(1);
/// How long a Connected tunnel gets to pass the connectivity probe before the
/// fail-safe engages. Right after login the first probes routinely fail —
/// in-tunnel DNS and the first TLS handshake lag the tunnel by seconds — so a
/// single-shot check would soft-disable a perfectly good VPN (observed on
/// hardware 2026-07-14). Mirrors the old script's ensure grace loop.
const HEALTH_GRACE: Duration = Duration::from_secs(20);
const HEALTH_POLL: Duration = Duration::from_secs(2);

/// One reconcile at a time — a shell-start run and a manual trigger must not
/// interleave their Mullvad/Blocky steps.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Snapshot the config (untracked — off the reactive graph here).
fn config() -> LoginNetworkConfig {
    config_manager().config().login_network().get_untracked()
}

/// Ephemeral shell toast — same surface the old script used (`mshellctl
/// toast`), so the login summary looks identical. Severity: calm / positive /
/// danger.
fn toast(title: &str, body: &str, severity: &str, icon: &str) {
    let args: Vec<String> = vec![
        "toast".into(),
        title.into(),
        body.into(),
        "--icon".into(),
        format!("{icon}-symbolic"),
        "--severity".into(),
        severity.into(),
    ];
    tokio_rt().spawn(async move {
        let _ = tokio::process::Command::new("mshellctl")
            .args(&args)
            .status()
            .await;
    });
}

async fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let s = tokio::process::Command::new(cmd)
        .args(args)
        .status()
        .await
        .map_err(|e| format!("{cmd} spawn: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {} exit {s}", args.join(" ")))
    }
}

async fn stdout_of(cmd: &str, args: &[&str]) -> String {
    tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// `nmcli connection up <name>` — rootless (NM lets the active session activate
/// a saved connection). No-op if it's already active.
async fn wifi_up(name: &str) -> Result<bool, String> {
    let already = stdout_of(
        "nmcli",
        &["-t", "-g", "NAME", "connection", "show", "--active"],
    )
    .await
    .lines()
    .any(|l| l == name);
    if already {
        return Ok(false);
    }
    run("nmcli", &["connection", "up", name])
        .await
        .map(|()| true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VpnState {
    Connected,
    Transitioning,
    Disconnected,
    /// `Blocked:` / revoked-device — connecting is pointless until the user
    /// intervenes; fall back rather than retry.
    Blocked,
}

async fn vpn_state() -> VpnState {
    let status = stdout_of("mullvad", &["status"]).await;
    let lower = status.to_lowercase();
    if lower.contains("blocked:") || lower.contains("device has been revoked") {
        VpnState::Blocked
    } else if status.contains("Connected") {
        VpnState::Connected
    } else if status.contains("Connecting") || status.contains("Disconnecting") {
        VpnState::Transitioning
    } else {
        VpnState::Disconnected
    }
}

/// Poll until the daemon leaves Connecting/Disconnecting (or `timeout`).
async fn settle_vpn(timeout: Duration) -> VpnState {
    let mut waited = Duration::ZERO;
    let mut state = vpn_state().await;
    while state == VpnState::Transitioning && waited < timeout {
        tokio::time::sleep(SETTLE_POLL).await;
        waited += SETTLE_POLL;
        state = vpn_state().await;
    }
    state
}

/// True when the tunnel actually routes: Mullvad's own connectivity endpoint
/// answers "You are connected". A `mullvad connect` exit 0 only means the
/// daemon accepted the request — not that the tunnel came up.
async fn internet_ok() -> bool {
    tokio::task::spawn_blocking(|| {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .build();
        match agent.get("https://am.i.mullvad.net/connected").call() {
            Ok(resp) => resp
                .into_string()
                .map(|body| body.contains("You are connected"))
                .unwrap_or(false),
            Err(_) => false,
        }
    })
    .await
    .unwrap_or(false)
}

/// Poll until the tunnel is Connected AND the connectivity probe passes, for
/// at most `grace`. Re-reads the daemon state every round, so it also rides
/// out a Connecting→Connected transition started just before the call.
async fn wait_healthy(grace: Duration) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if vpn_state().await == VpnState::Connected && internet_ok().await {
            return true;
        }
        if waited >= grace {
            tracing::warn!(
                state = ?vpn_state().await,
                "login-net: vpn not healthy after grace window"
            );
            return false;
        }
        tokio::time::sleep(HEALTH_POLL).await;
        waited += HEALTH_POLL;
    }
}

/// Leave Mullvad in a state that cannot blackhole routing while Blocky takes
/// over DNS: disconnect and switch off auto-connect + lockdown.
async fn vpn_soft_disable() {
    let _ = run("mullvad", &["disconnect"]).await;
    let _ = run("mullvad", &["auto-connect", "set", "off"]).await;
    let _ = run("mullvad", &["lockdown-mode", "set", "off"]).await;
}

async fn have_nopass_sudo() -> bool {
    tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stop Blocky (idempotent) and drop any stale resolvconf key it left behind.
/// Errors (incl. no NOPASSWD sudo) are non-fatal: the VPN is already up.
async fn blocky_stop() -> Result<(), String> {
    if !have_nopass_sudo().await {
        return Err("no passwordless sudo".into());
    }
    run("sudo", &["-n", "systemctl", "stop", "blocky.service"]).await?;
    let _ = run(
        "sudo",
        &[
            "-n",
            "bash",
            "-c",
            "command -v resolvconf >/dev/null && { resolvconf -f -d blocky || true; resolvconf -u; } || true",
        ],
    )
    .await;
    Ok(())
}

/// Start Blocky as the local resolver — the no-VPN DNS fallback.
async fn blocky_start() -> Result<(), String> {
    if !have_nopass_sudo().await {
        return Err("no passwordless sudo".into());
    }
    run("sudo", &["-n", "systemctl", "start", "blocky.service"]).await?;
    // Blocky listens on loopback; point the system resolver at it.
    run(
        "sudo",
        &[
            "-n",
            "bash",
            "-c",
            "rm -f /etc/resolv.conf; printf 'nameserver 127.0.0.1\\nnameserver ::1\\n' > /etc/resolv.conf",
        ],
    )
    .await
}

/// The VPN could not be made healthy — apply the fail-safe: routing open,
/// Blocky (if coupled) serving DNS. Returns the summary line for the toast.
async fn apply_fallback(couple_blocky: bool, reason: &str) -> String {
    vpn_soft_disable().await;
    if couple_blocky {
        match blocky_start().await {
            Ok(()) => format!("{reason}; Blocky fallback active"),
            Err(e) => {
                tracing::warn!(error = %e, "login-net: blocky fallback failed");
                format!("{reason}; Blocky fallback FAILED ({e})")
            }
        }
    } else {
        format!("{reason}; VPN left disconnected")
    }
}

/// The VPN is healthy — keep Blocky out of the resolver path.
async fn settle_blocky_off(couple_blocky: bool) {
    if couple_blocky && let Err(e) = blocky_stop().await {
        tracing::info!(error = %e, "login-net: blocky stop skipped");
    }
}

/// Reconcile Mullvad + Blocky. Returns `(healthy, summary)` for the toast.
async fn reconcile_vpn(couple_blocky: bool) -> (bool, String) {
    match settle_vpn(SETTLE_TIMEOUT).await {
        VpnState::Connected => {
            if wait_healthy(HEALTH_GRACE).await {
                settle_blocky_off(couple_blocky).await;
                (true, "already connected".into())
            } else {
                (
                    false,
                    apply_fallback(couple_blocky, "connected but unhealthy").await,
                )
            }
        }
        VpnState::Blocked => (
            false,
            apply_fallback(couple_blocky, "device blocked/revoked").await,
        ),
        VpnState::Disconnected => {
            // No account/session → connecting is pointless.
            if run("mullvad", &["account", "get"]).await.is_err() {
                return (
                    false,
                    apply_fallback(couple_blocky, "no Mullvad account/session").await,
                );
            }
            // OFF → ON: Blocky must release the resolver before the tunnel
            // owns DNS; if it can't be stopped, connecting would break DNS.
            if couple_blocky && let Err(e) = blocky_stop().await {
                tracing::info!(error = %e, "login-net: blocky stop before connect skipped");
            }
            if let Err(e) = run("mullvad", &["connect"]).await {
                tracing::warn!(error = %e, "login-net: mullvad connect");
                return (false, apply_fallback(couple_blocky, "connect failed").await);
            }
            if wait_healthy(HEALTH_GRACE).await {
                settle_blocky_off(couple_blocky).await;
                (true, "connected".into())
            } else {
                (
                    false,
                    apply_fallback(couple_blocky, "tunnel did not become healthy").await,
                )
            }
        }
        VpnState::Transitioning => {
            // Still in transition after the settle window: give it the grace
            // period to become healthy, then enforce the fail-safe.
            if wait_healthy(HEALTH_GRACE).await {
                settle_blocky_off(couple_blocky).await;
                (true, "connected after transition".into())
            } else {
                (
                    false,
                    apply_fallback(couple_blocky, "stuck in transition").await,
                )
            }
        }
    }
}

/// Run the full reconcile once. `loud` → shell toast with the summary.
pub async fn run_reconcile(loud: bool) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        tracing::info!("login-net: reconcile already running, skipping");
        return;
    }

    let cfg = config();
    let mut wifi_summary = "skipped".to_string();

    if !cfg.wifi_connection.is_empty() {
        match wifi_up(&cfg.wifi_connection).await {
            Ok(activated) => {
                wifi_summary = if activated {
                    format!("{} connected", cfg.wifi_connection)
                } else {
                    format!("{} already active", cfg.wifi_connection)
                };
            }
            Err(e) => {
                tracing::warn!(error = %e, "login-net: wifi");
                if loud {
                    toast(
                        "Home network",
                        &format!("Wi-Fi failed: {e}"),
                        "danger",
                        "network-wireless",
                    );
                }
                RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        }
    }

    if cfg.connect_vpn {
        let (healthy, vpn_summary) = reconcile_vpn(cfg.couple_blocky).await;
        tracing::info!(wifi = %wifi_summary, vpn = %vpn_summary, "login-net: reconcile done");
        if loud {
            let (severity, icon) = if healthy {
                ("positive", "network-vpn")
            } else {
                ("danger", "network-server")
            };
            toast(
                "Home network",
                &format!("Wi-Fi: {wifi_summary}\nMullvad: {vpn_summary}"),
                severity,
                icon,
            );
        }
    } else if loud {
        toast(
            "Home network",
            &format!("Wi-Fi: {wifi_summary}"),
            "positive",
            "network-wireless",
        );
    }

    RUNNING.store(false, Ordering::SeqCst);
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
