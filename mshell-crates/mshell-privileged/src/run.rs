//! The helper's `main`: parse + validate the argv ([`Request::parse`]),
//! then run exactly one fixed operation. Programs it starts (pacman,
//! smartctl, efibootmgr, ufw, systemctl) are absolute paths with a fixed
//! argv; nothing goes through a shell.

use crate::fsops::{self, Source, Sysroot};
use crate::{Error, Request, validate};
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
/// Root-owned places efibootmgr is installed.
const EFIBOOTMGR: [&str; 3] = [
    "/usr/bin/efibootmgr",
    "/usr/sbin/efibootmgr",
    "/run/current-system/sw/bin/efibootmgr",
];
/// Root-owned places ufw is installed (Arch, Debian, NixOS).
const UFW: [&str; 3] = [
    "/usr/bin/ufw",
    "/usr/sbin/ufw",
    "/run/current-system/sw/bin/ufw",
];
/// Root-owned places systemctl is installed.
const SYSTEMCTL: [&str; 3] = [
    "/usr/bin/systemctl",
    "/bin/systemctl",
    "/run/current-system/sw/bin/systemctl",
];
/// The `PATH` ufw runs with (it starts iptables, sysctl, …): root's
/// fixed directories only, never the caller's.
const UFW_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/run/current-system/sw/bin";
/// How often `smart-selftest` reports progress.
const SELFTEST_POLL: Duration = Duration::from_secs(15);
/// A self-test watch ends after this even if the reader stays (an HDD's
/// long test takes hours; none takes a day).
pub const SELFTEST_WATCH_MAX: Duration = Duration::from_secs(24 * 3600);
/// Test hook (fixture sysroot only, like [`SYSROOT_ENV`]): the poll
/// interval in milliseconds.
pub const POLL_MS_ENV: &str = "MSHELL_PRIVILEGED_POLL_MS";
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
        require_drive(d)?;
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

/// `dev` must be a [`drive_node`] right under `/dev`, never a symlink.
fn require_drive(dev: &str) -> Result<(), Error> {
    use std::os::unix::fs::FileTypeExt;
    let m = std::fs::symlink_metadata(dev).map_err(|e| Error::Refused(format!("{dev}: {e}")))?;
    let ft = m.file_type();
    if !drive_node(dev, ft.is_block_device(), ft.is_char_device()) {
        return Err(Error::Refused(format!("refusing {dev}: not a drive")));
    }
    Ok(())
}

/// The fixed smartctl argv (after the binary) for a self-test step:
/// starting it, or one progress read. Compact JSON, one document per line.
pub fn selftest_args(device: &str, test: Option<validate::SelfTest>) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec!["--json=c".into()];
    match test {
        Some(t) => {
            v.push("-t".into());
            v.push(t.keyword().into());
        }
        None => v.push("-a".into()),
    }
    v.push(device.into());
    v
}

/// Run smartctl with `args`, then copy its JSON to our stdout as one
/// line. The flag is `false` once the reader closed the pipe (stop
/// quietly); the code is smartctl's exit status.
fn smartctl_line(smartctl: &Path, args: &[OsString]) -> Result<(bool, Option<i32>), Error> {
    use std::io::Write;
    let out = std::process::Command::new(smartctl)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::Io(format!("{}: {e}", smartctl.display())))?;
    let mut doc: Vec<u8> = out
        .stdout
        .into_iter()
        .filter(|b| *b != b'\n' && *b != b'\r')
        .collect();
    doc.push(b'\n');
    let mut stdout = std::io::stdout().lock();
    let alive = stdout.write_all(&doc).and_then(|()| stdout.flush()).is_ok();
    Ok((alive, out.status.code()))
}

/// `smart-selftest`: start the test, then report a `smartctl -a`
/// document every poll until the reader goes away (the shell closes the
/// pipe once it sees the test finish) or [`SELFTEST_WATCH_MAX`] passes.
/// A refused start (smartctl status bits 0-2: bad argument, open failed,
/// or the drive refused, e.g. a test already running) ends it with an
/// error after printing smartctl's own JSON.
fn smart_selftest(root: &Sysroot, device: &str, test: validate::SelfTest) -> Result<(), Error> {
    require_drive(device)?;
    let smartctl = program(root, &SMARTCTL, "smartctl")?;
    let (mut alive, code) = smartctl_line(&smartctl, &selftest_args(device, Some(test)))?;
    match code {
        Some(c) if c & 0b111 == 0 => {}
        Some(c) => {
            return Err(Error::Io(format!(
                "smartctl could not start the {} self-test on {device} (status {c})",
                test.keyword()
            )));
        }
        None => return Err(Error::Io("smartctl was killed".to_string())),
    }
    let poll = poll_interval(root);
    let end = Instant::now() + SELFTEST_WATCH_MAX;
    while alive && Instant::now() < end {
        std::thread::sleep(poll);
        alive = smartctl_line(&smartctl, &selftest_args(device, None))?.0;
    }
    Ok(())
}

