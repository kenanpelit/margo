//! Trusted networks: connect the VPN when the machine joins a Wi-Fi
//! network that is not on the user's trusted list, and (optionally) drop
//! a tunnel this rule started once it is back on a trusted one.
//!
//! The shell feeds [`Watch`] the active network every few seconds; the
//! decision logic lives here, pure, so it is tested without a network.

/// The network the machine is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    /// No connection.
    Offline,
    /// A wired connection.
    Wired,
    /// Wi-Fi, by SSID (empty when the SSID is hidden / not known yet).
    Wifi(String),
}

/// The user's rule (`trusted_networks` in the shell config).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rule {
    /// Connect on joining an untrusted network.
    pub auto_connect: bool,
    /// Disconnect a tunnel this rule started when back on a trusted network.
    pub disconnect_on_trusted: bool,
    /// Any wired connection counts as trusted.
    pub trust_wired: bool,
    /// Trusted Wi-Fi SSIDs (exact, case-sensitive: SSIDs are byte strings).
    pub trusted: Vec<String>,
}

impl Rule {
    /// `Some(true)` trusted, `Some(false)` untrusted, `None` offline or a
    /// Wi-Fi whose SSID is not known yet (nothing to decide on).
    pub fn trusts(&self, net: &Network) -> Option<bool> {
        match net {
            Network::Offline => None,
            Network::Wired => Some(self.trust_wired),
            Network::Wifi(ssid) if ssid.trim().is_empty() => None,
            Network::Wifi(ssid) => Some(self.trusted.iter().any(|t| t.trim() == ssid.trim())),
        }
    }
}

/// What to do about the tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Connect: `ssid` (or "wired") is not trusted.
    Connect { network: String },
    /// Disconnect the tunnel this rule started: back on a trusted network.
    Disconnect { network: String },
}

/// How many consecutive samples a network must be seen in before it
/// counts as joined (debounce for roaming / reconnect blips).
pub const SETTLE_SAMPLES: u32 = 2;

/// The watcher's memory between samples.
#[derive(Debug, Default)]
pub struct Watch {
    /// The network being settled and how many samples in a row saw it.
    pending: Option<(Network, u32)>,
    /// The last settled network the rule acted (or chose not to act) on.
    current: Option<Network>,
    /// The tunnel is up because this rule connected it.
    started_by_rule: bool,
}

fn label(net: &Network) -> String {
    match net {
        Network::Wifi(ssid) => ssid.clone(),
        Network::Wired => "wired".into(),
        Network::Offline => String::new(),
    }
}

impl Watch {
    /// Forget the settled network, so the next one is acted on as if just
    /// joined (the rule was changed or switched on).
    pub fn reset(&mut self) {
        self.pending = None;
        self.current = None;
    }

    /// Whether the tunnel is up because of this rule.
    pub fn started_by_rule(&self) -> bool {
        self.started_by_rule
    }

    fn seen(&self, net: &Network) -> u32 {
        match &self.pending {
            Some((n, count)) if n == net => count + 1,
            _ => 1,
        }
    }

    /// Whether [`Watch::sample`] needs the tunnel state for `net`: a
    /// network is about to count as joined, or a tunnel this rule started
    /// is being followed. Otherwise the caller can skip asking the daemon.
    pub fn needs_vpn_state(&self, rule: &Rule, net: &Network) -> bool {
        self.started_by_rule
            || (rule.trusts(net).is_some()
                && self.seen(net) >= SETTLE_SAMPLES
                && self.current.as_ref() != Some(net))
    }

