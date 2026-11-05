//! Argument validators for the privilege boundary. Pure string checks:
//! the root side runs every one of them on its own argv before it opens
//! anything, and the shell runs the same ones before it raises a polkit
//! prompt. The file-system checks (symlinks, file types) come after, in
//! [`crate::fsops`], because only the root side can do them race-free.

use crate::Error;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

/// The only directory the pacnew subcommands write under.
pub const ETC: &str = "/etc";
/// The only directory a rollback installs packages from.
pub const PKG_CACHE: &str = "/var/cache/pacman/pkg";
/// The suffixes pacman gives the files the review panel handles.
pub const REVIEW_SUFFIXES: [&str; 3] = [".pacnew", ".pacsave", ".pacorig"];
/// Linux `PATH_MAX` / `NAME_MAX`.
const PATH_MAX: usize = 4096;
const NAME_MAX: usize = 255;

fn invalid(what: &str, arg: &OsStr) -> Error {
    Error::Invalid(format!("invalid {what}: {}", arg.to_string_lossy()))
}

/// A plain absolute path strictly below `dir`: no NUL, no empty, `.` or
/// `..` component, no trailing slash, every name within `NAME_MAX`.
/// Returns the components below `dir`.
pub fn below<'a>(dir: &str, arg: &'a OsStr) -> Option<Vec<&'a [u8]>> {
    let b = arg.as_bytes();
    if b.is_empty() || b.len() > PATH_MAX || b.contains(&0) {
        return None;
    }
    let rest = b.strip_prefix(dir.as_bytes())?.strip_prefix(b"/")?;
    let parts: Vec<&[u8]> = rest.split(|c| *c == b'/').collect();
    let ok = parts
        .iter()
        .all(|p| !p.is_empty() && *p != b"." && *p != b".." && p.len() <= NAME_MAX);
    ok.then_some(parts)
}

/// A path below `/etc` (see [`below`]).
pub fn etc_path(arg: &OsStr) -> Result<PathBuf, Error> {
    below(ETC, arg)
        .map(|_| PathBuf::from(arg))
        .ok_or_else(|| invalid("path (must be a plain path below /etc)", arg))
}

/// A `.pacnew` / `.pacsave` / `.pacorig` file below `/etc` whose live
/// file is also below `/etc` (so `/etc/.pacnew` is refused).
pub fn review_path(arg: &OsStr) -> Result<PathBuf, Error> {
    let review = etc_path(arg)?;
    let b = arg.as_bytes();
    let live = REVIEW_SUFFIXES
        .iter()
        .find_map(|s| b.strip_suffix(s.as_bytes()))
        .ok_or_else(|| invalid("review file (not a .pacnew/.pacsave/.pacorig)", arg))?;
    if live.ends_with(b"/") || below(ETC, OsStr::from_bytes(live)).is_none() {
        return Err(invalid("review file (no live file name)", arg));
    }
    Ok(review)
}

/// The `(live, review)` pair of a pacnew write: `review` is exactly
/// `live` plus one of [`REVIEW_SUFFIXES`], both below `/etc`.
pub fn pacnew_pair(live: &OsStr, review: &OsStr) -> Result<(PathBuf, PathBuf), Error> {
    let live_p = etc_path(live)?;
    let review_p = review_path(review)?;
    let paired = REVIEW_SUFFIXES
        .iter()
        .any(|s| review.as_bytes().strip_suffix(s.as_bytes()) == Some(live.as_bytes()));
    if !paired {
        return Err(Error::Invalid(format!(
            "{} is not the review file of {}",
            review.to_string_lossy(),
            live.to_string_lossy()
        )));
    }
    Ok((live_p, review_p))
}

/// Decimal digits only: no sign, no space, no leading zero (one `0` is
/// fine), at most 20 digits.
fn digits(s: &[u8]) -> bool {
    !s.is_empty()
        && s.len() <= 20
        && s.iter().all(u8::is_ascii_digit)
        && (s.len() == 1 || s[0] != b'0')
}

/// A pid the helper may signal: decimal, above 1 (never init or the
/// whole-group/broadcast forms 0 and -1), within `i32`.
pub fn pid(arg: &OsStr) -> Result<i32, Error> {
    let b = arg.as_bytes();
    if !digits(b) {
        return Err(invalid("pid", arg));
    }
    match std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
    {
        Some(p) if p > 1 => Ok(p),
        _ => Err(invalid("pid (init and pid 0/1 are never signalled)", arg)),
    }
}

/// A process start time (`/proc/<pid>/stat` field 22, clock ticks).
pub fn start_time(arg: &OsStr) -> Result<u64, Error> {
    let b = arg.as_bytes();
    digits(b)
        .then(|| std::str::from_utf8(b).ok()?.parse::<u64>().ok())
        .flatten()
        .ok_or_else(|| invalid("start time", arg))
}

