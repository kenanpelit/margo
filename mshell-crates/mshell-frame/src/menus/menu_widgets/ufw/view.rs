//! Pure pieces of the UFW menu: the header status line, the banner
//! (what's wrong and the one-click fixes), the policy line and each
//! rule row's texts. The component only lays them out.

use mshell_privileged::ufw::rules::{Rule, policy_word};
use mshell_privileged::ufw::state::{Fix, Health, Level, ReadError, Snapshot, Status};

/// A banner button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BannerAction {
    Fix(Fix),
    /// Read the rule files as root (`ufw-status`).
    LoadRules,
}

impl BannerAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            BannerAction::Fix(f) => f.label(),
            BannerAction::LoadRules => "Load rules",
        }
    }
}

/// The banner under the header, `None` when all is well (green, or a
/// plain off: the switch is the fix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Banner {
    /// `warn` (amber) or `muted` (informational) — a CSS class.
    pub tone: &'static str,
    pub text: String,
    pub actions: Vec<BannerAction>,
}

pub(super) fn banner(snap: &Snapshot, health: &Health) -> Option<Banner> {
    if *snap == Snapshot::loading() {
        return None;
    }
    let denied = snap.installed
        && (snap.rules == Err(ReadError::Denied) || snap.conf == Err(ReadError::Denied));
    let mut actions: Vec<BannerAction> = health
        .fixes
        .iter()
        .copied()
        .map(BannerAction::Fix)
        .collect();
    let mut text = health.detail.clone();
    if denied {
        actions.push(BannerAction::LoadRules);
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str("The rule files can't be read without root: Load rules asks once.");
    }
    let tone = match health.level {
        Level::Amber => "warn",
        _ => "muted",
    };
    let show = match health.level {
        Level::Green | Level::Red => denied,
        Level::Amber | Level::Muted => true,
    };
    (show && !text.is_empty()).then_some(Banner {
        tone,
        text,
        actions,
    })
}

/// The header's status line: `On · 9 rules`, `Off — no rule is enforced`.
pub(super) fn status_line(snap: &Snapshot, health: &Health) -> String {
    let rules = match &snap.rules {
        Ok(set) => match set.rules.len() {
            1 => " · 1 rule".to_string(),
            n => format!(" · {n} rules"),
        },
        Err(_) => String::new(),
    };
    match health.status {
        Status::Off => format!("Off — no rule is enforced{rules}"),
        Status::Missing => "ufw isn't installed".to_string(),
        _ if *snap == Snapshot::loading() => "Reading…".to_string(),
        _ => format!("{}{rules}", health.headline),
    }
}

/// `Incoming deny · Outgoing allow · Routed deny · Logging low`.
pub(super) fn policy_line(snap: &Snapshot) -> String {
    let mut parts = Vec::new();
    if let Some(d) = &snap.defaults {
        for (what, p) in [
            ("Incoming", &d.input),
            ("Outgoing", &d.output),
            ("Routed", &d.forward),
        ] {
            if let Some(p) = p {
                parts.push(format!("{what} {}", policy_word(p)));
            }
        }
    }
    if let Ok(c) = &snap.conf
        && let Some(l) = &c.loglevel
    {
        parts.push(format!("Logging {l}"));
    }
    if parts.is_empty() {
        "—".to_string()
    } else {
        parts.join(" · ")
    }
}

/// A rule row's title and subtitle (`22/tcp` / `Allow in · SSH`);
/// `ipv6`: ufw manages IPv6 too (`RuleSet::ipv6`).
pub(super) fn rule_texts(r: &Rule, ipv6: bool) -> (String, String) {
    let mut sub = r.summary(ipv6);
    if let Some(c) = &r.comment {
        sub = format!("{sub} · {c}");
    }
    (r.target(), sub)
}

/// The confirm question for a delete.
pub(super) fn delete_question(r: &Rule, ipv6: bool) -> String {
    let (title, sub) = rule_texts(r, ipv6);
    format!("Remove {title} ({sub})?")
}

/// Shown on a rule the menu can't delete.
pub(super) const NOT_HERE: &str = "This kind of rule (route, app profile or source port) is removed with `sudo ufw delete` in a terminal";

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_privileged::ufw::rules::{Conf, parse_defaults, parse_rules};
    use mshell_privileged::ufw::state::{UnitRead, classify, parse_show};

    fn snap(enabled: bool, show: &str) -> Snapshot {
        Snapshot {
            installed: true,
            conf: Ok(Conf {
                enabled: Some(enabled),
                loglevel: Some("low".into()),
            }),
            defaults: Some(parse_defaults(
                "DEFAULT_INPUT_POLICY=\"DROP\"\nDEFAULT_OUTPUT_POLICY=\"ACCEPT\"\nDEFAULT_FORWARD_POLICY=\"DROP\"\n",
            )),
            unit: UnitRead::Unit(parse_show(show)),
            rules: Ok(parse_rules(
                "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=535348\n### END RULES ###\n",
                None,
                false,
            )),
        }
    }

    const ON: &str = "LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n";
    const STOPPED: &str = "LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n";

    #[test]
    fn green_has_no_banner_and_a_calm_line() {
        let s = snap(true, ON);
        let h = classify(&s);
        assert_eq!(banner(&s, &h), None);
        assert_eq!(status_line(&s, &h), "On · 1 rule");
        assert_eq!(
            policy_line(&s),
            "Incoming deny · Outgoing allow · Routed deny · Logging low"
        );
        let r = &s.rules.as_ref().unwrap().rules[0];
        assert_eq!(
            rule_texts(r, false),
            ("22/tcp".into(), "Allow in · SSH".into())
        );
        assert_eq!(
            rule_texts(r, true),
            ("22/tcp".into(), "Allow in · IPv4 only · SSH".into())
        );
        assert_eq!(delete_question(r, false), "Remove 22/tcp (Allow in · SSH)?");
    }

    #[test]
    fn amber_offers_its_fixes() {
        let s = snap(true, STOPPED);
        let h = classify(&s);
        let b = banner(&s, &h).unwrap();
        assert_eq!(b.tone, "warn");
        assert_eq!(
            b.actions,
            [
                BannerAction::Fix(Fix::StartUnit),
                BannerAction::Fix(Fix::EnableAtBoot)
            ]
        );
        assert_eq!(status_line(&s, &h), "On — not running · 1 rule");
    }

    #[test]
    fn unreadable_rules_offer_load_rules_even_when_green() {
        let mut s = snap(true, ON);
        s.rules = Err(ReadError::Denied);
        let h = classify(&s);
        let b = banner(&s, &h).unwrap();
        assert_eq!(
            (b.tone, b.actions.as_slice()),
            ("muted", &[BannerAction::LoadRules][..])
        );
        assert_eq!(status_line(&s, &h), "On");
        let off = snap(false, ON);
        let h = classify(&off);
        assert_eq!(banner(&off, &h), None, "off: the switch is the fix");
        assert_eq!(status_line(&off, &h), "Off — no rule is enforced · 1 rule");
        assert_eq!(
            status_line(&Snapshot::loading(), &classify(&Snapshot::loading())),
            "Reading…"
        );
    }
}
