//! Trusted networks: bring Mullvad up when the machine joins a network
//! that is not trusted, and (optionally) take a tunnel this rule started
//! down again on a trusted one (`trusted_networks` in the config,
//! Settings → VPN). The decision logic is `mvpn_proto::trust`; this is
//! the loop that feeds it.
//!
//! Every few seconds it reads the active network from the in-memory
//! NetworkManager service (no process spawn). Only when a network is about
//! to count as joined — or a tunnel the rule started is being followed —
//! does it ask `mvpn status --json` for the tunnel state, and it acts
//! through `mvpn connect` / `mvpn disconnect` with a toast saying why.
//! Nothing runs while the rule is off.

use crate::{network_service, tokio_rt};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, TrustedNetworksConfig};
use mvpn_proto::trust::{Action, Network, Rule, Watch};
use reactive_graph::traits::GetUntracked;
use std::time::Duration;
use wayle_network::types::connectivity::ConnectionType;

/// How often the active network is sampled.
const SAMPLE_EVERY: Duration = Duration::from_secs(4);
/// Shell start: give NetworkManager and the at-login network reconcile
/// (`login_net`) a head start before the first sample.
const START_DELAY: Duration = Duration::from_secs(20);

fn rule_of(cfg: &TrustedNetworksConfig) -> Rule {
    Rule {
        auto_connect: cfg.auto_connect,
        disconnect_on_trusted: cfg.disconnect_on_trusted,
        trust_wired: cfg.trust_wired,
        trusted: cfg.trusted_ssids.clone(),
    }
}

fn current_rule() -> Rule {
    rule_of(&config_manager().config().trusted_networks().get_untracked())
}

/// The active network as NetworkManager reports it.
pub fn current_network() -> Network {
    let Some(svc) = network_service() else {
        return Network::Offline;
    };
    match svc.primary.get() {
        ConnectionType::Wifi => Network::Wifi(
            svc.wifi
                .get()
                .and_then(|w| w.ssid.get())
                .unwrap_or_default(),
        ),
        ConnectionType::Wired => Network::Wired,
        _ => Network::Offline,
    }
}

/// The SSID of the Wi-Fi network the machine is on, if any (Settings →
/// VPN's "Trust this network").
pub fn current_ssid() -> Option<String> {
    match current_network() {
        Network::Wifi(ssid) if !ssid.trim().is_empty() => Some(ssid),
        _ => None,
    }
}

/// Tunnel connected or connecting, from `mvpn status --json`.
async fn vpn_up() -> bool {
    let out = tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await;
    match out {
        Ok(o) => {
            let st = mvpn_proto::Status::parse(&String::from_utf8_lossy(&o.stdout));
            st.connected || st.connecting
        }
        Err(_) => false,
    }
}

fn toast(title: &str, body: &str) {
    use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
    push_toast(ToastEvent {
        icon: "network-vpn-symbolic".to_string(),
        title: title.to_string(),
        body: Some(body.to_string()),
        severity: ToastSeverity::Calm,
    });
}

async fn apply(action: &Action) {
    let (verb, title, body) = match action {
        Action::Connect { network } => (
            "connect",
            "Connecting VPN",
            format!("“{network}” is not a trusted network."),
        ),
        Action::Disconnect { network } => (
            "disconnect",
            "Disconnecting VPN",
            format!("Back on trusted “{network}”."),
        ),
    };
    toast(title, &body);
    // mvpn's own connect/disconnect notification would repeat the toast.
    let ok = tokio::process::Command::new("mvpn")
        .arg(verb)
        .env("MVPN_NO_NOTIFY", "1")
        .status()
        .await
        .is_ok_and(|s| s.success());
    if !ok {
        tracing::warn!("trusted networks: `mvpn {verb}` failed");
        toast("VPN", &format!("`mvpn {verb}` failed; see the VPN menu."));
    }
}

/// Start the sampling loop (once, at shell start). It idles while the
/// rule is off and exits when there is no NetworkManager.
pub fn spawn_trusted_networks_watch() {
    tokio_rt().spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        if network_service().is_none() {
            return;
        }
        let mut watch = Watch::default();
        let mut last_rule: Option<Rule> = None;
        loop {
            tokio::time::sleep(SAMPLE_EVERY).await;
            let rule = current_rule();
            if last_rule.as_ref() != Some(&rule) {
                // Switched on or edited: judge the current network afresh.
                watch.reset();
                last_rule = Some(rule.clone());
            }
            if !rule.auto_connect {
                continue;
            }
            let net = current_network();
            let state = if watch.needs_vpn_state(&rule, &net) {
                Some(vpn_up().await)
            } else {
                None
            };
            if let Some(action) = watch.sample(&rule, net, state) {
                apply(&action).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_maps_onto_the_rule() {
        let cfg = TrustedNetworksConfig {
            auto_connect: true,
            disconnect_on_trusted: true,
            trust_wired: false,
            trusted_ssids: vec!["Ken_5".into()],
        };
        let r = rule_of(&cfg);
        assert!(r.auto_connect && r.disconnect_on_trusted && !r.trust_wired);
        assert_eq!(r.trusts(&Network::Wifi("Ken_5".into())), Some(true));
        // The default config is inert.
        assert!(!rule_of(&TrustedNetworksConfig::default()).auto_connect);
    }
}
