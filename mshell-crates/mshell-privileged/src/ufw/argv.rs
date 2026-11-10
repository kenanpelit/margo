//! The fixed argv the helper runs `ufw` and `systemctl` with, built from
//! validated [`super::spec`] values only, and how it reads ufw's answer.
//! Pure: [`crate::run`] does the exec.

use super::spec::{AddRule, Ports, Proto, RuleSpec};

/// The only unit the `ufw-unit` verb touches.
pub const UNIT: &str = "ufw.service";

/// `systemctl` arguments for the unprivileged state read.
pub const SHOW_ARGS: [&str; 3] = [
    "show",
    UNIT,
    "--property=LoadState,ActiveState,SubState,UnitFileState",
];

/// Turn ufw on or off, or reload its rules (`ufw-switch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// `ufw --force enable`, then start the unit when it isn't running
    /// (one prompt for both; see [`crate::run`]).
    On,
    /// `ufw disable`: flush the rules and set `ENABLED=no` (persists
    /// across reboots, as `ufw disable` always does).
    Off,
    /// `ufw reload`.
    Reload,
}

impl Switch {
    pub fn keyword(self) -> &'static str {
        match self {
            Switch::On => "on",
            Switch::Off => "off",
            Switch::Reload => "reload",
        }
    }

    pub fn parse(s: &str) -> Option<Switch> {
        match s {
            "on" => Some(Switch::On),
            "off" => Some(Switch::Off),
            "reload" => Some(Switch::Reload),
            _ => None,
        }
    }
}

/// A `systemctl` verb on [`UNIT`] (`ufw-unit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitOp {
    Start,
    Stop,
    /// Start at boot (`systemctl enable`).
    Enable,
    /// Don't start at boot (`systemctl disable`).
    Disable,
}

impl UnitOp {
    pub fn keyword(self) -> &'static str {
        match self {
            UnitOp::Start => "start",
            UnitOp::Stop => "stop",
            UnitOp::Enable => "enable",
            UnitOp::Disable => "disable",
        }
    }

    pub fn parse(s: &str) -> Option<UnitOp> {
        match s {
            "start" => Some(UnitOp::Start),
            "stop" => Some(UnitOp::Stop),
            "enable" => Some(UnitOp::Enable),
            "disable" => Some(UnitOp::Disable),
            _ => None,
        }
    }
}

/// The rule after its action word: `in [on IF] [log] [proto P] from A to
/// B [port P]` — the full form ufw documents, so no word is ambiguous.
fn rule_words(spec: &RuleSpec, proto: Proto) -> Vec<String> {
    let mut v = vec![spec.direction.keyword().to_string()];
    if let Some(i) = &spec.iface {
        v.push("on".into());
        v.push(i.clone());
    }
    if let Some(l) = spec.log {
        v.push(l.keyword().into());
    }
    if proto != Proto::Any {
        v.push("proto".into());
        v.push(proto.keyword().into());
    }
    v.push("from".into());
    v.push(spec.from.to_string());
    v.push("to".into());
    v.push(spec.to.to_string());
    if let Some(p) = &spec.ports {
        v.push("port".into());
        v.push(p.to_string());
    }
    v
}

/// The ufw argvs (after the binary) of an add: one, or a TCP and a UDP
/// one for `Any` over a range / list (ufw refuses `any` there). No
/// `--force`: an add never prompts, and `ufw --force allow …` would be
/// parsed as `ufw default allow`.
pub fn add_argvs(add: &AddRule) -> Vec<Vec<String>> {
    let s = &add.spec;
    let multi = s.ports.as_ref().is_some_and(Ports::is_multi);
    let protos: &[Proto] = if s.proto == Proto::Any && multi {
        &[Proto::Tcp, Proto::Udp]
    } else {
        std::slice::from_ref(&s.proto)
    };
    protos
        .iter()
        .map(|p| {
            let mut v = vec![s.action.keyword().to_string()];
            v.extend(rule_words(s, *p));
            if let Some(c) = &add.comment {
                v.push("comment".into());
                v.push(c.clone());
            }
            v
        })
        .collect()
}

/// The ufw argv of a delete by rule (both the IPv4 and IPv6 copy when
/// the addresses are `any`; [`super::rules::Rule::delete_spec`] scopes
/// them to one family for a one-family rule). `--force` skips the
/// confirmation question.
pub fn delete_argv(spec: &RuleSpec) -> Vec<String> {
    let mut v = vec![
        "--force".to_string(),
        "delete".into(),
        spec.action.keyword().into(),
    ];
    v.extend(rule_words(spec, spec.proto));
    v
}

