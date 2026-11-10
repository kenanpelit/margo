//! The fields of a UFW rule as the helper takes them, and their
//! validators.
//!
//! Every field has one strict, canonical text form (what goes on the
//! helper's argv and what [`crate::Request::parse`] accepts as root) and,
//! for the fields a person types, a lenient input parser the shell's
//! add-rule form and `mshellctl ufw` run live ([`AddForm::validate`]), so
//! the form refuses exactly what root would refuse, with a message for
//! the field at fault. Nothing here builds a command line: that is
//! [`super::argv`], from these validated values only.

use crate::Error;
use std::fmt;
use std::net::IpAddr;

/// The most ports ufw takes in one rule (a range counts as two).
pub const MAX_PORT_WEIGHT: usize = 15;
/// The longest comment the form accepts, in characters.
pub const MAX_COMMENT_CHARS: usize = 64;
/// The longest interface name Linux has (`IFNAMSIZ - 1`).
const MAX_IFACE: usize = 15;
/// The placeholder for an absent optional field on the helper's argv
/// (no interface, no port, no comment). No real value can be `-`: every
/// validator refuses a leading `-`.
pub const NONE: &str = "-";

fn invalid(what: &str, arg: &str) -> Error {
    Error::Invalid(format!("invalid {what}: {arg}"))
}

/// What a rule does with matching traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Allow,
    Deny,
    Reject,
    /// Allow, but refuse an address that opens 6+ connections in 30 s.
    Limit,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::Allow, Action::Deny, Action::Reject, Action::Limit];

    pub fn keyword(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Deny => "deny",
            Action::Reject => "reject",
            Action::Limit => "limit",
        }
    }

    /// Title-case label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Action::Allow => "Allow",
            Action::Deny => "Deny",
            Action::Reject => "Reject",
            Action::Limit => "Limit",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.keyword() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    pub fn keyword(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }

    pub fn parse(s: &str) -> Option<Direction> {
        match s {
            "in" => Some(Direction::In),
            "out" => Some(Direction::Out),
            _ => None,
        }
    }
}

/// The protocols a rule from here may name. `Any` with a single port
/// is one ufw rule that matches TCP and UDP; with a range or a list ufw
/// needs a protocol, so an add expands it into a TCP and a UDP rule
/// ([`super::argv::add_argvs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proto {
    Tcp,
    Udp,
    Any,
}

impl Proto {
    pub fn keyword(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
            Proto::Any => "any",
        }
    }

    pub fn parse(s: &str) -> Option<Proto> {
        match s {
            "tcp" => Some(Proto::Tcp),
            "udp" => Some(Proto::Udp),
            "any" => Some(Proto::Any),
            _ => None,
        }
    }
}

/// Logging on a rule (`ufw allow log 22/tcp`). Only ever read from a
/// rule that has it, so its delete can name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Log {
    Log,
    LogAll,
}

impl Log {
    pub fn keyword(self) -> &'static str {
        match self {
            Log::Log => "log",
            Log::LogAll => "log-all",
        }
    }

    pub fn parse(s: &str) -> Option<Log> {
        match s {
            "log" => Some(Log::Log),
            "log-all" => Some(Log::LogAll),
            _ => None,
        }
    }
}

/// One element of a port spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortItem {
    One(u16),
    /// `lo:hi`, `lo < hi` (ufw refuses `hi:lo` and `n:n`).
    Range(u16, u16),
}

impl fmt::Display for PortItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortItem::One(p) => write!(f, "{p}"),
            PortItem::Range(a, b) => write!(f, "{a}:{b}"),
        }
    }
}

/// A destination port spec: one port, a range, or a comma list of
/// those, at most [`MAX_PORT_WEIGHT`] ports (a range counts as two).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ports(Vec<PortItem>);

/// A port number in canonical decimal (no sign, no leading zero), 1-65535.
fn port_number(s: &str) -> Option<u16> {
    let ok = !s.is_empty()
        && s.len() <= 5
        && s.bytes().all(|b| b.is_ascii_digit())
        && !s.starts_with('0');
    ok.then(|| s.parse::<u16>().ok()).flatten()
}