/// [`SELFTEST_POLL`], or [`POLL_MS_ENV`] on a fixture sysroot (tests).
fn poll_interval(root: &Sysroot) -> Duration {
    if root.is_real() {
        return SELFTEST_POLL;
    }
    std::env::var(POLL_MS_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(SELFTEST_POLL)
}

/// The efibootmgr argv (after the binary) that applies `target` to the
/// current `list`; `None` when there is nothing to do (clearing with no
/// `BootNext` pending). An entry the listing doesn't have is refused.
pub fn boot_next_args(
    list: &crate::efiboot::BootList,
    target: validate::BootTarget,
) -> Result<Option<Vec<OsString>>, Error> {
    use validate::BootTarget;
    match target {
        BootTarget::Entry(n) => {
            if list.entry(n).is_none() {
                return Err(Error::Refused(format!(
                    "Boot{n:04X} is not a boot entry on this machine"
                )));
            }
            Ok(Some(vec![
                "--quiet".into(),
                "--bootnext".into(),
                target.keyword().into(),
            ]))
        }
        BootTarget::Clear if list.next.is_none() => Ok(None),
        BootTarget::Clear => Ok(Some(vec!["--quiet".into(), "--delete-bootnext".into()])),
    }
}

/// `boot-next`: read efibootmgr's listing as root, check the target
/// against it, then exec efibootmgr with the fixed argv.
fn boot_next(root: &Sysroot, target: validate::BootTarget) -> Result<(), Error> {
    let efi = program(root, &EFIBOOTMGR, "efibootmgr")?;
    let out = std::process::Command::new(&efi)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::Io(format!("{}: {e}", efi.display())))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.lines().map(str::trim).find(|l| !l.is_empty());
        return Err(Error::Io(format!(
            "efibootmgr could not read the boot entries{}",
            msg.map(|m| format!(": {m}")).unwrap_or_default()
        )));
    }
    let list = crate::efiboot::parse(&String::from_utf8_lossy(&out.stdout));
    match boot_next_args(&list, target)? {
        Some(args) => Err(exec(&efi, &args)),
        None => Ok(()),
    }
}

/// Run `prog args…` with a fixed environment and stdin closed; echo its
/// output to our stdout (a terminal caller sees ufw's own words) and
/// return it with the exit status.
fn run_captured(prog: &Path, args: &[String]) -> Result<(Option<i32>, String), Error> {
    use std::io::Write;
    let out = std::process::Command::new(prog)
        .args(args)
        .env_clear()
        .env("PATH", UFW_PATH)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::Io(format!("{}: {e}", prog.display())))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
    Ok((out.status.code(), text))
}

fn ufw_run(ufw: &Path, args: &[String]) -> Result<(), Error> {
    let (code, text) = run_captured(ufw, args)?;
    crate::ufw::argv::ufw_outcome(code, &text).map_err(Error::Io)
}

fn systemctl_run(systemctl: &Path, args: &[String]) -> Result<(), Error> {
    let (code, text) = run_captured(systemctl, args)?;
    match code {
        Some(0) => Ok(()),
        _ => Err(Error::Io(
            text.lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .map_or_else(
                    || format!("systemctl {} failed", args.join(" ")),
                    str::to_string,
                ),
        )),
    }
}

/// `ufw-status`: the four files' parsed lines ([`crate::ufw::rules::dump`]).
fn ufw_status(root: &Sysroot) -> Result<(), Error> {
    use crate::ufw::rules;
    use std::io::Write;
    let read = |logical: &str| match crate::ufw::read_text(&root.phys(Path::new(logical))) {
        Ok(t) => Ok(Some(t)),
        Err(crate::ufw::state::ReadError::Missing) => Ok(None),
        Err(e) => Err(Error::Io(format!("{logical}: {e:?}"))),
    };
    let text = rules::dump(
        read(rules::UFW_CONF)?.as_deref(),
        read(rules::DEFAULTS)?.as_deref(),
        read(rules::USER_RULES)?.as_deref(),
        read(rules::USER6_RULES)?.as_deref(),
    );
    std::io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|e| Error::Io(format!("writing the rules: {e}")))
}

