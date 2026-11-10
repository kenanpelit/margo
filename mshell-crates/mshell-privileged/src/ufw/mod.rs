//! UFW: the firewall verbs of the helper and the unprivileged readers
//! the shell (bar pill, menu, Control Center tile) and `mshellctl ufw`
//! share.
//!
//! * [`spec`]: the rule fields and their validators (strict for root,
//!   lenient for the add form, one definition).
//! * [`argv`]: the fixed `ufw` / `systemctl` argv built from them.
//! * [`rules`]: `ufw.conf`, `/etc/default/ufw` and the rule files.
//! * [`state`]: the unit's state and what it all means (green / red /
//!   amber / muted, and the fix to offer).
//!
//! Root runs exactly five subcommands ([`UfwOp`]), each its own polkit
//! action: `ufw-status` (read the rule files), `ufw-switch on|off|reload`,
//! `ufw-add <rule> <comment>`, `ufw-delete <rule>` and
//! `ufw-unit start|stop|enable|disable` (`ufw.service` only).

pub mod argv;
pub mod rules;
pub mod spec;
pub mod state;

pub use argv::{Switch, UnitOp};
pub use spec::{AddRule, RuleSpec};

use crate::Error;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;

/// One firewall action, as the shell asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UfwOp {
    /// Print the rule files' rule lines and settings ([`rules::dump`]).
    Status,
    Switch(Switch),
    Add(AddRule),
    Delete(RuleSpec),
    Unit(UnitOp),
}

/// The subcommands, in policy order.
pub const SUBCOMMANDS: [&str; 5] = [
    "ufw-status",
    "ufw-switch",
    "ufw-add",
    "ufw-delete",
    "ufw-unit",
];

impl UfwOp {
    pub fn subcommand(&self) -> &'static str {
        match self {
            UfwOp::Status => "ufw-status",
            UfwOp::Switch(_) => "ufw-switch",
            UfwOp::Add(_) => "ufw-add",
            UfwOp::Delete(_) => "ufw-delete",
            UfwOp::Unit(_) => "ufw-unit",
        }
    }

    pub fn args(&self) -> Vec<OsString> {
        let v: Vec<String> = match self {
            UfwOp::Status => Vec::new(),
            UfwOp::Switch(s) => vec![s.keyword().into()],
            UfwOp::Add(a) => a.fields(),
            UfwOp::Delete(s) => s.fields(),
            UfwOp::Unit(u) => vec![u.keyword().into()],
        };
        v.into_iter().map(OsString::from).collect()
    }

    /// Parse and validate a `ufw-*` subcommand's arguments; `None` when
    /// `sub` isn't one.
    pub fn parse(sub: &str, rest: &[OsString]) -> Option<Result<UfwOp, Error>> {
        if !SUBCOMMANDS.contains(&sub) {
            return None;
        }
        let strs: Option<Vec<&str>> = rest.iter().map(|a| a.to_str()).collect();
        let Some(a) = strs else {
            return Some(Err(Error::Invalid(format!(
                "{sub}: arguments must be UTF-8"
            ))));
        };
        let arity = |n: usize| {
            if a.len() == n {
                Ok(())
            } else {
                Err(Error::Usage(format!(
                    "{sub} takes {n} argument(s), got {}",
                    a.len()
                )))
            }
        };
        let keyword =
            |what: &str| Error::Invalid(format!("invalid {what}: {}", a.first().unwrap_or(&"")));
        Some((|| -> Result<UfwOp, Error> {
            Ok(match sub {
                "ufw-status" => {
                    arity(0)?;
                    UfwOp::Status
                }
                "ufw-switch" => {
                    arity(1)?;
                    UfwOp::Switch(
                        Switch::parse(a[0]).ok_or_else(|| keyword("switch (on, off or reload)"))?,
                    )
                }
                "ufw-unit" => {
                    arity(1)?;
                    UfwOp::Unit(
                        UnitOp::parse(a[0]).ok_or_else(|| {
                            keyword("unit action (start, stop, enable or disable)")
                        })?,
                    )
                }
                "ufw-add" => {
                    arity(9)?;
                    UfwOp::Add(AddRule::from_fields(&a)?)
                }
                _ => {
                    arity(8)?;
                    let spec = RuleSpec::from_fields(&a)?;
                    spec.check(false)?;
                    UfwOp::Delete(spec)
                }
            })
        })())
    }
}