impl Ports {
    pub fn items(&self) -> &[PortItem] {
        &self.0
    }

    /// One port, `a:b`, or `a,b:c,…` in canonical form.
    pub fn parse(s: &str) -> Option<Ports> {
        if s.is_empty() || s.len() > 128 {
            return None;
        }
        let mut items = Vec::new();
        for part in s.split(',') {
            let item = match part.split_once(':') {
                Some((a, b)) => {
                    let (a, b) = (port_number(a)?, port_number(b)?);
                    if a >= b {
                        return None;
                    }
                    PortItem::Range(a, b)
                }
                None => PortItem::One(port_number(part)?),
            };
            items.push(item);
        }
        Ports::new(items)
    }

    /// `None` for an empty list or one over [`MAX_PORT_WEIGHT`].
    pub fn new(items: Vec<PortItem>) -> Option<Ports> {
        let weight: usize = items
            .iter()
            .map(|i| match i {
                PortItem::One(_) => 1,
                PortItem::Range(..) => 2,
            })
            .sum();
        (!items.is_empty() && weight <= MAX_PORT_WEIGHT).then_some(Ports(items))
    }

    pub fn one(port: u16) -> Option<Ports> {
        (port != 0).then(|| Ports(vec![PortItem::One(port)]))
    }

    /// More than one port: ufw then needs `proto tcp|udp`.
    pub fn is_multi(&self) -> bool {
        self.0.len() > 1 || matches!(self.0.first(), Some(PortItem::Range(..)))
    }
}

impl fmt::Display for Ports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, item) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{item}")?;
        }
        Ok(())
    }
}

/// A source or destination: anywhere, a host, or a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Addr {
    Any,
    Host(IpAddr),
    /// A network with its host bits clear (`0.0.0.0/0` and `::/0` too:
    /// "anywhere" of one family only, which a delete of a rule that
    /// exists for one family names).
    Net(IpAddr, u8),
}

fn max_prefix(ip: IpAddr) -> u8 {
    if ip.is_ipv4() { 32 } else { 128 }
}

/// `ip` with every bit past `prefix` cleared.
fn masked(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(a) => {
            let bits = u32::from(a);
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V4((bits & mask).into())
        }
        IpAddr::V6(a) => {
            let bits = u128::from(a);
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V6((bits & mask).into())
        }
    }
}

impl Addr {
    /// `any`, an address, or `address/prefix` with the host bits clear
    /// (the form ufw stores; ufw itself warns and rewrites anything else).
    pub fn parse(s: &str) -> Option<Addr> {
        if s == "any" {
            return Some(Addr::Any);
        }
        if s.is_empty() || s.len() > 64 {
            return None;
        }
        match s.split_once('/') {
            None => s.parse::<IpAddr>().ok().map(Addr::Host),
            Some((ip, prefix)) => {
                let ip = ip.parse::<IpAddr>().ok()?;
                let ok = !prefix.is_empty()
                    && prefix.len() <= 3
                    && prefix.bytes().all(|b| b.is_ascii_digit())
                    && (prefix == "0" || !prefix.starts_with('0'));
                let prefix: u8 = ok.then(|| prefix.parse().ok()).flatten()?;
                (prefix <= max_prefix(ip) && masked(ip, prefix) == ip)
                    .then_some(Addr::Net(ip, prefix))
            }
        }
    }

    /// The address family, `None` for anywhere.
    pub fn is_v6(self) -> Option<bool> {
        match self {
            Addr::Any => None,
            Addr::Host(ip) | Addr::Net(ip, _) => Some(ip.is_ipv6()),
        }
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Addr::Any => f.write_str("any"),
            Addr::Host(ip) => write!(f, "{ip}"),
            Addr::Net(ip, p) => write!(f, "{ip}/{p}"),
        }
    }
}

