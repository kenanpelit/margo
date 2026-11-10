//! ufw's saved state, read without root: `/etc/ufw/ufw.conf`,
//! `/etc/default/ufw` and the rule files `user.rules` / `user6.rules`.
//!
//! The rule files hold one machine-readable line above every rule ufw
//! manages (ufw's `backend_iptables.py`):
//!
//! ```text
//! ### tuple ### <action> <proto> <dport> <dst> <sport> <src> [<dapp> <sapp>] <dir>[_<iface>] [comment=<hex>]
//! ```
//!
//! * 6 fields (+dir = 7) for a plain rule, 8 (+dir = 9) with app
//!   profiles (`-` when unused, spaces as `%20`); without a direction
//!   (old files) it means `in`.
//! * action: `allow|deny|reject|limit`, with `_log` / `_log-all`; a
//!   route rule is `route:<action>`.
//! * dport / sport: `80`, `60000:60010`, `80,443` or `any`. dst / src:
//!   an address, a CIDR, or `0.0.0.0/0` / `::/0` (anywhere).
//! * dir: `in` / `out`, then `_<iface>`; a route rule naming both
//!   interfaces has `in_<a>!out_<b>`.
//! * `comment=` is the comment's bytes in hex, always last.
//! * `### END RULES ###` closes the list; without it the file was cut.
//!
//! A rule ufw keeps for both IPv4 and IPv6 appears in both files; the
//! two copies are merged into one [`Rule`] with `v4` and `v6` set.

use super::spec::{Action, Addr, Direction, Log, Ports, Proto, RuleSpec};
use std::collections::HashMap;

pub const UFW_CONF: &str = "/etc/ufw/ufw.conf";
pub const DEFAULTS: &str = "/etc/default/ufw";
pub const USER_RULES: &str = "/etc/ufw/user.rules";
pub const USER6_RULES: &str = "/etc/ufw/user6.rules";
/// A rule file larger than this isn't read (ufw's are a few KiB).
pub const MAX_FILE: u64 = 4 * 1024 * 1024;

const END_RULES: &str = "### END RULES ###";
const TUPLE: &str = "### tuple ###";

/// `ufw.conf`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conf {
    /// `ENABLED=yes|no`; `None` when the key is missing.
    pub enabled: Option<bool>,
    /// `LOGLEVEL` (`off`, `low`, `medium`, `high`, `full`).
    pub loglevel: Option<String>,
}

/// `/etc/default/ufw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    /// `DEFAULT_INPUT_POLICY` etc.: `DROP`, `ACCEPT` or `REJECT`.
    pub input: Option<String>,
    pub output: Option<String>,
    pub forward: Option<String>,
    /// `IPV6=yes` (the default when the key is missing).
    pub ipv6: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            input: None,
            output: None,
            forward: None,
            ipv6: true,
        }
    }
}

/// `KEY=value` / `KEY="value"` (first match, comments skipped).
fn setting(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim();
        if l.starts_with('#') {
            return None;
        }
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim().to_string())
    })
}

pub fn parse_conf(text: &str) -> Conf {
    Conf {
        enabled: setting(text, "ENABLED").map(|v| v.eq_ignore_ascii_case("yes")),
        loglevel: setting(text, "LOGLEVEL"),
    }
}

pub fn parse_defaults(text: &str) -> Defaults {
    Defaults {
        input: setting(text, "DEFAULT_INPUT_POLICY"),
        output: setting(text, "DEFAULT_OUTPUT_POLICY"),
        forward: setting(text, "DEFAULT_FORWARD_POLICY"),
        ipv6: setting(text, "IPV6").is_none_or(|v| !v.eq_ignore_ascii_case("no")),
    }
}

/// A policy word as ufw's status prints it (`DROP` → `deny`).
pub fn policy_word(p: &str) -> &'static str {
    match p.to_ascii_uppercase().as_str() {
        "DROP" => "deny",
        "ACCEPT" => "allow",
        "REJECT" => "reject",
        _ => "?",
    }
}