/// The signals the process tables send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Kill => "KILL",
        }
    }

    pub fn number(self) -> i32 {
        match self {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        }
    }

    /// `TERM` / `KILL` (case-sensitive; nothing else).
    pub fn parse(arg: &OsStr) -> Result<Signal, Error> {
        match arg.as_bytes() {
            b"TERM" => Ok(Signal::Term),
            b"KILL" => Ok(Signal::Kill),
            _ => Err(invalid("signal (TERM or KILL)", arg)),
        }
    }
}

/// A drive `smartctl --scan` lists: `/dev/<1–32 ASCII letters/digits>`.
pub fn device(arg: &OsStr) -> Result<String, Error> {
    let b = arg.as_bytes();
    match b.strip_prefix(b"/dev/") {
        Some(n) if !n.is_empty() && n.len() <= 32 && n.iter().all(u8::is_ascii_alphanumeric) => {
            Ok(String::from_utf8_lossy(b).into_owned())
        }
        _ => Err(invalid("device (must be /dev/<letters and digits>)", arg)),
    }
}

/// At most this many drives per SMART read.
pub const MAX_DEVICES: usize = 16;

/// A ThinkPad fan level `thinkpad_acpi` takes after `level `.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanLevel {
    Auto,
    Level(u8),
    FullSpeed,
}

impl FanLevel {
    pub fn keyword(self) -> String {
        match self {
            FanLevel::Auto => "auto".to_string(),
            FanLevel::Level(n) => n.min(7).to_string(),
            FanLevel::FullSpeed => "full-speed".to_string(),
        }
    }

    /// `auto`, `full-speed` or one digit `0`–`7`; nothing else.
    pub fn parse(arg: &OsStr) -> Result<FanLevel, Error> {
        match arg.as_bytes() {
            b"auto" => Ok(FanLevel::Auto),
            b"full-speed" => Ok(FanLevel::FullSpeed),
            [d @ b'0'..=b'7'] => Ok(FanLevel::Level(d - b'0')),
            _ => Err(invalid("fan level (auto, full-speed or 0-7)", arg)),
        }
    }
}

/// Package file extensions pacman writes to its cache.
const PKG_EXTS: [&str; 9] = [
    ".pkg.tar",
    ".pkg.tar.zst",
    ".pkg.tar.xz",
    ".pkg.tar.gz",
    ".pkg.tar.bz2",
    ".pkg.tar.lz4",
    ".pkg.tar.lrz",
    ".pkg.tar.lzo",
    ".pkg.tar.Z",
];