/// An interface name a rule may name: 1-15 of `A-Za-z0-9_.:@-`, not
/// starting with `-`, optionally ending in ufw's `+` wildcard.
pub fn iface(s: &str) -> Option<String> {
    let body = s.strip_suffix('+').unwrap_or(s);
    let ok = !body.is_empty()
        && s.len() <= MAX_IFACE
        && !body.starts_with('-')
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'@' | b'-'));
    ok.then(|| s.to_string())
}

/// Comment punctuation the form takes besides letters, digits and
/// spaces. No quote, backslash, `$`, backtick, `;`, `|`, `<`, `>` (ufw
/// itself chokes on quotes, and none of these belongs in a label).
const COMMENT_PUNCT: &str = "-_.,:/+()@#=!?%&*";

/// The words ufw's parser takes as its own even inside a comment.
const COMMENT_RESERVED: [&str; 4] = ["in", "out", "log", "log-all"];

/// Why a comment is refused, `None` when it is fine. `s` is taken as is
/// (the lenient form trims first).
pub fn comment_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() {
        return Some("the comment is empty");
    }
    if s.chars().count() > MAX_COMMENT_CHARS {
        return Some("the comment is longer than 64 characters");
    }
    if s.starts_with('-') {
        return Some("a comment can't start with '-'");
    }
    if s.starts_with(' ') || s.ends_with(' ') {
        return Some("a comment can't start or end with a space");
    }
    if COMMENT_RESERVED.iter().any(|w| s.eq_ignore_ascii_case(w)) {
        return Some("ufw takes this word as part of the rule; add a word to the comment");
    }
    if !s
        .chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || COMMENT_PUNCT.contains(c))
    {
        return Some("letters, digits, spaces and - _ . , : / + ( ) @ # = ! ? % & * only");
    }
    None
}

/// A rule's shape: everything ufw needs to add it or find it again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleSpec {
    pub action: Action,
    pub direction: Direction,
    pub iface: Option<String>,
    pub log: Option<Log>,
    pub proto: Proto,
    pub from: Addr,
    pub to: Addr,
    pub ports: Option<Ports>,
}

impl RuleSpec {
    /// An incoming rule for `ports`/`proto`, from `from`, to anywhere.
    pub fn incoming(action: Action, ports: Option<Ports>, proto: Proto, from: Addr) -> RuleSpec {
        RuleSpec {
            action,
            direction: Direction::In,
            iface: None,
            log: None,
            proto,
            from,
            to: Addr::Any,
            ports,
        }
    }

    /// The cross-field checks (each field is already valid on its own).
    /// `add`: an add may expand `Any` over several ports (into TCP + UDP)
    /// but must name a port or an address; a delete matches a rule that
    /// exists, so it names exactly one protocol for several ports.
    pub fn check(&self, add: bool) -> Result<(), Error> {
        if let (Some(a), Some(b)) = (self.from.is_v6(), self.to.is_v6())
            && a != b
        {
            return Err(Error::Invalid(
                "the source and destination are different IP versions".into(),
            ));
        }
        let multi = self.ports.as_ref().is_some_and(Ports::is_multi);
        if multi && self.proto == Proto::Any && !add {
            return Err(Error::Invalid(
                "a port range or list needs tcp or udp".into(),
            ));
        }
        if add && self.ports.is_none() && self.from == Addr::Any && self.to == Addr::Any {
            return Err(Error::Invalid(
                "a rule needs a port or an address (this one would match all traffic)".into(),
            ));
        }
        Ok(())
    }

    /// The helper argv fields: action, direction, interface, log,
    /// protocol, from, to, ports ([`NONE`] for an absent one).
    pub fn fields(&self) -> Vec<String> {
        vec![
            self.action.keyword().to_string(),
            self.direction.keyword().to_string(),
            self.iface.clone().unwrap_or_else(|| NONE.to_string()),
            self.log.map_or(NONE, Log::keyword).to_string(),
            self.proto.keyword().to_string(),
            self.from.to_string(),
            self.to.to_string(),
            self.ports
                .as_ref()
                .map_or_else(|| NONE.to_string(), Ports::to_string),
        ]
    }