/// One rule ufw manages, both IP versions merged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    /// `allow`, `deny`, `reject`, `limit`.
    pub action: Action,
    pub log: Option<Log>,
    /// A `ufw route` (forwarding) rule.
    pub route: bool,
    pub direction: Direction,
    /// `eth9`, or `eth9>eth8` for a route rule naming both interfaces.
    pub iface: Option<String>,
    /// `tcp`, `udp`, `any`, or another IP protocol (`esp`, `gre`, …).
    pub proto: String,
    /// The destination port spec, `None` for any port.
    pub port: Option<String>,
    /// `any` or an address / network.
    pub from: String,
    pub from_port: Option<String>,
    pub to: String,
    /// Destination / source app profile (`allow OpenSSH`).
    pub app: Option<String>,
    pub sapp: Option<String>,
    pub comment: Option<String>,
    pub v4: bool,
    pub v6: bool,
}

fn anywhere(a: &str) -> String {
    match a {
        "" | "0.0.0.0/0" | "::/0" => "any".into(),
        a => a.into(),
    }
}

/// ufw writes a comment as its UTF-8 bytes in hex; anything else in
/// there is a broken line.
fn unhex(hex: &str) -> Option<String> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    Some(String::from_utf8_lossy(&bytes?).into_owned())
}

fn none_if_any(s: &str) -> Option<String> {
    (s != "any").then(|| s.to_string())
}

/// One `### tuple ###` line, `None` for any other line or a shape
/// this parser doesn't know.
pub fn parse_tuple(line: &str) -> Option<Rule> {
    let mut body = line.strip_prefix(TUPLE)?.trim();
    let mut comment = None;
    if let Some((rest, hex)) = body.rsplit_once(" comment=") {
        comment = unhex(hex.trim());
        comment.as_ref()?;
        body = rest.trim_end();
    }
    let t: Vec<&str> = body.split_whitespace().collect();
    let n = t.len();
    if !(6..=9).contains(&n) {
        return None;
    }
    let (mut direction, mut iface) = (Direction::In, None);
    if n == 7 || n == 9 {
        let last = t[n - 1];
        let (d, rest) = match last.split_once('_') {
            Some((d, r)) => (d, Some(r)),
            None => (last, None),
        };
        direction = Direction::parse(d)?;
        if let Some(rest) = rest {
            iface = Some(match rest.split_once("!out_") {
                Some((i, o)) => format!("{i}>{o}"),
                None => rest.to_string(),
            });
        }
    }
    let (route, action) = match t[0].split_once(':') {
        Some(("route", a)) => (true, a),
        Some(_) => return None,
        None => (false, t[0]),
    };
    let (action, log) = match action.split_once('_') {
        Some((a, l)) => (a, Some(Log::parse(l)?)),
        None => (action, None),
    };
    let app_field = |s: &str| (n >= 8 && s != "-").then(|| s.replace("%20", " "));
    Some(Rule {
        action: Action::parse(action)?,
        log,
        route,
        direction,
        iface,
        proto: t[1].to_string(),
        port: none_if_any(t[2]),
        to: anywhere(t[3]),
        from_port: none_if_any(t[4]),
        from: anywhere(t[5]),
        app: t.get(6).and_then(|s| app_field(s)),
        sapp: t.get(7).and_then(|s| app_field(s)),
        comment,
        v4: false,
        v6: false,
    })
}

/// The rules of both files, in file order (IPv4 first), and whether
/// they are complete: each file ended with `### END RULES ###` (with
/// IPv6 on, `user6.rules` is needed too, or every both-copies rule would
/// look IPv4-only and its delete would leave the IPv6 copy).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    pub complete: bool,
}

impl RuleSet {
    /// Incoming `allow` / `limit` rules (no route rules): what the bar
    /// pill counts.
    pub fn allowed_in(&self) -> usize {
        self.rules.iter().filter(|r| r.is_allowed_in()).count()
    }
}

/// The merge key: the rule without its comment and IP version.
fn key(r: &Rule) -> Rule {
    Rule {
        comment: None,
        v4: false,
        v6: false,
        ..r.clone()
    }
}

