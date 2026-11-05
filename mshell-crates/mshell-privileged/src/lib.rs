//! `mshell-privileged`: the one root helper mshell runs through `pkexec`.
//!
//! Every privileged action the shell offers (pacnew review, rollback,
//! the background upgrade, signalling another user's process, a SMART
//! read, a ThinkPad fan level, the battery charge limit, the logind
//! power-key/lid drop-in) is one fixed subcommand of this binary,
//! with its own polkit action (`org.margo.mshell.privileged.<sub>`,
//! matched on `exec.path` + `exec.argv1`), so the password dialog says
//! exactly what root is about to do instead of "run /bin/sh". There is
//! no shell and no free-form write: each argument is re-validated here,
//! as root ([`validate`]), and the file-system side refuses symlinks
//! and walks directories with `O_NOFOLLOW` ([`fsops`]).
//!
//! The shell builds its `pkexec` argv from [`Request`], so both sides
//! share one definition of the vocabulary.

pub mod fsops;
pub mod procsig;
pub mod run;
pub mod validate;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
pub use validate::{FanLevel, Signal};

/// Where the packages install the helper. Nix builds point it at the
/// store path with `MSHELL_PRIVILEGED_HELPER` at compile time; the
/// polkit policy's `exec.path` must name the same file.
pub const HELPER: &str = match option_env!("MSHELL_PRIVILEGED_HELPER") {
    Some(p) => p,
    None => "/usr/lib/mshell/mshell-privileged",
};

/// The prefix of every action id in `org.margo.mshell.privileged.policy`.
pub const ACTION_PREFIX: &str = "org.margo.mshell.privileged.";

/// Shown when the helper isn't installed (pkexec would report 127,
/// which reads as "not authorized").
pub const MISSING: &str =
    "The mshell-privileged helper isn't installed (/usr/lib/mshell); reinstall margo";

/// Whether [`HELPER`] is there to run.
pub fn installed() -> bool {
    std::path::Path::new(HELPER).is_file()
}

/// Every subcommand, in policy order.
pub const SUBCOMMANDS: [&str; 10] = [
    "pacnew-install",
    "pacnew-merge",
    "pacnew-remove",
    "kill",
    "smart-read",
    "fan-set",
    "charge-limit",
    "logind-dropin",
    "upgrade",
    "rollback",
];

/// Why the helper refused or failed. The exit code tells the caller
/// which (never 126/127: those are pkexec's own "dismissed" / "not
/// authorized").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Unknown subcommand / wrong argument count.
    Usage(String),
    /// An argument failed validation.
    Invalid(String),
    /// Valid, but the target is unsafe or gone (symlink, wrong file
    /// type, a reused pid).
    Refused(String),
    /// The operation itself failed.
    Io(String),
}

impl Error {
    pub fn code(&self) -> u8 {
        match self {
            Error::Io(_) => 1,
            Error::Usage(_) | Error::Invalid(_) => 2,
            Error::Refused(_) => 3,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Error::Usage(m) | Error::Invalid(m) | Error::Refused(m) | Error::Io(m) => m,
        }
    }
}

/// One privileged action, as the shell asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Replace `live` with the `review` file's content (backup kept
    /// as `<live>.mshell-bak`, mode/owner kept), then remove `review`.
    PacnewInstall { live: PathBuf, review: PathBuf },
    /// The same with a merge the user resolved, read from stdin.
    PacnewMerge { live: PathBuf, review: PathBuf },
    /// Delete a `.pacnew` / `.pacsave` / `.pacorig`.
    PacnewRemove { review: PathBuf },
    /// Signal `pid` only while it is still the process started at `start`.
    Kill {
        pid: i32,
        start: u64,
        signal: Signal,
    },
    /// `smartctl -j -a` per device, JSON on stdout.
    SmartRead { devices: Vec<String> },
    /// Write `level <level>` to `/proc/acpi/ibm/fan`.
    FanSet { level: FanLevel },
    /// Write the charge limit (end threshold, %) of every battery that
    /// has one under `/sys/class/power_supply` (found by the helper
    /// itself), and the start threshold to 5 below it where present.
    ChargeLimit { limit: u8 },
    /// Write [`validate::LOGIND_DROPIN`] from a body on stdin that
    /// passes [`validate::logind_dropin`] (the power key and lid keys
    /// Settings manages, nothing else).
    LogindDropin,
    /// `pacman -Syu --noconfirm` (official repositories only).
    Upgrade,
    /// `pacman -U --noconfirm <cached package>`.
    Rollback { package: PathBuf },
}