    /// Parse [`Self::fields`] (8 strings), strictly.
    pub fn from_fields(f: &[&str]) -> Result<RuleSpec, Error> {
        let [action, direction, ifc, log, proto, from, to, ports] = f else {
            return Err(Error::Usage(format!(
                "a rule takes 8 fields, got {}",
                f.len()
            )));
        };
        Ok(RuleSpec {
            action: Action::parse(action)
                .ok_or_else(|| invalid("action (allow, deny, reject or limit)", action))?,
            direction: Direction::parse(direction)
                .ok_or_else(|| invalid("direction (in or out)", direction))?,
            iface: match *ifc {
                NONE => None,
                s => Some(iface(s).ok_or_else(|| invalid("interface", s))?),
            },
            log: match *log {
                NONE => None,
                s => Some(Log::parse(s).ok_or_else(|| invalid("log (log or log-all)", s))?),
            },
            proto: Proto::parse(proto)
                .ok_or_else(|| invalid("protocol (tcp, udp or any)", proto))?,
            from: Addr::parse(from).ok_or_else(|| invalid("source address", from))?,
            to: Addr::parse(to).ok_or_else(|| invalid("destination address", to))?,
            ports: match *ports {
                NONE => None,
                s => Some(
                    Ports::parse(s).ok_or_else(|| invalid("port (1-65535, a:b, or a list)", s))?,
                ),
            },
        })
    }

    /// `22/tcp`, `1714:1764/udp`, `5353`, `anywhere` (no port): the
    /// destination as ufw's own status shows it.
    pub fn port_label(&self) -> String {
        let ports = self
            .ports
            .as_ref()
            .map_or_else(|| "any port".to_string(), Ports::to_string);
        match (self.proto, &self.ports) {
            (Proto::Any, _) => ports,
            (p, Some(_)) => format!("{ports}/{}", p.keyword()),
            (p, None) => format!("{ports} {}", p.keyword()),
        }
    }
}

/// An add: a rule plus its optional comment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddRule {
    pub spec: RuleSpec,
    pub comment: Option<String>,
}

impl AddRule {
    pub fn fields(&self) -> Vec<String> {
        let mut f = self.spec.fields();
        f.push(self.comment.clone().unwrap_or_else(|| NONE.to_string()));
        f
    }

    /// Parse [`Self::fields`] (9 strings), strictly, with the add checks.
    pub fn from_fields(f: &[&str]) -> Result<AddRule, Error> {
        let Some((comment, spec)) = f.split_last() else {
            return Err(Error::Usage("an add takes 9 fields, got 0".into()));
        };
        if f.len() != 9 {
            return Err(Error::Usage(format!(
                "an add takes 9 fields, got {}",
                f.len()
            )));
        }
        let spec = RuleSpec::from_fields(spec)?;
        spec.check(true)?;
        let comment = match *comment {
            NONE => None,
            c => match comment_problem(c) {
                None => Some(c.to_string()),
                Some(why) => return Err(Error::Invalid(format!("invalid comment: {why}"))),
            },
        };
        Ok(AddRule { spec, comment })
    }
}

// ── Lenient input (what a person types) ──────────────────────────────

/// The field of the add form a problem belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Port,
    From,
    Comment,
}

/// A form problem: which field, and what to tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormError {
    pub field: Field,
    pub message: String,
}

fn form_err(field: Field, message: impl Into<String>) -> FormError {
    FormError {
        field,
        message: message.into(),
    }
}