pub fn parse_rules(v4: &str, v6: Option<&str>, ipv6: bool) -> RuleSet {
    let mut rules: Vec<Rule> = Vec::new();
    let mut index = HashMap::new();
    let mut add_file = |text: &str, six: bool| -> bool {
        let mut ended = false;
        for line in text.lines() {
            if line.starts_with(END_RULES) {
                ended = true;
                continue;
            }
            let Some(rule) = parse_tuple(line) else {
                continue;
            };
            let i = *index.entry(key(&rule)).or_insert_with(|| {
                rules.push(rule.clone());
                rules.len() - 1
            });
            let r = &mut rules[i];
            if six {
                r.v6 = true;
            } else {
                r.v4 = true;
            }
            if r.comment.is_none() {
                r.comment = rule.comment;
            }
        }
        ended
    };
    let mut complete = add_file(v4, false);
    if ipv6 {
        complete &= v6.is_some_and(|t| add_file(t, true));
    }
    RuleSet { rules, complete }
}

impl Rule {
    pub fn is_allowed_in(&self) -> bool {
        matches!(self.action, Action::Allow | Action::Limit)
            && self.direction == Direction::In
            && !self.route
    }

    /// The exact rule spec a `ufw delete` of this rule needs, `None` for
    /// shapes the helper doesn't delete (route and app-profile rules, a
    /// source port, other protocols), and for a rule seen in only one
    /// file while the set is incomplete (unknown: maybe both).
    ///
    /// `any` matches both IP versions; for a rule that exists in one
    /// file only, `any` becomes that family's `0.0.0.0/0` / `::/0`, so
    /// the delete removes exactly what is listed.
    pub fn delete_spec(&self, complete: bool) -> Option<RuleSpec> {
        if self.route || self.app.is_some() || self.sapp.is_some() || self.from_port.is_some() {
            return None;
        }
        let both = self.v4 && self.v6;
        if !complete && !both {
            return None;
        }
        let family_any = match (self.v4, self.v6) {
            (true, true) => "any",
            (true, false) => "0.0.0.0/0",
            (false, true) => "::/0",
            (false, false) => return None,
        };
        let addr = |a: &str| Addr::parse(if a == "any" { family_any } else { a });
        let iface = match &self.iface {
            Some(i) if i.contains('>') => return None,
            Some(i) => Some(super::spec::iface(i)?),
            None => None,
        };
        let spec = RuleSpec {
            action: self.action,
            direction: self.direction,
            iface,
            log: self.log,
            proto: Proto::parse(&self.proto)?,
            from: addr(&self.from)?,
            to: addr(&self.to)?,
            ports: match &self.port {
                Some(p) => Some(Ports::parse(p)?),
                None => None,
            },
        };
        spec.check(false).ok()?;
        Some(spec)
    }

    /// `22/tcp`, `1714:1764/udp`, `5353`, `OpenSSH`, `any port`: the
    /// destination the way ufw's status shows it.
    pub fn target(&self) -> String {
        if let Some(app) = &self.app {
            return app.clone();
        }
        let port = self.port.clone().unwrap_or_else(|| "any port".into());
        let mut s = match (self.proto.as_str(), &self.port) {
            ("any", _) => port,
            (p, Some(_)) => format!("{port}/{p}"),
            (p, None) => format!("{port} {p}"),
        };
        if self.to != "any" {
            s = format!("{s} to {}", self.to);
        }
        s
    }

    /// `Allow in · from 192.168.0.0/24 · on eth0 · IPv4 only`.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "{}{} {}",
            if self.route { "Route " } else { "" },
            if self.route {
                self.action.keyword()
            } else {
                self.action.label()
            },
            self.direction.keyword()
        )];
        if self.from != "any" {
            let from = match &self.from_port {
                Some(p) => format!("from {} port {p}", self.from),
                None => format!("from {}", self.from),
            };
            parts.push(from);
        } else if let Some(p) = &self.from_port {
            parts.push(format!("from port {p}"));
        }
        if let Some(i) = &self.iface {
            parts.push(format!("on {}", i.replace('>', " → ")));
        }
        if let Some(l) = self.log {
            parts.push(l.keyword().to_string());
        }
        match (self.v4, self.v6) {
            (true, false) => parts.push("IPv4 only".into()),
            (false, true) => parts.push("IPv6 only".into()),
            _ => {}
        }
        parts.join(" · ")
    }
}