impl Request {
    pub fn subcommand(&self) -> &'static str {
        match self {
            Request::PacnewInstall { .. } => "pacnew-install",
            Request::PacnewMerge { .. } => "pacnew-merge",
            Request::PacnewRemove { .. } => "pacnew-remove",
            Request::Kill { .. } => "kill",
            Request::SmartRead { .. } => "smart-read",
            Request::FanSet { .. } => "fan-set",
            Request::ChargeLimit { .. } => "charge-limit",
            Request::LogindDropin => "logind-dropin",
            Request::Upgrade => "upgrade",
            Request::Rollback { .. } => "rollback",
        }
    }

    /// The polkit action this request raises.
    pub fn action_id(&self) -> String {
        format!("{ACTION_PREFIX}{}", self.subcommand())
    }

    fn args(&self) -> Vec<OsString> {
        let p = |p: &PathBuf| p.clone().into_os_string();
        match self {
            Request::PacnewInstall { live, review } | Request::PacnewMerge { live, review } => {
                vec![p(live), p(review)]
            }
            Request::PacnewRemove { review } => vec![p(review)],
            Request::Kill { pid, start, signal } => vec![
                pid.to_string().into(),
                start.to_string().into(),
                signal.name().into(),
            ],
            Request::SmartRead { devices } => devices.iter().map(OsString::from).collect(),
            Request::FanSet { level } => vec![level.keyword().into()],
            Request::ChargeLimit { limit } => vec![limit.to_string().into()],
            Request::LogindDropin | Request::Upgrade => Vec::new(),
            Request::Rollback { package } => vec![p(package)],
        }
    }

    /// The argv after `pkexec`: `[HELPER, subcommand, args…]`.
    pub fn argv(&self) -> Vec<OsString> {
        let mut v = vec![OsString::from(HELPER), self.subcommand().into()];
        v.extend(self.args());
        v
    }

    /// Parse and validate `[subcommand, args…]` (the helper's argv
    /// after its own name). The shell runs this on its own request
    /// too, so an argument root would refuse never raises a prompt.
    pub fn parse(argv: &[OsString]) -> Result<Request, Error> {
        let (sub, rest) = argv.split_first().ok_or_else(|| {
            Error::Usage(format!(
                "usage: mshell-privileged <{}> …",
                SUBCOMMANDS.join("|")
            ))
        })?;
        let arity = |n: usize| -> Result<(), Error> {
            if rest.len() == n {
                Ok(())
            } else {
                Err(Error::Usage(format!(
                    "{} takes {n} argument(s), got {}",
                    sub.to_string_lossy(),
                    rest.len()
                )))
            }
        };
        let a = |i: usize| rest[i].as_os_str();
        let sub_b: &OsStr = sub;
        Ok(match sub_b.to_str().unwrap_or("") {
            "pacnew-install" => {
                arity(2)?;
                let (live, review) = validate::pacnew_pair(a(0), a(1))?;
                Request::PacnewInstall { live, review }
            }
            "pacnew-merge" => {
                arity(2)?;
                let (live, review) = validate::pacnew_pair(a(0), a(1))?;
                Request::PacnewMerge { live, review }
            }
            "pacnew-remove" => {
                arity(1)?;
                Request::PacnewRemove {
                    review: validate::review_path(a(0))?,
                }
            }
            "kill" => {
                arity(3)?;
                Request::Kill {
                    pid: validate::pid(a(0))?,
                    start: validate::start_time(a(1))?,
                    signal: Signal::parse(a(2))?,
                }
            }
            "smart-read" => {
                if rest.is_empty() || rest.len() > validate::MAX_DEVICES {
                    return Err(Error::Usage(format!(
                        "smart-read takes 1 to {} devices",
                        validate::MAX_DEVICES
                    )));
                }
                let devices = rest
                    .iter()
                    .map(|d| validate::device(d))
                    .collect::<Result<_, _>>()?;
                Request::SmartRead { devices }
            }
            "fan-set" => {
                arity(1)?;
                Request::FanSet {
                    level: FanLevel::parse(a(0))?,
                }
            }
            "charge-limit" => {
                arity(1)?;
                Request::ChargeLimit {
                    limit: validate::charge_limit(a(0))?,
                }
            }
            "logind-dropin" => {
                arity(0)?;
                Request::LogindDropin
            }
            "upgrade" => {
                arity(0)?;
                Request::Upgrade
            }
            "rollback" => {
                arity(1)?;
                Request::Rollback {
                    package: validate::package_file(a(0))?,
                }
            }
            _ => {
                return Err(Error::Usage(format!(
                    "unknown subcommand {:?} (one of: {})",
                    sub.to_string_lossy(),
                    SUBCOMMANDS.join(", ")
                )));
            }
        })
    }

    /// The request as one `pkexec …` command for a shell step (the
    /// background runner's `sh`): every word single-quoted, so nothing
    /// in it is shell syntax.
    pub fn pkexec_command(&self) -> String {
        let mut words = vec!["pkexec".to_string()];
        words.extend(
            self.argv()
                .iter()
                .map(|w| format!("'{}'", w.to_string_lossy().replace('\'', "'\\''"))),
        );
        words.join(" ")
    }

    /// [`Request::parse`] on this request's own argv: what the root
    /// side will decide, checked before the prompt.
    pub fn validated(self) -> Result<Request, Error> {
        let argv = self.argv();
        Request::parse(&argv[1..])
    }
}