/// A typed port spec: `22`, `8000:8100` (or `8000-8100`), `80,443`, with
/// spaces and leading zeros forgiven. An optional `/tcp` or `/udp`
/// suffix (as in `ufw allow 22/tcp`) is returned as the protocol.
pub fn parse_port_input(text: &str) -> Result<(Ports, Option<Proto>), String> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return Err("enter a port, a range like 8000:8100, or a list like 80,443".into());
    }
    let (body, proto) = match t.rsplit_once('/') {
        Some((b, "tcp")) => (b, Some(Proto::Tcp)),
        Some((b, "udp")) => (b, Some(Proto::Udp)),
        Some(_) => return Err("the protocol after '/' must be tcp or udp".into()),
        None => (t.as_str(), None),
    };
    let num = |s: &str| -> Result<u16, String> {
        let digits = s.trim_start_matches('0');
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("'{s}' is not a port number"));
        }
        match digits.parse::<u32>() {
            Ok(n) if (1..=65535).contains(&n) => Ok(n as u16),
            _ if digits.is_empty() => Err("port 0 isn't a port; use 1-65535".into()),
            _ => Err(format!("{s} is out of range; ports are 1-65535")),
        }
    };
    let mut items = Vec::new();
    for part in body.split(',') {
        let range = part.split_once(':').or_else(|| part.split_once('-'));
        items.push(match range {
            Some((a, b)) => {
                let (a, b) = (num(a)?, num(b)?);
                if a >= b {
                    return Err(format!("a range goes from low to high ({b}:{a}?)"));
                }
                PortItem::Range(a, b)
            }
            None => PortItem::One(num(part)?),
        });
    }
    let ports = Ports::new(items)
        .ok_or_else(|| format!("at most {MAX_PORT_WEIGHT} ports per rule (a range counts as 2)"))?;
    Ok((ports, proto))
}

/// A typed source: blank / `any` / `anywhere` = anywhere; an address;
/// or a network, whose host bits are cleared (`192.168.1.7/24` →
/// `192.168.1.0/24`, as ufw itself would store it). `/32` and `/128`
/// are a single host.
pub fn parse_addr_input(text: &str) -> Result<Addr, String> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("any") || t.eq_ignore_ascii_case("anywhere") {
        return Ok(Addr::Any);
    }
    let bad = || format!("'{t}' is not an IP address or a network like 192.168.1.0/24");
    match t.split_once('/') {
        None => t.parse::<IpAddr>().map(Addr::Host).map_err(|_| bad()),
        Some((ip, prefix)) => {
            let ip = ip.parse::<IpAddr>().map_err(|_| bad())?;
            let prefix: u8 = prefix
                .parse()
                .ok()
                .filter(|p| *p <= max_prefix(ip))
                .ok_or_else(|| {
                    format!(
                        "the prefix after '/' must be 0-{} for this address",
                        max_prefix(ip)
                    )
                })?;
            if prefix == max_prefix(ip) {
                Ok(Addr::Host(ip))
            } else {
                Ok(Addr::Net(masked(ip, prefix), prefix))
            }
        }
    }
}

/// The add-rule form as typed: an incoming rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddForm {
    pub action: Action,
    pub port: String,
    pub proto: Proto,
    pub from: String,
    pub comment: String,
}

impl AddForm {
    /// The form as the add the helper will run, or the first problem.
    /// The same checks [`AddRule::from_fields`] runs as root, plus
    /// typing help: a `/tcp` suffix must agree with the protocol picked.
    pub fn validate(&self) -> Result<AddRule, FormError> {
        let (ports, suffix) = parse_port_input(&self.port).map_err(|m| form_err(Field::Port, m))?;
        let proto = match (suffix, self.proto) {
            (Some(s), Proto::Any) => s,
            (Some(s), p) if s != p => {
                return Err(form_err(
                    Field::Port,
                    format!(
                        "'/{}' disagrees with the {} protocol picked",
                        s.keyword(),
                        p.keyword().to_uppercase()
                    ),
                ));
            }
            (_, p) => p,
        };
        let from = parse_addr_input(&self.from).map_err(|m| form_err(Field::From, m))?;
        let comment = self.comment.trim();
        let comment = if comment.is_empty() {
            None
        } else if let Some(why) = comment_problem(comment) {
            return Err(form_err(Field::Comment, why));
        } else {
            Some(comment.to_string())
        };
        let add = AddRule {
            spec: RuleSpec::incoming(self.action, Some(ports), proto, from),
            comment,
        };
        // What root will check: never offer a prompt root would refuse.
        let fields = add.fields();
        let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        AddRule::from_fields(&refs).map_err(|e| form_err(Field::Port, e.message()))
    }
}

