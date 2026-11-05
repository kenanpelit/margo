//! The helper's `main`: parse + validate the argv ([`Request::parse`]),
//! then run exactly one fixed operation. Programs it starts (pacman,
//! smartctl) are absolute paths with a fixed argv; nothing goes through
//! a shell.

use crate::fsops::{self, Source, Sysroot};
use crate::{Error, Request, validate};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// Test hook: an unprivileged run may point the logical paths at a
/// fixture tree. Never honoured when any uid is 0 (pkexec also drops it
/// from the environment), so a root run always works on the real `/`.
pub const SYSROOT_ENV: &str = "MSHELL_PRIVILEGED_SYSROOT";

pub fn sysroot_from_env(var: Option<OsString>) -> Result<Sysroot, Error> {
    // SAFETY: getters with no arguments.
    let root = unsafe { libc::getuid() == 0 || libc::geteuid() == 0 };
    match var {
        Some(dir) if !root && !dir.is_empty() => Sysroot::fixture(Path::new(&dir)),
        _ => Ok(Sysroot::real()),
    }
}

/// The pacman binary (Arch: always here).
const PACMAN: &str = "/usr/bin/pacman";
/// Root-owned places smartctl is installed (Arch/Debian, NixOS).
const SMARTCTL: [&str; 3] = [
    "/usr/bin/smartctl",
    "/usr/sbin/smartctl",
    "/run/current-system/sw/bin/smartctl",
];
/// pacman's non-interactive flags, shared by upgrade and rollback.
const QUIET: [&str; 4] = ["--noconfirm", "--noprogressbar", "--color", "never"];

fn program(root: &Sysroot, candidates: &[&str], name: &str) -> Result<PathBuf, Error> {
    candidates
        .iter()
        .map(|c| root.phys(Path::new(c)))
        .find(|p| p.is_file())
        .ok_or_else(|| Error::Io(format!("{name} is not installed")))
}

/// Replace this process with `prog args…` (its exit status is the
/// caller's result). Returns only on failure.
fn exec(prog: &Path, args: &[OsString]) -> Error {
    let e = std::process::Command::new(prog).args(args).exec();
    Error::Io(format!("{}: {e}", prog.display()))
}

/// The pacman argv (after the binary) for a request, `None` when the
/// request isn't a pacman one.
pub fn pacman_args(req: &Request, package: Option<&Path>) -> Option<Vec<OsString>> {
    let mut v: Vec<OsString> = match req {
        Request::Upgrade => vec!["-Syu".into()],
        Request::Rollback { .. } => vec!["-U".into()],
        _ => return None,
    };
    v.extend(QUIET.iter().map(OsString::from));
    if let Some(p) = package {
        v.push("--".into());
        v.push(p.as_os_str().to_owned());
    }
    Some(v)
}

/// Whether a node is a drive smartctl may open: any block device, or
/// an NVMe controller (`/dev/nvme<N>`, a character device). Other
/// character devices are refused: opening some has side effects
/// (`/dev/watchdog` arms the watchdog).
pub fn drive_node(dev: &str, block: bool, chr: bool) -> bool {
    let nvme_ctrl = dev
        .strip_prefix("/dev/nvme")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    block || (chr && nvme_ctrl)
}

/// Every drive: `smartctl -j -a <dev>`, JSON on stdout. Each device
/// must be a [`drive_node`] right under `/dev` (never a symlink);
/// smartctl's own status is a bitmask, so the JSON decides.
fn smart_read(root: &Sysroot, devices: &[String]) -> Result<(), Error> {
    for d in devices {
        let m = std::fs::symlink_metadata(d).map_err(|e| Error::Refused(format!("{d}: {e}")))?;
        use std::os::unix::fs::FileTypeExt;
        let ft = m.file_type();
        if !drive_node(d, ft.is_block_device(), ft.is_char_device()) {
            return Err(Error::Refused(format!("refusing {d}: not a drive")));
        }
    }
    let smartctl = program(root, &SMARTCTL, "smartctl")?;
    for d in devices {
        std::process::Command::new(&smartctl)
            .args(["-j", "-a", d])
            .status()
            .map_err(|e| Error::Io(format!("{}: {e}", smartctl.display())))?;
    }
    Ok(())
}