/// A package file directly in [`PKG_CACHE`]: the name is
/// `[A-Za-z0-9@._+:-]`, starts with a letter or digit, and ends in a
/// package extension (never a `.sig`).
pub fn package_file(arg: &OsStr) -> Result<PathBuf, Error> {
    let bad = || {
        invalid(
            "package (must be a package file in /var/cache/pacman/pkg)",
            arg,
        )
    };
    let parts = below(PKG_CACHE, arg).ok_or_else(bad)?;
    let [name] = parts.as_slice() else {
        return Err(bad());
    };
    let charset = name
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || b"@._+:-".contains(c));
    let first = name.first().is_some_and(u8::is_ascii_alphanumeric);
    let ext = PKG_EXTS.iter().any(|e| {
        name.strip_suffix(e.as_bytes())
            .is_some_and(|stem| !stem.is_empty())
    });
    if charset && first && ext {
        Ok(PathBuf::from(arg))
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    #[test]
    fn etc_paths_are_plain_and_confined() {
        assert!(etc_path(os("/etc/locale.gen")).is_ok());
        assert!(etc_path(os("/etc/pam.d/system-auth")).is_ok());
        for bad in [
            "/etc",
            "/etc/",
            "/etc/../root/.ssh/authorized_keys",
            "/etc/pam.d/../../root/x",
            "/etc/./passwd",
            "/etc//passwd",
            "/etc/passwd/",
            "/etcfoo/passwd",
            "etc/passwd",
            "/usr/lib/os-release",
            "/etc/..",
            "",
        ] {
            assert!(etc_path(os(bad)).is_err(), "{bad:?} accepted");
        }
        let long = format!("/etc/{}", "a".repeat(256));
        assert!(etc_path(os(&long)).is_err(), "name over NAME_MAX");
        let nul = OsString::from_vec(b"/etc/a\0/../../root".to_vec());
        assert!(etc_path(&nul).is_err(), "NUL byte");
        let non_utf8 = OsString::from_vec(b"/etc/caf\xe9.conf".to_vec());
        assert!(
            etc_path(&non_utf8).is_ok(),
            "any byte but / and NUL is a name"
        );
    }

    #[test]
    fn review_files_carry_a_pacman_suffix_and_a_live_name() {
        assert!(review_path(os("/etc/locale.gen.pacnew")).is_ok());
        assert!(review_path(os("/etc/x/y.pacsave")).is_ok());
        assert!(review_path(os("/etc/x/y.pacorig")).is_ok());
        for bad in [
            "/etc/.pacnew",
            "/etc/x/.pacsave",
            "/etc/passwd",
            "/etc/shadow.pacnew.bak",
            "/etc/../x.pacnew",
            "/tmp/x.pacnew",
        ] {
            assert!(review_path(os(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn a_pacnew_pair_is_exactly_live_plus_suffix() {
        let (l, r) = pacnew_pair(os("/etc/locale.gen"), os("/etc/locale.gen.pacnew")).unwrap();
        assert_eq!(l, PathBuf::from("/etc/locale.gen"));
        assert_eq!(r, PathBuf::from("/etc/locale.gen.pacnew"));
        assert!(pacnew_pair(os("/etc/a"), os("/etc/a.pacsave")).is_ok());
        for (live, review) in [
            ("/etc/shadow", "/etc/locale.gen.pacnew"),
            ("/etc/a", "/etc/b/a.pacnew"),
            ("/etc/a", "/etc/a.pacnew.pacnew"),
            ("/etc/../etc/a", "/etc/../etc/a.pacnew"),
            ("/root/a", "/root/a.pacnew"),
            ("/etc/a.pacnew", "/etc/a.pacnew"),
        ] {
            assert!(
                pacnew_pair(os(live), os(review)).is_err(),
                "{live} {review}"
            );
        }
    }

    #[test]
    fn pids_and_start_times_are_plain_decimals() {
        assert_eq!(pid(os("4242")).unwrap(), 4242);
        for bad in [
            "0",
            "1",
            "-1",
            "+5",
            " 5",
            "5 ",
            "05",
            "0x10",
            "",
            "4294967296",
            "99999999999",
            "1e3",
        ] {
            assert!(pid(os(bad)).is_err(), "{bad:?} accepted");
        }
        assert_eq!(start_time(os("123456")).unwrap(), 123456);
        assert_eq!(start_time(os("0")).unwrap(), 0);
        for bad in ["-3", "007", "1.5", "", "18446744073709551616", "12a"] {
            assert!(start_time(os(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn only_term_and_kill() {
        assert_eq!(Signal::parse(os("TERM")).unwrap(), Signal::Term);
        assert_eq!(Signal::parse(os("KILL")).unwrap().number(), libc::SIGKILL);
        for bad in ["term", "9", "SIGKILL", "HUP", "STOP", "", "KILL "] {
            assert!(Signal::parse(os(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn devices_are_dev_alnum_names() {
        assert_eq!(device(os("/dev/nvme0")).unwrap(), "/dev/nvme0");
        assert!(device(os("/dev/sda")).is_ok());
        for bad in [
            "/dev/",
            "/dev/../etc/shadow",
            "/dev/disk/by-id/x",
            "/dev/nvme0; rm -rf ~",
            "/dev/sda1 ",
            "/dev/-a",
            "nvme0",
            "/proc/self/mem",
            "/dev/nvme0n1p1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(device(os(bad)).is_err(), "{bad:?} accepted");
        }
        let nul = OsString::from_vec(b"/dev/sda\0".to_vec());
        assert!(device(&nul).is_err());
    }

    #[test]
    fn fan_levels_come_from_the_allow_list() {
        assert_eq!(FanLevel::parse(os("auto")).unwrap(), FanLevel::Auto);
        assert_eq!(
            FanLevel::parse(os("full-speed")).unwrap(),
            FanLevel::FullSpeed
        );
        for n in 0..=7u8 {
            let s = n.to_string();
            assert_eq!(FanLevel::parse(os(&s)).unwrap(), FanLevel::Level(n));
            assert_eq!(FanLevel::Level(n).keyword(), s);
        }
        for bad in [
            "8",
            "disengaged",
            "level 3",
            "3\nwatchdog 0",
            "enable",
            "disable",
            "",
            "07",
        ] {
            assert!(FanLevel::parse(os(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn packages_come_from_the_pacman_cache_only() {
        let ok = "/var/cache/pacman/pkg/mkinitcpio-42.1-1-any.pkg.tar.zst";
        assert_eq!(package_file(os(ok)).unwrap(), PathBuf::from(ok));
        assert!(
            package_file(os(
                "/var/cache/pacman/pkg/python-1:3.11-1-x86_64.pkg.tar.xz"
            ))
            .is_ok()
        );
        for bad in [
            "/var/cache/pacman/pkg/x;reboot.pkg.tar.zst",
            "/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst.sig",
            "/var/cache/pacman/pkg/../../../tmp/evil.pkg.tar.zst",
            "/var/cache/pacman/pkg/sub/a-1-1-any.pkg.tar.zst",
            "/var/cache/pacman/pkg/-U.pkg.tar.zst",
            "/var/cache/pacman/pkg/.hidden.pkg.tar.zst",
            "/var/cache/pacman/pkg/.pkg.tar.zst",
            "/var/cache/pacman/pkg/a b.pkg.tar.zst",
            "/tmp/a-1-1-any.pkg.tar.zst",
            "a-1-1-any.pkg.tar.zst",
            "/var/cache/pacman/pkg/a-1-1-any.tar.zst",
        ] {
            assert!(package_file(os(bad)).is_err(), "{bad:?} accepted");
        }
    }
}