/// A common service the add form can fill in (data, not code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub name: &'static str,
    pub port: &'static str,
    pub proto: Proto,
    pub comment: &'static str,
}

/// The add form's presets. `Any` over a range adds a TCP and a UDP rule.
pub const PRESETS: [Preset; 6] = [
    Preset {
        name: "SSH",
        port: "22",
        proto: Proto::Tcp,
        comment: "SSH",
    },
    Preset {
        name: "HTTP",
        port: "80",
        proto: Proto::Tcp,
        comment: "HTTP",
    },
    Preset {
        name: "HTTPS",
        port: "443",
        proto: Proto::Tcp,
        comment: "HTTPS",
    },
    Preset {
        name: "KDE Connect",
        port: "1714:1764",
        proto: Proto::Any,
        comment: "KDE Connect / Valent",
    },
    Preset {
        name: "LocalSend",
        port: "53317",
        proto: Proto::Any,
        comment: "LocalSend",
    },
    Preset {
        name: "mDNS",
        port: "5353",
        proto: Proto::Udp,
        comment: "mDNS",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_canonical_and_bounded() {
        assert_eq!(Ports::parse("22").unwrap().to_string(), "22");
        assert_eq!(Ports::parse("1714:1764").unwrap().to_string(), "1714:1764");
        assert_eq!(Ports::parse("80,443,8000:8100").unwrap().items().len(), 3);
        for bad in [
            "", "0", "65536", "022", "+22", "-22", " 22", "22 ", "22:22", "30:20", "1:", ":5",
            "80,,443", "80,", "a", "22/tcp", "1e3", "99999999",
        ] {
            assert_eq!(Ports::parse(bad), None, "{bad:?}");
        }
        let fifteen: Vec<String> = (1..=15).map(|p| p.to_string()).collect();
        assert!(Ports::parse(&fifteen.join(",")).is_some());
        let sixteen: Vec<String> = (1..=16).map(|p| p.to_string()).collect();
        assert_eq!(Ports::parse(&sixteen.join(",")), None);
        // a range weighs two
        let eight_ranges: Vec<String> = (1..=8)
            .map(|p| format!("{}:{}", p * 10, p * 10 + 1))
            .collect();
        assert_eq!(Ports::parse(&eight_ranges.join(",")), None);
        assert!(!Ports::parse("22").unwrap().is_multi());
        assert!(Ports::parse("1:2").unwrap().is_multi());
        assert!(Ports::parse("1,2").unwrap().is_multi());
    }

    #[test]
    fn addresses_parse_strictly() {
        assert_eq!(Addr::parse("any"), Some(Addr::Any));
        assert_eq!(Addr::parse("10.1.2.3").unwrap().to_string(), "10.1.2.3");
        assert_eq!(
            Addr::parse("192.168.0.0/24").unwrap().to_string(),
            "192.168.0.0/24"
        );
        assert_eq!(Addr::parse("0.0.0.0/0").unwrap().to_string(), "0.0.0.0/0");
        assert_eq!(Addr::parse("::/0").unwrap().to_string(), "::/0");
        assert_eq!(
            Addr::parse("2001:db8::/32").unwrap().to_string(),
            "2001:db8::/32"
        );
        assert_eq!(Addr::parse("fe80::1").unwrap().is_v6(), Some(true));
        for bad in [
            "",
            "Any",
            "anywhere",
            "192.168.0.5/24",
            "10.0.0.0/33",
            "10.0.0.0/",
            "10.0.0.0/08",
            "10.0.0.0/-1",
            "-10.0.0.1",
            "10.0.0",
            "1.2.3.4.5",
            "10.0.0.1 ",
            "::/129",
            "2001:db8::1/32",
            "host.example",
            "10.0.0.1;id",
            "$(id)",
        ] {
            assert_eq!(Addr::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn interfaces() {
        assert_eq!(iface("eth9").as_deref(), Some("eth9"));
        assert_eq!(iface("wlp0s20f3").as_deref(), Some("wlp0s20f3"));
        assert_eq!(iface("eth+").as_deref(), Some("eth+"));
        assert_eq!(iface("br-1a2b").as_deref(), Some("br-1a2b"));
        for bad in [
            "",
            "-",
            "-eth0",
            "+",
            "eth0 x",
            "eth0;id",
            "averyveryverylong0",
            "eth/0",
            "eth0++",
        ] {
            assert_eq!(iface(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn comments_refuse_what_ufw_or_a_shell_would_misread() {
        for good in [
            "SSH",
            "KDE Connect / Valent",
            "Yazıcı ağ",
            "my web: 8080/tcp #1 (dev) a=b, c+d @x!",
            "from",
            &"a".repeat(64),
        ] {
            assert_eq!(comment_problem(good), None, "{good:?}");
        }
        for bad in [
            "",
            "in",
            "IN",
            "out",
            "Log",
            "log-all",
            "-x",
            " lead",
            "trail ",
            "a'b",
            "a\"b",
            "a\\b",
            "a`b",
            "a$b",
            "a;b",
            "a|b",
            "a<b",
            "a>b",
            "tab\tx",
            "new\nline",
            &"a".repeat(65),
        ] {
            assert!(comment_problem(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn rule_fields_round_trip() {
        let spec = RuleSpec {
            action: Action::Reject,
            direction: Direction::Out,
            iface: Some("eth9".into()),
            log: Some(Log::LogAll),
            proto: Proto::Udp,
            from: Addr::parse("2001:db8::/32").unwrap(),
            to: Addr::Any,
            ports: Ports::parse("53,5353"),
        };
        let f = spec.fields();
        assert_eq!(
            f,
            [
                "reject",
                "out",
                "eth9",
                "log-all",
                "udp",
                "2001:db8::/32",
                "any",
                "53,5353"
            ]
        );
        let refs: Vec<&str> = f.iter().map(String::as_str).collect();
        assert_eq!(RuleSpec::from_fields(&refs).unwrap(), spec);
        let bare = RuleSpec::incoming(
            Action::Allow,
            None,
            Proto::Any,
            Addr::parse("10.0.0.1").unwrap(),
        );
        assert_eq!(
            bare.fields(),
            ["allow", "in", "-", "-", "any", "10.0.0.1", "any", "-"]
        );
    }

    #[test]
    fn cross_field_checks() {
        let range = Ports::parse("1714:1764");
        let mut s = RuleSpec::incoming(Action::Allow, range, Proto::Any, Addr::Any);
        assert!(s.check(true).is_ok(), "an add expands any into tcp+udp");
        assert!(s.check(false).is_err(), "a delete names the one protocol");
        s.proto = Proto::Tcp;
        assert!(s.check(false).is_ok());
        let all = RuleSpec::incoming(Action::Allow, None, Proto::Any, Addr::Any);
        assert!(
            all.check(true).is_err(),
            "never add an allow-everything rule"
        );
        assert!(all.check(false).is_ok(), "but one that exists may go");
        let mixed = RuleSpec {
            to: Addr::parse("10.0.0.0/8").unwrap(),
            ..RuleSpec::incoming(
                Action::Allow,
                Ports::parse("22"),
                Proto::Tcp,
                Addr::parse("::1").unwrap(),
            )
        };
        assert!(mixed.check(true).is_err());
    }

    #[test]
    fn add_fields_are_validated_as_root_would() {
        let ok = ["allow", "in", "-", "-", "tcp", "any", "any", "22", "SSH"];
        let add = AddRule::from_fields(&ok).unwrap();
        assert_eq!(add.comment.as_deref(), Some("SSH"));
        assert_eq!(add.fields(), ok);
        let mut no_comment = ok;
        no_comment[8] = "-";
        assert_eq!(AddRule::from_fields(&no_comment).unwrap().comment, None);
        for (i, bad) in [
            (0, "accept"),
            (1, "both"),
            (2, "-eth0"),
            (3, "verbose"),
            (4, "icmp"),
            (5, "192.168.0.5/24"),
            (6, "anywhere"),
            (7, "0"),
            (7, "--dry-run"),
            (8, "in"),
            (8, "a;b"),
            (8, ""),
        ] {
            let mut f = ok;
            f[i] = bad;
            assert_eq!(
                AddRule::from_fields(&f).unwrap_err().code(),
                2,
                "{i} {bad:?}"
            );
        }
        assert_eq!(AddRule::from_fields(&ok[..8]).unwrap_err().code(), 2);
    }

    #[test]
    fn typed_ports_are_forgiven_but_checked() {
        let p = |s| parse_port_input(s).map(|(p, proto)| (p.to_string(), proto));
        assert_eq!(p("22"), Ok(("22".into(), None)));
        assert_eq!(p(" 0022 "), Ok(("22".into(), None)));
        assert_eq!(p("1714-1764"), Ok(("1714:1764".into(), None)));
        assert_eq!(p("80, 443"), Ok(("80,443".into(), None)));
        assert_eq!(p("22/tcp"), Ok(("22".into(), Some(Proto::Tcp))));
        assert_eq!(p("53/udp"), Ok(("53".into(), Some(Proto::Udp))));
        for bad in [
            "", "0", "65536", "ssh", "22/icmp", "30:20", "22:22", "-5", "1,,2",
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn typed_addresses() {
        let a = |s| parse_addr_input(s).map(|a| a.to_string());
        assert_eq!(a(""), Ok("any".into()));
        assert_eq!(a(" Anywhere "), Ok("any".into()));
        assert_eq!(a("192.168.1.7/24"), Ok("192.168.1.0/24".into()));
        assert_eq!(a("10.0.0.1/32"), Ok("10.0.0.1".into()));
        assert_eq!(a("2001:db8::5/64"), Ok("2001:db8::/64".into()));
        assert_eq!(a("fe80::1"), Ok("fe80::1".into()));
        for bad in ["lan", "10.0.0.1/33", "10.0.0.1/x", "-1.2.3.4", "1.2.3"] {
            assert!(a(bad).is_err(), "{bad:?}");
        }
    }

    fn form(port: &str, proto: Proto, from: &str, comment: &str) -> AddForm {
        AddForm {
            action: Action::Allow,
            port: port.into(),
            proto,
            from: from.into(),
            comment: comment.into(),
        }
    }

    #[test]
    fn the_form_names_the_field_at_fault() {
        let add = form("22", Proto::Tcp, "", " SSH ").validate().unwrap();
        assert_eq!(add.spec.port_label(), "22/tcp");
        assert_eq!(add.comment.as_deref(), Some("SSH"));
        let add = form("22/udp", Proto::Any, "", "").validate().unwrap();
        assert_eq!(add.spec.proto, Proto::Udp);
        assert_eq!(
            form("22/udp", Proto::Tcp, "", "")
                .validate()
                .unwrap_err()
                .field,
            Field::Port
        );
        assert_eq!(
            form("x", Proto::Tcp, "", "").validate().unwrap_err().field,
            Field::Port
        );
        assert_eq!(
            form("22", Proto::Tcp, "lan", "")
                .validate()
                .unwrap_err()
                .field,
            Field::From
        );
        assert_eq!(
            form("22", Proto::Tcp, "", "in")
                .validate()
                .unwrap_err()
                .field,
            Field::Comment
        );
        // a range with "both" is fine: it becomes two rules
        assert!(
            form("1714:1764", Proto::Any, "192.168.0.0/24", "")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn every_preset_validates() {
        for p in PRESETS {
            let f = form(p.port, p.proto, "", p.comment);
            assert!(f.validate().is_ok(), "{}", p.name);
        }
    }
}