/// Read one file of at most [`rules::MAX_FILE`] bytes.
pub fn read_text(path: &Path) -> Result<String, state::ReadError> {
    let f = std::fs::File::open(path).map_err(|e| state::ReadError::from_io(&e))?;
    let mut buf = Vec::new();
    f.take(rules::MAX_FILE)
        .read_to_end(&mut buf)
        .map_err(|e| state::ReadError::from_io(&e))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The saved state of the four files, read below `root` (`/` for the
/// real ones; a fixture tree in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub conf: Result<rules::Conf, state::ReadError>,
    pub defaults: Option<rules::Defaults>,
    pub rules: Result<rules::RuleSet, state::ReadError>,
}

fn under(root: &Path, logical: &str) -> std::path::PathBuf {
    root.join(logical.trim_start_matches('/'))
}

/// Read and parse the four files without root. A missing `user6.rules`
/// leaves the set incomplete (with IPv6 on); an unreadable `user.rules`
/// makes the rules an error the caller can fix with `ufw-status`.
pub fn read_saved(root: &Path) -> Saved {
    let conf = read_text(&under(root, rules::UFW_CONF)).map(|t| rules::parse_conf(&t));
    let defaults = read_text(&under(root, rules::DEFAULTS))
        .ok()
        .map(|t| rules::parse_defaults(&t));
    let ipv6 = defaults.as_ref().is_none_or(|d| d.ipv6);
    let rules = read_text(&under(root, rules::USER_RULES)).map(|v4| {
        let v6 = ipv6
            .then(|| read_text(&under(root, rules::USER6_RULES)).ok())
            .flatten();
        rules::parse_rules(&v4, v6.as_deref(), ipv6)
    });
    Saved {
        conf,
        defaults,
        rules,
    }
}

/// The same from a root `ufw-status` dump.
pub fn saved_from_dump(d: &rules::Dump) -> Saved {
    let conf = d
        .conf
        .as_deref()
        .map(rules::parse_conf)
        .ok_or(state::ReadError::Missing);
    let defaults = d.defaults.as_deref().map(rules::parse_defaults);
    let ipv6 = defaults.as_ref().is_none_or(|x| x.ipv6);
    let rules =
        d.v4.as_deref()
            .map(|v4| rules::parse_rules(v4, d.v6.as_deref(), ipv6))
            .ok_or(state::ReadError::Missing);
    Saved {
        conf,
        defaults,
        rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn every_op_round_trips() {
        let ops = [
            UfwOp::Status,
            UfwOp::Switch(Switch::On),
            UfwOp::Switch(Switch::Off),
            UfwOp::Switch(Switch::Reload),
            UfwOp::Unit(UnitOp::Start),
            UfwOp::Unit(UnitOp::Disable),
            UfwOp::Add(
                spec::AddForm {
                    action: spec::Action::Allow,
                    port: "1714:1764".into(),
                    proto: spec::Proto::Any,
                    from: "192.168.0.0/24".into(),
                    comment: "KDE Connect".into(),
                }
                .validate()
                .unwrap(),
            ),
            UfwOp::Delete(
                RuleSpec::from_fields(&["deny", "out", "-", "-", "udp", "any", "any", "53"])
                    .unwrap(),
            ),
        ];
        for op in ops {
            let back = UfwOp::parse(op.subcommand(), &op.args()).unwrap();
            assert_eq!(back, Ok(op));
        }
    }

    #[test]
    fn refusals() {
        let p = |sub: &str, a: &[&str]| UfwOp::parse(sub, &os(a)).unwrap().unwrap_err().code();
        assert!(UfwOp::parse("ufw", &[]).is_none());
        assert_eq!(p("ufw-status", &["x"]), 2);
        assert_eq!(p("ufw-switch", &["reset"]), 2);
        assert_eq!(p("ufw-switch", &[]), 2);
        assert_eq!(p("ufw-unit", &["mask"]), 2);
        assert_eq!(p("ufw-unit", &["start", "sshd"]), 2);
        assert_eq!(
            p(
                "ufw-delete",
                &["allow", "in", "-", "-", "any", "any", "any", "1:2"]
            ),
            2
        );
        assert_eq!(
            p(
                "ufw-add",
                &["allow", "in", "-", "-", "any", "any", "any", "-", "-"]
            ),
            2
        );
        assert_eq!(
            p(
                "ufw-add",
                &["allow", "in", "-", "-", "tcp", "any", "any", "22", "a;b"]
            ),
            2
        );
        assert_eq!(
            p(
                "ufw-add",
                &["allow", "in", "-", "-", "tcp", "any", "any", "22"]
            ),
            2
        );
        let bad = UfwOp::parse(
            "ufw-switch",
            &[std::os::unix::ffi::OsStringExt::from_vec(vec![0xff])],
        );
        assert_eq!(bad.unwrap().unwrap_err().code(), 2);
    }

    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (logical, text) in files {
            let p = under(dir.path(), logical);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    const V4: &str = include_str!("../../tests/data/ufw/user.rules");
    const V6: &str = include_str!("../../tests/data/ufw/user6.rules");
    const CONF: &str = include_str!("../../tests/data/ufw/ufw.conf");
    const DEFAULTS: &str = include_str!("../../tests/data/ufw/default-ufw");

    #[test]
    fn the_saved_state_reads_from_a_tree() {
        let dir = fixture(&[
            (rules::UFW_CONF, CONF),
            (rules::DEFAULTS, DEFAULTS),
            (rules::USER_RULES, V4),
            (rules::USER6_RULES, V6),
        ]);
        let s = read_saved(dir.path());
        assert_eq!(s.conf.unwrap().enabled, Some(true));
        assert_eq!(s.defaults.unwrap().input.as_deref(), Some("DROP"));
        let set = s.rules.unwrap();
        assert!(set.complete);
        assert_eq!(set.allowed_in(), 9);

        let empty = tempfile::tempdir().unwrap();
        let s = read_saved(empty.path());
        assert_eq!(s.conf, Err(state::ReadError::Missing));
        assert_eq!(s.rules, Err(state::ReadError::Missing));
    }

    #[test]
    fn an_unreadable_rule_file_is_denied() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: getter.
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads anything
        }
        let dir = fixture(&[(rules::UFW_CONF, CONF), (rules::USER_RULES, V4)]);
        let p = under(dir.path(), rules::USER_RULES);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(read_saved(dir.path()).rules, Err(state::ReadError::Denied));
    }

    #[test]
    fn a_dump_gives_the_same_state() {
        let text = rules::dump(Some(CONF), Some(DEFAULTS), Some(V4), Some(V6));
        let d = rules::parse_dump(&text).unwrap();
        let from_dump = saved_from_dump(&d);
        let dir = fixture(&[
            (rules::UFW_CONF, CONF),
            (rules::DEFAULTS, DEFAULTS),
            (rules::USER_RULES, V4),
            (rules::USER6_RULES, V6),
        ]);
        let from_files = read_saved(dir.path());
        assert_eq!(from_dump.conf, from_files.conf);
        assert_eq!(from_dump.rules, from_files.rules);
        assert_eq!(
            from_dump.defaults.map(|d| d.input),
            from_files.defaults.map(|d| d.input)
        );
    }
}