// ── The `ufw-status` dump (root read when the files aren't readable) ──

/// What `ufw-status` prints: four sections, each only the lines the
/// parsers above read (settings and tuple / end lines), so nothing else
/// of root's files reaches the caller.
pub fn dump(
    conf: Option<&str>,
    defaults: Option<&str>,
    v4: Option<&str>,
    v6: Option<&str>,
) -> String {
    let mut out = String::new();
    let mut section = |name: &str, text: Option<&str>, keep: &dyn Fn(&str) -> bool| {
        out.push_str("@@");
        out.push_str(name);
        out.push('\n');
        match text {
            None => out.push_str("@@missing\n"),
            Some(t) => {
                for l in t.lines().filter(|l| keep(l)) {
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
    };
    let key_line = |keys: &'static [&'static str]| {
        move |l: &str| {
            l.split_once('=')
                .is_some_and(|(k, _)| keys.contains(&k.trim()))
        }
    };
    let rule_line = |l: &str| l.starts_with(TUPLE) || l.starts_with(END_RULES);
    section("conf", conf, &key_line(&["ENABLED", "LOGLEVEL"]));
    section(
        "defaults",
        defaults,
        &key_line(&[
            "DEFAULT_INPUT_POLICY",
            "DEFAULT_OUTPUT_POLICY",
            "DEFAULT_FORWARD_POLICY",
            "IPV6",
        ]),
    );
    section("v4", v4, &rule_line);
    section("v6", v6, &rule_line);
    out.push_str("@@end\n");
    out
}

/// The four sections of a [`dump`]: `None` for a missing file. `None`
/// overall when the dump is cut (no `@@end`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dump {
    pub conf: Option<String>,
    pub defaults: Option<String>,
    pub v4: Option<String>,
    pub v6: Option<String>,
}

pub fn parse_dump(text: &str) -> Option<Dump> {
    let mut d = Dump::default();
    let mut cur: Option<(&str, String, bool)> = None;
    let mut ended = false;
    let flush = |cur: &mut Option<(&str, String, bool)>, d: &mut Dump| {
        if let Some((name, body, missing)) = cur.take() {
            let v = (!missing).then_some(body);
            match name {
                "conf" => d.conf = v,
                "defaults" => d.defaults = v,
                "v4" => d.v4 = v,
                "v6" => d.v6 = v,
                _ => {}
            }
        }
    };
    for line in text.lines() {
        match line.strip_prefix("@@") {
            Some("end") => {
                flush(&mut cur, &mut d);
                ended = true;
                break;
            }
            Some("missing") => {
                if let Some(c) = cur.as_mut() {
                    c.2 = true;
                }
            }
            Some(name @ ("conf" | "defaults" | "v4" | "v6")) => {
                flush(&mut cur, &mut d);
                cur = Some((name, String::new(), false));
            }
            Some(_) => return None,
            None => {
                if let Some(c) = cur.as_mut() {
                    c.1.push_str(line);
                    c.1.push('\n');
                }
            }
        }
    }
    ended.then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from this machine's /etc/ufw (ufw 0.36.2; LAN subnet and
    /// service ports only, nothing personal).
    const REAL_V4: &str = include_str!("../../tests/data/ufw/user.rules");
    const REAL_V6: &str = include_str!("../../tests/data/ufw/user6.rules");
    /// Every shape ufw 0.36.2 wrote in a bwrap sandbox (made-up rules).
    const SHAPES_V4: &str = include_str!("../../tests/data/ufw/shapes.rules");
    const SHAPES_V6: &str = include_str!("../../tests/data/ufw/shapes6.rules");

    #[test]
    fn real_rules_parse_and_merge() {
        let set = parse_rules(REAL_V4, Some(REAL_V6), true);
        assert!(set.complete);
        let targets: Vec<String> = set.rules.iter().map(Rule::target).collect();
        assert_eq!(
            targets,
            [
                "22/tcp",
                "5353/udp",
                "1714:1764/udp",
                "1714:1764/tcp",
                "13378/tcp",
                "8443/tcp",
                "53317/udp",
                "53317/tcp",
                "8096/tcp"
            ]
        );
        assert!(
            set.rules[0].v4 && set.rules[0].v6,
            "22/tcp is in both files"
        );
        assert!(
            set.rules[2].v4 && !set.rules[2].v6,
            "a v4 source is v4 only"
        );
        assert_eq!(set.rules[2].from, "192.168.0.0/24");
        assert_eq!(set.allowed_in(), 9);
        assert_eq!(
            set.rules[2].summary(),
            "Allow in · from 192.168.0.0/24 · IPv4 only"
        );
        assert_eq!(set.rules[0].summary(), "Allow in");
    }

    #[test]
    fn every_real_rule_is_deletable_exactly() {
        let set = parse_rules(REAL_V4, Some(REAL_V6), true);
        let both = set.rules[0].delete_spec(true).unwrap();
        assert_eq!(
            both.fields(),
            ["allow", "in", "-", "-", "tcp", "any", "any", "22"]
        );
        let lan = set.rules[3].delete_spec(true).unwrap();
        assert_eq!(
            lan.fields(),
            [
                "allow",
                "in",
                "-",
                "-",
                "tcp",
                "192.168.0.0/24",
                "0.0.0.0/0",
                "1714:1764"
            ]
        );
        assert!(set.rules.iter().all(|r| r.delete_spec(true).is_some()));
    }

    #[test]
    fn shapes_cover_what_ufw_writes() {
        let set = parse_rules(SHAPES_V4, Some(SHAPES_V6), true);
        assert!(set.complete);
        let find = |target: &str, sum: &str| {
            set.rules
                .iter()
                .find(|r| r.target() == target && r.summary() == sum)
                .unwrap_or_else(|| panic!("{target} / {sum} in {:#?}", set.rules))
        };
        let ssh = find("22/tcp", "Allow in");
        assert_eq!(ssh.comment.as_deref(), Some("ssh"));
        let printer = find("8080/tcp", "Reject in · from 2001:db8::/32 · IPv6 only");
        assert_eq!(printer.comment.as_deref(), Some("Yazıcı ağ"));
        let spec = printer.delete_spec(true).unwrap();
        assert_eq!(spec.to.to_string(), "::/0");
        let eth = find("43117", "Allow in · on eth9");
        assert_eq!(
            eth.delete_spec(true).unwrap().iface.as_deref(),
            Some("eth9")
        );
        let out = find("53/udp", "Deny out");
        assert_eq!(out.direction, Direction::Out);
        assert!(!out.is_allowed_in());
        let logged = find("41001/tcp", "Allow in · log");
        assert_eq!(logged.delete_spec(true).unwrap().log, Some(Log::Log));
        let list = find("80,443/tcp", "Allow in");
        assert_eq!(
            list.delete_spec(true).unwrap().ports.unwrap().to_string(),
            "80,443"
        );
        let host = find("any port", "Allow in · from 198.51.100.7 · IPv4 only");
        assert!(host.delete_spec(true).is_some());
        let dest = find(
            "41007/tcp to 10.0.0.0/8",
            "Allow in · from port 41005 · IPv4 only",
        );
        assert_eq!(
            dest.delete_spec(true),
            None,
            "a source port isn't deleted from here"
        );
        let app = set.rules.iter().find(|r| r.app.is_some()).unwrap();
        assert_eq!(app.target(), "WWW Full");
        assert_eq!(app.delete_spec(true), None);
        let route = set.rules.iter().find(|r| r.route).unwrap();
        assert_eq!(route.iface.as_deref(), Some("eth9>eth8"));
        assert_eq!(route.delete_spec(true), None);
        assert!(!route.is_allowed_in());
        let esp = set.rules.iter().find(|r| r.proto == "esp").unwrap();
        assert_eq!(esp.delete_spec(true), None);
    }

    #[test]
    fn a_cut_or_missing_v6_file_makes_the_set_incomplete() {
        let cut = REAL_V6.replace(END_RULES, "");
        let set = parse_rules(REAL_V4, Some(&cut), true);
        assert!(!set.complete);
        // Both-copies rules are still safe to delete; one-file ones aren't.
        assert!(set.rules[0].delete_spec(false).is_some());
        assert!(set.rules[2].delete_spec(false).is_none());
        assert!(!parse_rules(REAL_V4, None, true).complete);
        let v4_only = parse_rules(REAL_V4, None, false);
        assert!(v4_only.complete, "IPV6=no needs only user.rules");
    }

    #[test]
    fn garbage_tuples_are_skipped() {
        for bad in [
            "### tuple ### allow tcp 22",
            "### tuple ### accept tcp 22 0.0.0.0/0 any 0.0.0.0/0 in",
            "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 sideways",
            "### tuple ### allow_verbose tcp 22 0.0.0.0/0 any 0.0.0.0/0 in",
            "### tuple ### foo:allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in",
            "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=zz",
            "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=abc",
            "### tuple ### a b c d e f g h i j",
            "-A ufw-user-input -p tcp --dport 22 -j ACCEPT",
        ] {
            assert_eq!(parse_tuple(bad), None, "{bad}");
        }
        let lossy =
            parse_tuple("### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=61ff")
                .unwrap();
        assert_eq!(
            lossy.comment.as_deref(),
            Some("a\u{fffd}"),
            "a rule is never hidden"
        );
        let old = parse_tuple("### tuple ### deny udp 53 0.0.0.0/0 any 0.0.0.0/0").unwrap();
        assert_eq!(old.direction, Direction::In, "no direction = in");
    }

    #[test]
    fn settings() {
        let c = parse_conf("# /etc/ufw/ufw.conf\nENABLED=yes\nLOGLEVEL=low\n");
        assert_eq!(c.enabled, Some(true));
        assert_eq!(c.loglevel.as_deref(), Some("low"));
        assert_eq!(parse_conf("ENABLED=\"no\"\n").enabled, Some(false));
        assert_eq!(parse_conf("#ENABLED=yes\n").enabled, None);
        let d = parse_defaults(
            "DEFAULT_INPUT_POLICY=\"DROP\"\nDEFAULT_OUTPUT_POLICY=\"ACCEPT\"\nDEFAULT_FORWARD_POLICY=\"DROP\"\nIPV6=yes\n",
        );
        assert_eq!(d.input.as_deref().map(policy_word), Some("deny"));
        assert_eq!(d.output.as_deref().map(policy_word), Some("allow"));
        assert!(d.ipv6);
        assert!(!parse_defaults("IPV6=no").ipv6);
        assert!(parse_defaults("").ipv6);
    }

    #[test]
    fn the_root_dump_round_trips_and_keeps_only_what_is_parsed() {
        let conf = "# comment\nENABLED=yes\nLOGLEVEL=low\nSECRET=x\n";
        let text = dump(Some(conf), None, Some(REAL_V4), Some(REAL_V6));
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("-A ufw-user-input"));
        let d = parse_dump(&text).unwrap();
        assert_eq!(d.defaults, None);
        assert_eq!(parse_conf(d.conf.as_deref().unwrap()).enabled, Some(true));
        let set = parse_rules(d.v4.as_deref().unwrap(), d.v6.as_deref(), true);
        assert_eq!(set, parse_rules(REAL_V4, Some(REAL_V6), true));
        assert_eq!(parse_dump(text.trim_end_matches("@@end\n")), None, "cut");
        assert_eq!(parse_dump("@@bogus\n@@end\n"), None);
    }
}