/// Run one request. `stdin` feeds `pacnew-merge`.
pub fn execute(root: &Sysroot, req: Request, stdin: &mut dyn std::io::Read) -> Result<(), Error> {
    match &req {
        Request::PacnewInstall { live, review } => {
            fsops::pacnew_install(root, live, review, Source::Review)
        }
        Request::PacnewMerge { live, review } => {
            fsops::pacnew_install(root, live, review, Source::Stream(stdin))
        }
        Request::PacnewRemove { review } => fsops::pacnew_remove(root, review),
        Request::Kill { pid, start, signal } => crate::procsig::kill(*pid, *start, *signal),
        Request::SmartRead { devices } => smart_read(root, devices),
        Request::FanSet { level } => fsops::fan_set(root, *level),
        Request::Upgrade => {
            let pacman = program(root, &[PACMAN], "pacman")?;
            let args = pacman_args(&req, None).unwrap_or_default();
            Err(exec(&pacman, &args))
        }
        Request::Rollback { package } => {
            fsops::require_regular(root, validate::PKG_CACHE, package)?;
            let pacman = program(root, &[PACMAN], "pacman")?;
            let phys = root.phys(package);
            let args = pacman_args(&req, Some(&phys)).unwrap_or_default();
            Err(exec(&pacman, &args))
        }
    }
}

/// `main` for the binary: the exit code (0 or [`Error::code`]); the
/// error's message goes to stderr as the last line, which the shell
/// shows.
pub fn main(argv: &[OsString]) -> u8 {
    let res = sysroot_from_env(std::env::var_os(SYSROOT_ENV)).and_then(|root| {
        let req = Request::parse(argv)?;
        execute(&root, req, &mut std::io::stdin().lock())
    });
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e.message());
            e.code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacman_argv_is_fixed() {
        let up = pacman_args(&Request::Upgrade, None).unwrap();
        assert_eq!(
            up,
            ["-Syu", "--noconfirm", "--noprogressbar", "--color", "never"]
        );
        let pkg = Path::new("/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst");
        let rb = pacman_args(
            &Request::Rollback {
                package: pkg.into(),
            },
            Some(pkg),
        )
        .unwrap();
        assert_eq!(
            rb,
            [
                "-U",
                "--noconfirm",
                "--noprogressbar",
                "--color",
                "never",
                "--",
                "/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst"
            ]
        );
        assert_eq!(
            pacman_args(
                &Request::PacnewRemove {
                    review: "/etc/a.pacnew".into()
                },
                None
            ),
            None
        );
    }

    #[test]
    fn only_drives_are_opened() {
        assert!(drive_node("/dev/sda", true, false));
        assert!(drive_node("/dev/nvme0n1", true, false));
        assert!(drive_node("/dev/nvme0", false, true));
        assert!(drive_node("/dev/nvme12", false, true));
        assert!(!drive_node("/dev/watchdog", false, true));
        assert!(!drive_node("/dev/watchdog0", false, true));
        assert!(!drive_node("/dev/mem", false, true));
        assert!(!drive_node("/dev/nvme", false, true));
        assert!(
            !drive_node("/dev/nvme0", false, false),
            "a regular file named nvme0"
        );
        assert!(!drive_node("/dev/sda", false, false));
    }

    #[test]
    fn the_test_sysroot_is_only_for_ordinary_users() {
        // SAFETY: getter.
        let root = unsafe { libc::geteuid() } == 0;
        let dir = tempfile::tempdir().unwrap();
        let got = sysroot_from_env(Some(dir.path().into())).unwrap();
        assert_eq!(got.is_real(), root);
        assert!(sysroot_from_env(None).unwrap().is_real());
        assert!(sysroot_from_env(Some(OsString::new())).unwrap().is_real());
    }
}