pub fn switch_argv(s: Switch) -> Vec<String> {
    match s {
        Switch::On => vec!["--force".into(), "enable".into()],
        Switch::Off => vec!["disable".into()],
        Switch::Reload => vec!["reload".into()],
    }
}

/// `systemctl <verb> ufw.service`.
pub fn unit_argv(op: UnitOp) -> Vec<String> {
    vec![op.keyword().into(), UNIT.into()]
}

/// What a ufw run means: ufw exits 0 on several non-changes, so its
/// output decides too. `Err` carries the line to show.
///
/// * a non-zero exit: its last line (`ERROR: Bad port '0'` → `Bad port '0'`);
/// * `Skipping unsupported …` (a `limit` without the recent module), or a
///   `Could not …` other than a one-family miss next to a deletion;
/// * every line `Skipping adding existing rule`: nothing was added.
pub fn ufw_outcome(code: Option<i32>, output: &str) -> Result<(), String> {
    // ufw's own warnings (`WARN: Rule changed after normalization`, the
    // ownership checks) and iptables' `Warning:` lines say nothing about
    // whether the change happened.
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("WARN") && !l.starts_with("Warning"))
        .collect();
    let clean = |l: &str| l.trim_start_matches("ERROR:").trim().to_string();
    match code {
        Some(0) => {}
        Some(c) => {
            return Err(lines
                .iter()
                .rev()
                .find(|l| l.starts_with("ERROR"))
                .or(lines.last())
                .map_or_else(|| format!("ufw failed (exit {c})"), |l| clean(l)));
        }
        None => return Err("ufw was killed".into()),
    }
    let changed = lines
        .iter()
        .any(|l| l.starts_with("Rule added") || l.starts_with("Rule deleted"));
    for l in &lines {
        let skip_existing = l.starts_with("Skipping adding existing rule");
        let miss = l.starts_with("Could not delete non-existent rule");
        if l.starts_with("ERROR") {
            return Err(clean(l));
        }
        if (l.starts_with("Skipping") && !skip_existing) || (l.starts_with("Could not") && !miss) {
            return Err((*l).to_string());
        }
        if miss && !changed {
            return Err("That rule doesn't exist (any more)".into());
        }
    }
    if !lines.is_empty()
        && lines
            .iter()
            .all(|l| l.starts_with("Skipping adding existing rule"))
    {
        return Err("That rule already exists".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::spec::{Action, Addr, Direction, Log, RuleSpec};
    use super::*;

    fn spec(action: Action, ports: &str, proto: Proto, from: &str) -> RuleSpec {
        RuleSpec::incoming(
            action,
            Ports::parse(ports),
            proto,
            Addr::parse(from).unwrap(),
        )
    }

    #[test]
    fn add_argv_is_exact() {
        let add = AddRule {
            spec: spec(Action::Allow, "22", Proto::Tcp, "any"),
            comment: Some("SSH".into()),
        };
        assert_eq!(
            add_argvs(&add),
            [vec![
                "allow", "in", "proto", "tcp", "from", "any", "to", "any", "port", "22", "comment",
                "SSH"
            ]]
        );
        let any = AddRule {
            spec: spec(Action::Limit, "5353", Proto::Any, "192.168.0.0/24"),
            comment: None,
        };
        assert_eq!(
            add_argvs(&any),
            [vec![
                "limit",
                "in",
                "from",
                "192.168.0.0/24",
                "to",
                "any",
                "port",
                "5353"
            ]]
        );
    }

    #[test]
    fn any_over_a_range_adds_tcp_and_udp() {
        let add = AddRule {
            spec: spec(Action::Allow, "1714:1764", Proto::Any, "192.168.0.0/24"),
            comment: Some("KDE Connect".into()),
        };
        let tail = [
            "from",
            "192.168.0.0/24",
            "to",
            "any",
            "port",
            "1714:1764",
            "comment",
            "KDE Connect",
        ];
        let mut tcp = vec!["allow", "in", "proto", "tcp"];
        tcp.extend(tail);
        let mut udp = vec!["allow", "in", "proto", "udp"];
        udp.extend(tail);
        assert_eq!(add_argvs(&add), [tcp, udp]);
    }

    #[test]
    fn delete_argv_is_exact() {
        assert_eq!(
            delete_argv(&spec(Action::Deny, "53", Proto::Udp, "0.0.0.0/0")),
            [
                "--force",
                "delete",
                "deny",
                "in",
                "proto",
                "udp",
                "from",
                "0.0.0.0/0",
                "to",
                "any",
                "port",
                "53"
            ]
        );
        let s = RuleSpec {
            action: Action::Allow,
            direction: Direction::In,
            iface: Some("eth9".into()),
            log: Some(Log::LogAll),
            proto: Proto::Any,
            from: Addr::Any,
            to: Addr::Any,
            ports: Ports::parse("43117"),
        };
        assert_eq!(
            delete_argv(&s),
            [
                "--force", "delete", "allow", "in", "on", "eth9", "log-all", "from", "any", "to",
                "any", "port", "43117"
            ]
        );
        let host = RuleSpec::incoming(
            Action::Allow,
            None,
            Proto::Any,
            Addr::parse("198.51.100.7").unwrap(),
        );
        assert_eq!(
            delete_argv(&host),
            [
                "--force",
                "delete",
                "allow",
                "in",
                "from",
                "198.51.100.7",
                "to",
                "any"
            ]
        );
    }

    #[test]
    fn switch_and_unit_argv_are_fixed() {
        assert_eq!(switch_argv(Switch::On), ["--force", "enable"]);
        assert_eq!(switch_argv(Switch::Off), ["disable"]);
        assert_eq!(switch_argv(Switch::Reload), ["reload"]);
        assert_eq!(unit_argv(UnitOp::Start), ["start", "ufw.service"]);
        assert_eq!(unit_argv(UnitOp::Stop), ["stop", "ufw.service"]);
        assert_eq!(unit_argv(UnitOp::Enable), ["enable", "ufw.service"]);
        assert_eq!(unit_argv(UnitOp::Disable), ["disable", "ufw.service"]);
        for s in [Switch::On, Switch::Off, Switch::Reload] {
            assert_eq!(Switch::parse(s.keyword()), Some(s));
        }
        for u in [UnitOp::Start, UnitOp::Stop, UnitOp::Enable, UnitOp::Disable] {
            assert_eq!(UnitOp::parse(u.keyword()), Some(u));
        }
        assert_eq!(Switch::parse("reset"), None);
        assert_eq!(UnitOp::parse("mask"), None);
    }

    /// Outputs captured from ufw 0.36.2 in a bwrap sandbox.
    #[test]
    fn ufw_output_decides_the_outcome() {
        assert_eq!(
            ufw_outcome(Some(0), "Rule added\nRule added (v6)\n"),
            Ok(())
        );
        assert_eq!(ufw_outcome(Some(0), "Rule deleted (v6)\n"), Ok(()));
        assert_eq!(
            ufw_outcome(Some(0), "Rules updated\nRules updated (v6)\n"),
            Ok(())
        );
        assert_eq!(
            ufw_outcome(
                Some(0),
                "Firewall is active and enabled on system startup\n"
            ),
            Ok(())
        );
        assert_eq!(ufw_outcome(Some(0), "Firewall reloaded\n"), Ok(()));
        // Only the v6 copy was there already: the rule now exists.
        assert_eq!(
            ufw_outcome(Some(0), "Rule added\nSkipping adding existing rule (v6)\n"),
            Ok(())
        );
        // A both-copies delete of a rule that had one copy: it's gone.
        assert_eq!(
            ufw_outcome(
                Some(0),
                "Rule deleted\nCould not delete non-existent rule (v6)\n"
            ),
            Ok(())
        );
        assert_eq!(
            ufw_outcome(
                Some(0),
                "Skipping adding existing rule\nSkipping adding existing rule (v6)\n"
            )
            .unwrap_err(),
            "That rule already exists"
        );
        assert_eq!(
            ufw_outcome(
                Some(0),
                "Could not delete non-existent rule\nCould not delete non-existent rule (v6)\n"
            )
            .unwrap_err(),
            "That rule doesn't exist (any more)"
        );
        // Warnings around the answer don't change it.
        assert_eq!(
            ufw_outcome(
                Some(0),
                "Skipping adding existing rule\nWARN: uid is 0 but '/etc' is owned by 65534\n"
            )
            .unwrap_err(),
            "That rule already exists"
        );
        assert_eq!(
            ufw_outcome(
                Some(0),
                "WARN: Rule changed after normalization\nRule added\n"
            ),
            Ok(())
        );
        assert_eq!(
            ufw_outcome(Some(0), "Skipping unsupported IPv4 'limit' rule\n").unwrap_err(),
            "Skipping unsupported IPv4 'limit' rule"
        );
        assert_eq!(
            ufw_outcome(Some(1), "ERROR: Bad port '0'\n").unwrap_err(),
            "Bad port '0'"
        );
        assert_eq!(
            ufw_outcome(
                Some(1),
                "\nUsage: ufw COMMAND\n...\nERROR: Invalid syntax\n"
            )
            .unwrap_err(),
            "Invalid syntax"
        );
        assert_eq!(ufw_outcome(Some(1), "").unwrap_err(), "ufw failed (exit 1)");
        assert!(ufw_outcome(None, "").is_err());
    }
}