/// `ufw-switch on`: `ufw --force enable`, then start `ufw.service` when
/// systemd has it and it isn't running (so the rules load at boot and
/// the state reads green). A unit state that can't be read is left alone.
fn ufw_switch_on(root: &Sysroot, ufw: &Path) -> Result<(), Error> {
    use crate::ufw::{argv, state};
    ufw_run(ufw, &argv::switch_argv(argv::Switch::On))?;
    let Ok(systemctl) = program(root, &SYSTEMCTL, "systemctl") else {
        return Ok(());
    };
    let show: Vec<String> = argv::SHOW_ARGS.iter().map(|s| s.to_string()).collect();
    let (code, text) = run_captured(&systemctl, &show)?;
    let unit = state::parse_show(&text);
    if code == Some(0) && unit.exists() && !unit.running() {
        systemctl_run(&systemctl, &argv::unit_argv(argv::UnitOp::Start))?;
    }
    Ok(())
}

fn ufw_op(root: &Sysroot, op: &crate::UfwOp) -> Result<(), Error> {
    use crate::UfwOp;
    use crate::ufw::argv;
    match op {
        UfwOp::Status => ufw_status(root),
        UfwOp::Unit(u) => {
            let systemctl = program(root, &SYSTEMCTL, "systemctl")?;
            systemctl_run(&systemctl, &argv::unit_argv(*u))
        }
        UfwOp::Switch(argv::Switch::On) => ufw_switch_on(root, &program(root, &UFW, "ufw")?),
        UfwOp::Switch(s) => ufw_run(&program(root, &UFW, "ufw")?, &argv::switch_argv(*s)),
        UfwOp::Add(add) => {
            let ufw = program(root, &UFW, "ufw")?;
            for args in argv::add_argvs(add) {
                ufw_run(&ufw, &args)?;
            }
            Ok(())
        }
        UfwOp::Delete(spec) => ufw_run(&program(root, &UFW, "ufw")?, &argv::delete_argv(spec)),
    }
}

/// Run one request. `stdin` feeds `pacnew-merge` and `logind-dropin`.
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
        Request::ChargeLimit { limit } => fsops::charge_limit(root, *limit),
        Request::LogindDropin => {
            let mut body = Vec::new();
            stdin
                .take(validate::MAX_LOGIND_DROPIN as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| Error::Io(format!("reading the drop-in: {e}")))?;
            validate::logind_dropin(&body)?;
            fsops::logind_dropin(root, &body)
        }
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
        Request::BootNext { target } => boot_next(root, *target),
        Request::LedSet { name, value } => fsops::led_set(root, name, *value),
        Request::SmartSelfTest { device, test } => smart_selftest(root, device, *test),
        Request::Ufw(op) => ufw_op(root, op),
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
    fn boot_next_argv_is_fixed_and_checked_against_the_listing() {
        use validate::BootTarget;
        let list = crate::efiboot::parse("BootCurrent: 0001\nBoot0001* a\tHD()\nBoot001F* b\n");
        assert_eq!(
            boot_next_args(&list, BootTarget::Entry(0x1F))
                .unwrap()
                .unwrap(),
            ["--quiet", "--bootnext", "001F"]
        );
        let e = boot_next_args(&list, BootTarget::Entry(2)).unwrap_err();
        assert_eq!(e.code(), 3);
        assert!(e.message().contains("Boot0002"), "{}", e.message());
        assert_eq!(boot_next_args(&list, BootTarget::Clear).unwrap(), None);
        let pending = crate::efiboot::parse("BootNext: 001F\nBoot001F* b\n");
        assert_eq!(
            boot_next_args(&pending, BootTarget::Clear)
                .unwrap()
                .unwrap(),
            ["--quiet", "--delete-bootnext"]
        );
    }

    #[test]
    fn selftest_argv_is_fixed() {
        use validate::SelfTest;
        assert_eq!(
            selftest_args("/dev/nvme0", Some(SelfTest::Short)),
            ["--json=c", "-t", "short", "/dev/nvme0"]
        );
        assert_eq!(
            selftest_args("/dev/sda", Some(SelfTest::Long)),
            ["--json=c", "-t", "long", "/dev/sda"]
        );
        assert_eq!(
            selftest_args("/dev/sda", None),
            ["--json=c", "-a", "/dev/sda"]
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