    /// One sample: the network now and, when [`Watch::needs_vpn_state`]
    /// asked for it, whether the tunnel is up (connected or connecting).
    /// Returns the action to take, at most once per joined network. Going
    /// offline is not a change: rejoining the same network after a drop
    /// does nothing, so a tunnel the user turned off stays off.
    pub fn sample(&mut self, rule: &Rule, net: Network, vpn_up: Option<bool>) -> Option<Action> {
        // A tunnel that went down (the user, or the daemon) is no longer ours.
        if vpn_up == Some(false) {
            self.started_by_rule = false;
        }
        let vpn_up = vpn_up.unwrap_or(false);
        if net == Network::Offline || rule.trusts(&net).is_none() {
            self.pending = None;
            return None;
        }
        let seen = self.seen(&net);
        self.pending = Some((net.clone(), seen));
        if seen < SETTLE_SAMPLES || self.current.as_ref() == Some(&net) {
            return None;
        }
        self.current = Some(net.clone());
        if !rule.auto_connect {
            return None;
        }
        match rule.trusts(&net) {
            Some(false) if !vpn_up => {
                self.started_by_rule = true;
                Some(Action::Connect {
                    network: label(&net),
                })
            }
            Some(true) if rule.disconnect_on_trusted && self.started_by_rule && vpn_up => {
                self.started_by_rule = false;
                Some(Action::Disconnect {
                    network: label(&net),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Rule {
        Rule {
            auto_connect: true,
            disconnect_on_trusted: true,
            trust_wired: true,
            trusted: vec!["Ken_5".into(), "Office".into()],
        }
    }

    fn wifi(s: &str) -> Network {
        Network::Wifi(s.into())
    }

    /// Feed `n` identical samples; the actions they produced.
    fn feed(w: &mut Watch, r: &Rule, net: Network, up: bool, n: u32) -> Vec<Action> {
        (0..n)
            .filter_map(|_| {
                let state = w.needs_vpn_state(r, &net).then_some(up);
                w.sample(r, net.clone(), state)
            })
            .collect()
    }

    #[test]
    fn trust_verdicts() {
        let r = rule();
        assert_eq!(r.trusts(&wifi("Ken_5")), Some(true));
        assert_eq!(r.trusts(&wifi(" Ken_5 ")), Some(true));
        assert_eq!(
            r.trusts(&wifi("ken_5")),
            Some(false),
            "SSIDs are case-sensitive"
        );
        assert_eq!(r.trusts(&wifi("Cafe")), Some(false));
        assert_eq!(r.trusts(&wifi("")), None);
        assert_eq!(r.trusts(&Network::Wired), Some(true));
        assert_eq!(r.trusts(&Network::Offline), None);
        let r = Rule {
            trust_wired: false,
            ..rule()
        };
        assert_eq!(r.trusts(&Network::Wired), Some(false));
    }

    #[test]
    fn joining_an_untrusted_wifi_connects_once_it_settles() {
        let r = rule();
        let mut w = Watch::default();
        assert!(
            !w.needs_vpn_state(&r, &wifi("Cafe")),
            "no daemon call for a blip"
        );
        assert_eq!(
            w.sample(&r, wifi("Cafe"), None),
            None,
            "one sample is a blip"
        );
        assert!(w.needs_vpn_state(&r, &wifi("Cafe")), "about to settle");
        assert_eq!(
            w.sample(&r, wifi("Cafe"), Some(false)),
            Some(Action::Connect {
                network: "Cafe".into()
            })
        );
        assert!(w.started_by_rule());
        // Staying there: nothing more, even while the tunnel comes up; the
        // tunnel is followed (it is ours) until it goes down.
        assert!(feed(&mut w, &r, wifi("Cafe"), true, 5).is_empty());
        assert!(w.needs_vpn_state(&r, &wifi("Cafe")));
        feed(&mut w, &r, wifi("Cafe"), false, 1);
        assert!(!w.started_by_rule());
        assert!(
            !w.needs_vpn_state(&r, &wifi("Cafe")),
            "settled, nothing to follow"
        );
    }

    #[test]
    fn back_on_a_trusted_network_drops_only_the_tunnel_it_started() {
        let r = rule();
        let mut w = Watch::default();
        feed(&mut w, &r, wifi("Cafe"), false, 2);
        assert_eq!(
            feed(&mut w, &r, wifi("Ken_5"), true, 2),
            vec![Action::Disconnect {
                network: "Ken_5".into()
            }]
        );
        // A tunnel the user started by hand is left alone.
        let mut w = Watch::default();
        assert!(
            feed(&mut w, &r, wifi("Cafe"), true, 2).is_empty(),
            "already up"
        );
        assert!(!w.started_by_rule());
        assert!(feed(&mut w, &r, wifi("Ken_5"), true, 2).is_empty());
        // Without disconnect_on_trusted, never.
        let keep = Rule {
            disconnect_on_trusted: false,
            ..rule()
        };
        let mut w = Watch::default();
        feed(&mut w, &keep, wifi("Cafe"), false, 2);
        assert!(feed(&mut w, &keep, wifi("Ken_5"), true, 2).is_empty());
    }

    #[test]
    fn a_tunnel_the_user_turned_off_stays_off() {
        let r = rule();
        let mut w = Watch::default();
        feed(&mut w, &r, wifi("Cafe"), false, 2);
        // The user disconnects on the same network: no reconnect, and a
        // Wi-Fi drop + rejoin of the same network is not a new join.
        assert!(feed(&mut w, &r, wifi("Cafe"), false, 3).is_empty());
        assert!(!w.started_by_rule());
        assert!(feed(&mut w, &r, Network::Offline, false, 3).is_empty());
        assert!(feed(&mut w, &r, wifi("Cafe"), false, 3).is_empty());
        // Back on a trusted network: the tunnel isn't ours, nothing to drop.
        assert!(feed(&mut w, &r, wifi("Ken_5"), false, 2).is_empty());
        // A different untrusted network is a new join.
        assert_eq!(feed(&mut w, &r, wifi("Airport"), false, 2).len(), 1);
    }

    #[test]
    fn wired_follows_trust_wired_and_a_switched_off_rule_does_nothing() {
        let mut w = Watch::default();
        assert!(feed(&mut w, &rule(), Network::Wired, false, 3).is_empty());
        let untrusted_wire = Rule {
            trust_wired: false,
            ..rule()
        };
        let mut w = Watch::default();
        assert_eq!(
            feed(&mut w, &untrusted_wire, Network::Wired, false, 2),
            vec![Action::Connect {
                network: "wired".into()
            }]
        );
        let off = Rule {
            auto_connect: false,
            ..rule()
        };
        let mut w = Watch::default();
        assert!(feed(&mut w, &off, wifi("Cafe"), false, 4).is_empty());
        // Switched on while there: `reset` makes the current network count.
        w.reset();
        assert_eq!(feed(&mut w, &rule(), wifi("Cafe"), false, 2).len(), 1);
    }

    #[test]
    fn hidden_ssids_and_flapping_decide_nothing() {
        let r = rule();
        let mut w = Watch::default();
        assert!(feed(&mut w, &r, wifi(""), false, 4).is_empty());
        // Alternating samples never settle.
        for _ in 0..4 {
            assert_eq!(w.sample(&r, wifi("A"), Some(false)), None);
            assert_eq!(w.sample(&r, wifi("B"), Some(false)), None);
        }
    }
}