/// `argv` as strings for a log line / `Command::args`.
pub fn lossy(argv: &[OsString]) -> Vec<String> {
    argv.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(r: Request) {
        let argv = r.argv();
        assert_eq!(argv[0], OsString::from(HELPER));
        assert_eq!(argv[1], OsString::from(r.subcommand()));
        assert_eq!(Request::parse(&argv[1..]), Ok(r));
    }

    #[test]
    fn every_request_round_trips_through_its_argv() {
        round_trip(Request::PacnewInstall {
            live: "/etc/locale.gen".into(),
            review: "/etc/locale.gen.pacnew".into(),
        });
        round_trip(Request::PacnewMerge {
            live: "/etc/a".into(),
            review: "/etc/a.pacsave".into(),
        });
        round_trip(Request::PacnewRemove {
            review: "/etc/a.pacorig".into(),
        });
        round_trip(Request::Kill {
            pid: 4242,
            start: 99,
            signal: Signal::Kill,
        });
        round_trip(Request::SmartRead {
            devices: vec!["/dev/nvme0".into(), "/dev/sda".into()],
        });
        round_trip(Request::FanSet {
            level: FanLevel::FullSpeed,
        });
        round_trip(Request::ChargeLimit { limit: 80 });
        round_trip(Request::LogindDropin);
        round_trip(Request::Upgrade);
        round_trip(Request::Rollback {
            package: "/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst".into(),
        });
    }

    #[test]
    fn the_argv_is_fixed_and_carries_no_shell() {
        let argv = lossy(
            &Request::Kill {
                pid: 7,
                start: 1,
                signal: Signal::Term,
            }
            .argv(),
        );
        assert_eq!(argv, [HELPER, "kill", "7", "1", "TERM"]);
        assert!(!argv.iter().any(|a| a == "-c" || a.ends_with("/sh")));
    }

    #[test]
    fn the_shell_step_quotes_every_word() {
        let cmd = Request::Rollback {
            package: "/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst".into(),
        }
        .pkexec_command();
        assert_eq!(
            cmd,
            format!("pkexec '{HELPER}' 'rollback' '/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst'")
        );
        let out = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "set -- {}; printf '%s|' \"$@\"",
                    Request::Upgrade.pkexec_command()
                ),
            ])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("pkexec|{HELPER}|upgrade|")
        );
    }

    #[test]
    fn usage_errors() {
        let p = |v: &[&str]| Request::parse(&v.iter().map(OsString::from).collect::<Vec<_>>());
        assert_eq!(p(&[]).unwrap_err().code(), 2);
        assert_eq!(p(&["sh", "-c", "id"]).unwrap_err().code(), 2);
        assert_eq!(p(&["upgrade", "--overwrite=*"]).unwrap_err().code(), 2);
        assert_eq!(p(&["kill", "5", "1"]).unwrap_err().code(), 2);
        assert_eq!(p(&["smart-read"]).unwrap_err().code(), 2);
        assert_eq!(p(&["charge-limit", "101"]).unwrap_err().code(), 2);
        assert!(Request::ChargeLimit { limit: 0 }.validated().is_err());
        let many: Vec<String> = (0..17).map(|i| format!("/dev/sd{i}")).collect();
        let mut v = vec!["smart-read"];
        v.extend(many.iter().map(String::as_str));
        assert_eq!(p(&v).unwrap_err().code(), 2);
        assert_eq!(p(&["pacnew-remove", "/etc/passwd"]).unwrap_err().code(), 2);
        assert!(
            Request::PacnewRemove {
                review: "/etc/../x.pacnew".into()
            }
            .validated()
            .is_err()
        );
    }

    #[test]
    fn no_exit_code_collides_with_pkexec() {
        for e in [
            Error::Usage(String::new()),
            Error::Invalid(String::new()),
            Error::Refused(String::new()),
            Error::Io(String::new()),
        ] {
            assert!(!matches!(e.code(), 0 | 126 | 127));
        }
    }
}
