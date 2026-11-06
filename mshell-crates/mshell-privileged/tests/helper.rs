//! The real helper binary, run as the ordinary test user against fixture
//! trees (`MSHELL_PRIVILEGED_SYSROOT`): no sudo, and never the live /etc.

use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_mshell-privileged");

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical");
        std::fs::create_dir_all(root.join("etc")).expect("etc");
        Fixture { _dir: dir, root }
    }

    fn p(&self, logical: &str) -> PathBuf {
        self.root.join(logical.trim_start_matches('/'))
    }

    fn write(&self, logical: &str, content: &str) -> PathBuf {
        let p = self.p(logical);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(&p, content).expect("write");
        p
    }

    fn read(&self, logical: &str) -> String {
        std::fs::read_to_string(self.p(logical)).expect("read")
    }

    /// A fake program in the fixture that prints its argv, one per line.
    fn fake(&self, logical: &str) {
        let p = self.write(logical, "#!/bin/sh\nfor a do printf '%s\\n' \"$a\"; done\n");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_stdin(args, None)
    }

    fn run_stdin(&self, args: &[&str], stdin: Option<&[u8]>) -> Output {
        // As root the helper ignores MSHELL_PRIVILEGED_SYSROOT by design, so
        // it would act on the machine's real /etc, sysfs and pacman.
        assert!(!as_root(), "never run the helper fixtures as root");
        let mut child = Command::new(BIN)
            .args(args)
            .env("MSHELL_PRIVILEGED_SYSROOT", &self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn helper");
        let mut input = child.stdin.take().expect("stdin");
        if let Some(b) = stdin {
            // A helper that refuses its argv exits before reading stdin;
            // the write then races its exit and may hit a closed pipe.
            match input.write_all(b) {
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                r => r.expect("feed"),
            }
        }
        drop(input);
        child.wait_with_output().expect("wait")
    }
}

/// The fixtures need the helper to honour `MSHELL_PRIVILEGED_SYSROOT`, which
/// it ignores when any uid is 0 (a root caller must not redirect it). CI's
/// container runs the tests as root: there these tests skip, and the
/// validators stay covered by the crate's unit tests.
fn as_root() -> bool {
    // SAFETY: plain getters.
    unsafe { libc::getuid() == 0 || libc::geteuid() == 0 }
}

macro_rules! skip_as_root {
    () => {
        if as_root() {
            eprintln!("skipped as root: the helper ignores the fixture sysroot");
            return;
        }
    };
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn mode(p: &Path) -> u32 {
    std::fs::symlink_metadata(p)
        .expect("stat")
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn install_keeps_mode_backs_up_and_consumes_the_review_file() {
    skip_as_root!();
    let f = Fixture::new();
    let live = f.write("/etc/f.conf", "mine\n");
    f.write("/etc/f.conf.pacnew", "theirs\n");
    std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o640)).expect("chmod");
    let o = f.run(&["pacnew-install", "/etc/f.conf", "/etc/f.conf.pacnew"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read("/etc/f.conf"), "theirs\n");
    assert_eq!(f.read("/etc/f.conf.mshell-bak"), "mine\n");
    assert_eq!(mode(&live), 0o640);
    assert_eq!(mode(&f.p("/etc/f.conf.mshell-bak")), 0o640);
    assert!(
        !f.p("/etc/f.conf.pacnew").exists(),
        "the .pacnew is consumed"
    );
    let leftovers: Vec<_> = std::fs::read_dir(f.p("/etc"))
        .expect("ls")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("mshell-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files left");
}

#[test]
fn a_merge_arrives_on_stdin() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/etc/sub/f.conf", "mine\n");
    f.write("/etc/sub/f.conf.pacnew", "theirs\n");
    let o = f.run_stdin(
        &["pacnew-merge", "/etc/sub/f.conf", "/etc/sub/f.conf.pacnew"],
        Some(b"merged\n"),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read("/etc/sub/f.conf"), "merged\n");
    assert_eq!(f.read("/etc/sub/f.conf.mshell-bak"), "mine\n");
    assert!(!f.p("/etc/sub/f.conf.pacnew").exists());
}

#[test]
fn restoring_a_pacsave_creates_the_live_file_0644() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/etc/gone.conf.pacsave", "saved\n");
    let o = f.run(&["pacnew-install", "/etc/gone.conf", "/etc/gone.conf.pacsave"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read("/etc/gone.conf"), "saved\n");
    assert_eq!(mode(&f.p("/etc/gone.conf")), 0o644);
    assert!(
        !f.p("/etc/gone.conf.mshell-bak").exists(),
        "nothing to back up"
    );
}

#[test]
fn a_symlinked_live_file_is_refused() {
    skip_as_root!();
    // systemd-resolved's /etc/resolv.conf is a symlink: writing through it
    // (or replacing it with a 0777 copy of the link's mode) is refused.
    let f = Fixture::new();
    let target = f.write("/run/stub-resolv.conf", "nameserver 127.0.0.53\n");
    symlink(&target, f.p("/etc/resolv.conf")).expect("symlink");
    f.write("/etc/resolv.conf.pacnew", "# new\n");
    let o = f.run(&[
        "pacnew-install",
        "/etc/resolv.conf",
        "/etc/resolv.conf.pacnew",
    ]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("symlink"), "{}", stderr(&o));
    assert!(
        std::fs::symlink_metadata(f.p("/etc/resolv.conf"))
            .expect("lstat")
            .file_type()
            .is_symlink()
    );
    assert_eq!(f.read("/run/stub-resolv.conf"), "nameserver 127.0.0.53\n");
    assert!(f.p("/etc/resolv.conf.pacnew").exists(), "nothing consumed");
}

#[test]
fn a_symlinked_directory_or_review_file_is_refused() {
    skip_as_root!();
    let f = Fixture::new();
    // A dotfiles dir linked into /etc.
    f.write("/home/u/dots/pacman.conf", "mine\n");
    f.write("/home/u/dots/pacman.conf.pacnew", "theirs\n");
    symlink(f.p("/home/u/dots"), f.p("/etc/pacman.d")).expect("symlink");
    let o = f.run(&[
        "pacnew-install",
        "/etc/pacman.d/pacman.conf",
        "/etc/pacman.d/pacman.conf.pacnew",
    ]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(f.read("/home/u/dots/pacman.conf"), "mine\n");
    let o = f.run(&["pacnew-remove", "/etc/pacman.d/pacman.conf.pacnew"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(f.p("/home/u/dots/pacman.conf.pacnew").exists());

    // A review file that is a symlink to a secret is never copied.
    f.write("/etc/shadow", "root:hash\n");
    f.write("/etc/x.conf", "mine\n");
    symlink(f.p("/etc/shadow"), f.p("/etc/x.conf.pacnew")).expect("symlink");
    let o = f.run(&["pacnew-install", "/etc/x.conf", "/etc/x.conf.pacnew"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(f.read("/etc/x.conf"), "mine\n");
    let o = f.run(&["pacnew-remove", "/etc/x.conf.pacnew"]);
    assert_eq!(code(&o), 3, "a symlinked review file is not removed either");

    // A symlinked /etc itself.
    let g = Fixture::new();
    std::fs::remove_dir(g.p("/etc")).expect("rmdir");
    g.write("/elsewhere/a.pacnew", "x\n");
    symlink(g.p("/elsewhere"), g.p("/etc")).expect("symlink");
    let o = g.run(&["pacnew-remove", "/etc/a.pacnew"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(g.p("/elsewhere/a.pacnew").exists());
}

#[test]
fn a_planted_backup_symlink_is_replaced_not_followed() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/etc/f.conf", "mine\n");
    f.write("/etc/f.conf.pacnew", "theirs\n");
    let victim = f.write("/root/.ssh/authorized_keys", "KEY\n");
    symlink(&victim, f.p("/etc/f.conf.mshell-bak")).expect("symlink");
    let o = f.run(&["pacnew-install", "/etc/f.conf", "/etc/f.conf.pacnew"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read("/root/.ssh/authorized_keys"), "KEY\n");
    assert!(
        std::fs::symlink_metadata(f.p("/etc/f.conf.mshell-bak"))
            .expect("lstat")
            .file_type()
            .is_file()
    );
}

#[test]
fn fifos_and_directories_are_refused() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/etc/a.conf", "x\n");
    let fifo = f.p("/etc/a.conf.pacnew");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).expect("cstr");
    // SAFETY: NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let o = f.run(&["pacnew-install", "/etc/a.conf", "/etc/a.conf.pacnew"]);
    assert_eq!(code(&o), 3, "a FIFO would hang the read: {}", stderr(&o));
    std::fs::create_dir(f.p("/etc/d.conf")).expect("mkdir");
    f.write("/etc/d.conf.pacnew", "x\n");
    let o = f.run(&["pacnew-install", "/etc/d.conf", "/etc/d.conf.pacnew"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
}

#[test]
fn hostile_paths_never_reach_the_file_system() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/root/x.pacnew", "x\n");
    f.write("/etc/passwd", "root\n");
    for args in [
        &["pacnew-remove", "/etc/../root/x.pacnew"][..],
        &["pacnew-remove", "/root/x.pacnew"],
        &["pacnew-remove", "/etc/passwd"],
        &["pacnew-install", "/etc/passwd", "/etc/x.pacnew"],
        &["pacnew-install", "/etc/./passwd", "/etc/./passwd.pacnew"],
        &["pacnew-merge", "/etc//passwd", "/etc//passwd.pacnew"],
        &["pacnew-install", "/etc/passwd"],
        &["sh", "-c", "id"],
        &[],
    ] {
        let o = f.run(args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert!(f.p("/root/x.pacnew").exists());
    assert_eq!(f.read("/etc/passwd"), "root\n");
}

#[test]
fn remove_deletes_only_the_review_file() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/etc/a.conf", "live\n");
    f.write("/etc/a.conf.pacsave", "saved\n");
    let o = f.run(&["pacnew-remove", "/etc/a.conf.pacsave"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!f.p("/etc/a.conf.pacsave").exists());
    assert_eq!(f.read("/etc/a.conf"), "live\n");
    let o = f.run(&["pacnew-remove", "/etc/a.conf.pacsave"]);
    assert_eq!(code(&o), 3, "already gone");
}

#[test]
fn fan_writes_one_allowed_line_to_the_fixed_file() {
    skip_as_root!();
    let f = Fixture::new();
    f.write("/proc/acpi/ibm/fan", "");
    let o = f.run(&["fan-set", "3"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read("/proc/acpi/ibm/fan"), "level 3\n");
    for bad in ["8", "disengaged", "3\nwatchdog 0", "level 3", "enable"] {
        let o = f.run(&["fan-set", bad]);
        assert_eq!(code(&o), 2, "{bad:?}");
    }
    assert_eq!(f.read("/proc/acpi/ibm/fan"), "level 3\n");
    assert_eq!(code(&f.run(&["fan-set", "auto", "extra"])), 2);

    // A symlink in place of the control file is not written through.
    let g = Fixture::new();
    let victim = g.write("/etc/sudoers", "root ALL\n");
    std::fs::create_dir_all(g.p("/proc/acpi/ibm")).expect("mkdir");
    symlink(&victim, g.p("/proc/acpi/ibm/fan")).expect("symlink");
    assert_eq!(code(&g.run(&["fan-set", "full-speed"])), 3);
    assert_eq!(g.read("/etc/sudoers"), "root ALL\n");
    // No file: nothing is created.
    let h = Fixture::new();
    assert_eq!(code(&h.run(&["fan-set", "auto"])), 3);
    assert!(!h.p("/proc/acpi/ibm/fan").exists());
}

/// A battery in the fixture's `/sys/class/power_supply` with both
/// thresholds.
fn battery(f: &Fixture, name: &str, start: &str, end: &str) {
    let dir = format!("/sys/class/power_supply/{name}");
    f.write(&format!("{dir}/type"), "Battery\n");
    f.write(&format!("{dir}/charge_control_start_threshold"), start);
    f.write(&format!("{dir}/charge_control_end_threshold"), end);
}

#[test]
fn charge_limit_writes_the_thresholds_the_helper_finds_itself() {
    skip_as_root!();
    let f = Fixture::new();
    battery(&f, "BAT0", "75\n", "80\n");
    battery(&f, "BAT1", "75\n", "80\n");
    // Not batteries: the AC adapter and a peripheral's battery-less
    // supply (a fake threshold file must not be written).
    f.write("/sys/class/power_supply/AC/type", "Mains\n");
    f.write(
        "/sys/class/power_supply/AC/charge_control_end_threshold",
        "keep",
    );
    // Lowering: the start threshold goes first (start < end always).
    let o = f.run(&["charge-limit", "60"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    for b in ["BAT0", "BAT1"] {
        let d = format!("/sys/class/power_supply/{b}");
        assert_eq!(f.read(&format!("{d}/charge_control_end_threshold")), "60\n");
        assert_eq!(
            f.read(&format!("{d}/charge_control_start_threshold")),
            "55\n"
        );
    }
    assert_eq!(
        f.read("/sys/class/power_supply/AC/charge_control_end_threshold"),
        "keep"
    );
    // Raising: end first, then start.
    let o = f.run(&["charge-limit", "100"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        f.read("/sys/class/power_supply/BAT0/charge_control_end_threshold"),
        "100\n"
    );
    assert_eq!(
        f.read("/sys/class/power_supply/BAT0/charge_control_start_threshold"),
        "95\n"
    );
    // A battery with only an end threshold (generic drivers).
    let g = Fixture::new();
    g.write("/sys/class/power_supply/BAT0/type", "Battery\n");
    g.write(
        "/sys/class/power_supply/BAT0/charge_control_end_threshold",
        "100\n",
    );
    let o = g.run(&["charge-limit", "80"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        g.read("/sys/class/power_supply/BAT0/charge_control_end_threshold"),
        "80\n"
    );
    assert!(
        !g.p("/sys/class/power_supply/BAT0/charge_control_start_threshold")
            .exists(),
        "nothing is created"
    );
}

#[test]
fn charge_limit_refuses_hostile_values_and_planted_links() {
    skip_as_root!();
    let f = Fixture::new();
    battery(&f, "BAT0", "75\n", "80\n");
    for bad in [
        "0", "101", "-5", "080", "80 ", " 80", "8a", "", "80\n", "1e2", "255", "256",
    ] {
        let o = f.run(&["charge-limit", bad]);
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
    }
    assert_eq!(code(&f.run(&["charge-limit"])), 2);
    assert_eq!(code(&f.run(&["charge-limit", "80", "/etc/shadow"])), 2);
    assert_eq!(
        f.read("/sys/class/power_supply/BAT0/charge_control_end_threshold"),
        "80\n"
    );

    // A threshold file replaced by a symlink is not written through.
    let g = Fixture::new();
    let victim = g.write("/etc/sudoers", "root ALL\n");
    g.write("/sys/class/power_supply/BAT0/type", "Battery\n");
    symlink(
        &victim,
        g.p("/sys/class/power_supply/BAT0/charge_control_end_threshold"),
    )
    .expect("symlink");
    let o = g.run(&["charge-limit", "60"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(g.read("/etc/sudoers"), "root ALL\n");

    // No battery with a threshold: refused, nothing created.
    let h = Fixture::new();
    std::fs::create_dir_all(h.p("/sys/class/power_supply/BAT0")).expect("mkdir");
    h.write("/sys/class/power_supply/BAT0/type", "Battery\n");
    let o = h.run(&["charge-limit", "80"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("charge limit"), "{}", stderr(&o));
    assert!(
        !h.p("/sys/class/power_supply/BAT0/charge_control_end_threshold")
            .exists()
    );
}

const DROPIN: &str = "/etc/systemd/logind.conf.d/99-margo.conf";
const GOOD_DROPIN: &str = "# Managed by margo Settings — do not edit by hand.\n[Login]\nHandlePowerKey=ignore\nHandleLidSwitch=suspend\nHandleLidSwitchExternalPower=lock\nHandleLidSwitchDocked=suspend\n";

#[test]
fn logind_dropin_writes_the_fixed_file_from_stdin() {
    skip_as_root!();
    let f = Fixture::new();
    std::fs::create_dir_all(f.p("/etc/systemd")).expect("mkdir");
    let o = f.run_stdin(&["logind-dropin"], Some(GOOD_DROPIN.as_bytes()));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read(DROPIN), GOOD_DROPIN);
    assert_eq!(mode(&f.p(DROPIN)), 0o644);
    // Rewriting replaces it.
    let next = GOOD_DROPIN.replace("HandlePowerKey=ignore", "HandlePowerKey=poweroff");
    let o = f.run_stdin(&["logind-dropin"], Some(next.as_bytes()));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(f.read(DROPIN), next);
    let leftovers = std::fs::read_dir(f.p("/etc/systemd/logind.conf.d"))
        .expect("ls")
        .count();
    assert_eq!(leftovers, 1, "no temp files left");
    // It takes no arguments: a path on the command line is a usage error.
    let o = f.run_stdin(&["logind-dropin", "/etc/shadow"], Some(next.as_bytes()));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn logind_dropin_refuses_anything_but_the_settings_keys() {
    skip_as_root!();
    let f = Fixture::new();
    std::fs::create_dir_all(f.p("/etc/systemd/logind.conf.d")).expect("mkdir");
    f.write(DROPIN, "old\n");
    let long = format!("[Login]\nHandlePowerKey=ignore\n#{}\n", "x".repeat(5000));
    for bad in [
        "",
        "HandlePowerKey=ignore\n",
        "[Login]\n",
        "[Login]\nKillUserProcesses=no\n",
        "[Login]\nHandlePowerKey=ignore\nHandlePowerKey=lock\n",
        "[Login]\nHandlePowerKey=factory-reset\n",
        "[Login]\nHandlePowerKey=ignore\n[Other]\nHandleLidSwitch=lock\n",
        "[Login]\n[Login]\nHandlePowerKey=ignore\n",
        "[Login]\nHandlePowerKey=ignore\\\nKillUserProcesses=no\n",
        "[Login]\n# comment \\\nHandlePowerKey=ignore\n",
        "[Login]\nHandlePowerKey = ignore\n",
        "[Login]\nHandlePowerKey=ignore\r\n",
        "[Login]\nHandlePowerKey=ignore\0\n",
        "[Login]\nHandlePowerKey=ignore suspend\n",
        "[Login]\nHandlePowerKey\n",
        "[Login]\n.include /tmp/evil.conf\n",
        &long,
    ] {
        let o = f.run_stdin(&["logind-dropin"], Some(bad.as_bytes()));
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
    }
    let o = f.run_stdin(
        &["logind-dropin"],
        Some(b"[Login]\nHandlePowerKey=caf\xe9\n"),
    );
    assert_eq!(code(&o), 2, "non-UTF-8");
    assert_eq!(f.read(DROPIN), "old\n", "nothing written");
}

#[test]
fn logind_dropin_refuses_planted_links() {
    skip_as_root!();
    // A symlinked drop-in (or drop-in directory) is not written through.
    let f = Fixture::new();
    let victim = f.write("/etc/sudoers", "root ALL\n");
    std::fs::create_dir_all(f.p("/etc/systemd/logind.conf.d")).expect("mkdir");
    symlink(&victim, f.p(DROPIN)).expect("symlink");
    let o = f.run_stdin(&["logind-dropin"], Some(GOOD_DROPIN.as_bytes()));
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(f.read("/etc/sudoers"), "root ALL\n");
    assert!(
        std::fs::symlink_metadata(f.p(DROPIN))
            .expect("lstat")
            .file_type()
            .is_symlink()
    );

    let g = Fixture::new();
    std::fs::create_dir_all(g.p("/etc/systemd")).expect("mkdir");
    std::fs::create_dir_all(g.p("/home/u/dots")).expect("mkdir");
    symlink(g.p("/home/u/dots"), g.p("/etc/systemd/logind.conf.d")).expect("symlink");
    let o = g.run_stdin(&["logind-dropin"], Some(GOOD_DROPIN.as_bytes()));
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(!g.p("/home/u/dots/99-margo.conf").exists());

    // The drop-in name taken by a directory.
    let h = Fixture::new();
    std::fs::create_dir_all(h.p(DROPIN)).expect("mkdir");
    let o = h.run_stdin(&["logind-dropin"], Some(GOOD_DROPIN.as_bytes()));
    assert_eq!(code(&o), 3, "{}", stderr(&o));
}

#[test]
fn kill_rechecks_the_start_time() {
    skip_as_root!();
    let f = Fixture::new();
    let mut child = Command::new("sleep").arg("30").spawn().expect("sleep");
    let pid = child.id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
    let start = mshell_privileged::procsig::parse_stat(&stat)
        .expect("parse")
        .0;
    let (p, s) = (pid.to_string(), start.to_string());
    let reused = (start + 1).to_string();
    let o = f.run(&["kill", &p, &reused, "TERM"]);
    assert_eq!(code(&o), 3);
    assert_eq!(
        stderr(&o).trim(),
        format!("Process {pid} has already exited")
    );
    assert!(
        child.try_wait().expect("wait").is_none(),
        "a reused pid is not signalled"
    );
    for bad in [
        &["kill", "1", "0", "TERM"][..],
        &["kill", &p, &s, "HUP"],
        &["kill", &p, &s, "9"],
        &["kill", "-1", &s, "KILL"],
        &["kill", &p, "-5", "TERM"],
        &["kill", &p, &s],
    ] {
        assert_eq!(code(&f.run(bad)), 2, "{bad:?}");
    }
    assert!(child.try_wait().expect("wait").is_none());
    // A kernel thread is refused even with its true start time. Use any one
    // this /proc shows: a container's pid namespace (CI) has none, and the
    // refusal itself is unit-tested against a fixture /proc in procsig.rs.
    if let Some((kpid, kstart)) = visible_kernel_thread() {
        let (kpid, kstart) = (kpid.to_string(), kstart.to_string());
        assert_eq!(code(&f.run(&["kill", &kpid, &kstart, "KILL"])), 3);
    }

    let o = f.run(&["kill", &p, &s, "TERM"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!child.wait().expect("wait").success(), "terminated");
}

#[test]
fn smart_read_runs_smartctl_per_validated_drive() {
    skip_as_root!();
    let f = Fixture::new();
    f.fake("/usr/bin/smartctl");
    for bad in ["/dev/../etc/shadow", "/dev/sda;id", "/dev/", "sda"] {
        assert_eq!(code(&f.run(&["smart-read", bad])), 2, "{bad:?}");
    }
    // /dev/stdin is a symlink, /dev/shm a directory, /dev/null a
    // character device that is not a drive, /dev/nosuch absent.
    for bad in ["/dev/stdin", "/dev/shm", "/dev/null", "/dev/nosuch0"] {
        let o = f.run(&["smart-read", bad]);
        assert_eq!(code(&o), 3, "{bad:?}: {}", stderr(&o));
        assert!(o.stdout.is_empty(), "smartctl not run for {bad}");
    }
    // A real block device on this machine, if the test can see one.
    use std::os::unix::fs::FileTypeExt;
    let Some(dev) = std::fs::read_dir("/dev")
        .expect("dev")
        .flatten()
        .find_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let ok = name.bytes().all(|b| b.is_ascii_alphanumeric()) && name.len() <= 32;
            (ok && e.file_type().ok()?.is_block_device()).then(|| format!("/dev/{name}"))
        })
    else {
        return;
    };
    let o = f.run(&["smart-read", &dev, &dev]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(out, format!("-j\n-a\n{dev}\n-j\n-a\n{dev}\n"));
    // Missing smartctl.
    let g = Fixture::new();
    let o = g.run(&["smart-read", &dev]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("smartctl is not installed"));
}

#[test]
fn upgrade_and_rollback_exec_pacman_with_a_fixed_argv() {
    skip_as_root!();
    let f = Fixture::new();
    f.fake("/usr/bin/pacman");
    let o = f.run(&["upgrade"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "-Syu\n--noconfirm\n--noprogressbar\n--color\nnever\n"
    );
    assert_eq!(code(&f.run(&["upgrade", "--overwrite", "*"])), 2);

    let pkg = "/var/cache/pacman/pkg/mkinitcpio-42.1-1-any.pkg.tar.zst";
    f.write(pkg, "pkg");
    let o = f.run(&["rollback", pkg]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        format!(
            "-U\n--noconfirm\n--noprogressbar\n--color\nnever\n--\n{}\n",
            f.p(pkg).display()
        )
    );
    for bad in [
        "/tmp/evil-1-1-any.pkg.tar.zst",
        "/var/cache/pacman/pkg/../../../tmp/evil-1-1-any.pkg.tar.zst",
        "/var/cache/pacman/pkg/x;reboot.pkg.tar.zst",
        "/var/cache/pacman/pkg/a-1-1-any.pkg.tar.zst.sig",
    ] {
        assert_eq!(code(&f.run(&["rollback", bad])), 2, "{bad:?}");
    }
    // A symlinked package (to a file the user planted) or a missing one.
    let evil = f.write("/home/u/evil-1-1-any.pkg.tar.zst", "evil");
    symlink(&evil, f.p("/var/cache/pacman/pkg/evil-1-1-any.pkg.tar.zst")).expect("symlink");
    let o = f.run(&["rollback", "/var/cache/pacman/pkg/evil-1-1-any.pkg.tar.zst"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(o.stdout.is_empty(), "pacman not run");
    assert_eq!(
        code(&f.run(&["rollback", "/var/cache/pacman/pkg/none-1-1-any.pkg.tar.zst"])),
        3
    );
}

/// A fake efibootmgr: no arguments prints the fixture listing (the one
/// captured on the owner's machine, or `listing`), anything else echoes
/// its argv one per line.
fn fake_efibootmgr(f: &Fixture, listing: &str) {
    f.write("/usr/bin/efibootmgr.listing", listing);
    let script = format!(
        "#!/bin/sh\nif [ $# -eq 0 ]; then cat '{}'; exit 0; fi\nfor a do printf '%s\\n' \"$a\"; done\n",
        f.p("/usr/bin/efibootmgr.listing").display()
    );
    let p = f.write("/usr/bin/efibootmgr", &script);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

const OWNER_EFI: &str = include_str!("data/efibootmgr.txt");

#[test]
fn boot_next_sets_only_a_listed_entry() {
    skip_as_root!();
    let f = Fixture::new();
    fake_efibootmgr(&f, OWNER_EFI);
    let o = f.run(&["boot-next", "0003"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "--quiet\n--bootnext\n0003\n"
    );
    // A well-formed number the firmware doesn't have is refused as root.
    let o = f.run(&["boot-next", "0099"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(o.stdout.is_empty(), "efibootmgr not run to set it");
    assert!(stderr(&o).contains("Boot0099"), "{}", stderr(&o));
    // Hostile or malformed arguments never reach efibootmgr.
    for bad in [
        &["boot-next", "3"][..],
        &["boot-next", "001f"],
        &["boot-next", "--delete-bootorder"],
        &["boot-next", "-o"],
        &["boot-next", "0003", "--bootorder"],
        &["boot-next", "0003;reboot"],
        &["boot-next", ""],
        &["boot-next"],
    ] {
        let o = f.run(bad);
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
        assert!(o.stdout.is_empty(), "{bad:?}");
    }
}

#[test]
fn boot_next_clears_only_a_pending_entry() {
    skip_as_root!();
    let f = Fixture::new();
    fake_efibootmgr(&f, OWNER_EFI);
    // Nothing pending: success without running efibootmgr -N (which would
    // fail with ENOENT).
    let o = f.run(&["boot-next", "none"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(o.stdout.is_empty());
    let pending = OWNER_EFI.replace("Timeout:", "BootNext: 0003\nTimeout:");
    fake_efibootmgr(&f, &pending);
    let o = f.run(&["boot-next", "none"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "--quiet\n--delete-bootnext\n"
    );
}

#[test]
fn boot_next_without_efibootmgr_or_efi() {
    skip_as_root!();
    let f = Fixture::new();
    let o = f.run(&["boot-next", "0001"]);
    assert_eq!(code(&o), 1);
    assert!(
        stderr(&o).contains("efibootmgr is not installed"),
        "{}",
        stderr(&o)
    );
    // Legacy BIOS boot: efibootmgr fails to read the variables.
    let p = f.write(
        "/usr/bin/efibootmgr",
        "#!/bin/sh\necho 'EFI variables are not supported on this system.' >&2\nexit 2\n",
    );
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let o = f.run(&["boot-next", "0001"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("not supported"), "{}", stderr(&o));
}

/// A thinkpad_acpi LED in the fixture's `/sys/class/leds`.
fn led(f: &Fixture, name: &str, brightness: &str, max: &str) {
    let dir = format!("/sys/class/leds/{name}");
    f.write(&format!("{dir}/brightness"), brightness);
    f.write(&format!("{dir}/max_brightness"), max);
}

#[test]
fn led_set_writes_the_brightness_of_a_thinkpad_led() {
    skip_as_root!();
    let f = Fixture::new();
    led(&f, "tpacpi::lid_logo_dot", "0\n", "255\n");
    led(&f, "tpacpi::thinklight", "0\n", "1\n");
    let o = f.run(&["led-set", "tpacpi::lid_logo_dot", "255"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        f.read("/sys/class/leds/tpacpi::lid_logo_dot/brightness"),
        "255\n"
    );
    let o = f.run(&["led-set", "tpacpi::lid_logo_dot", "0"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        f.read("/sys/class/leds/tpacpi::lid_logo_dot/brightness"),
        "0\n"
    );
    // Above the LED's own maximum: refused, untouched.
    let o = f.run(&["led-set", "tpacpi::thinklight", "2"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("0-1"), "{}", stderr(&o));
    assert_eq!(
        f.read("/sys/class/leds/tpacpi::thinklight/brightness"),
        "0\n"
    );
}

#[test]
fn led_set_refuses_other_leds_hostile_names_and_planted_links() {
    skip_as_root!();
    let f = Fixture::new();
    led(&f, "input3::capslock", "0\n", "1\n");
    led(&f, "tpacpi::power", "0\n", "255\n");
    for bad in [
        &["led-set", "input3::capslock", "1"][..],
        &["led-set", "tpacpi::../input3::capslock", "1"],
        &["led-set", "tpacpi::power/../../input3::capslock", "1"],
        &["led-set", "tpacpi::power", "256"],
        &["led-set", "tpacpi::power", "-1"],
        &["led-set", "tpacpi::power", "1\nheartbeat"],
        &["led-set", "tpacpi::power"],
        &["led-set", "tpacpi::power", "1", "extra"],
    ] {
        let o = f.run(bad);
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
    }
    assert_eq!(f.read("/sys/class/leds/input3::capslock/brightness"), "0\n");
    assert_eq!(f.read("/sys/class/leds/tpacpi::power/brightness"), "0\n");
    // No such LED: refused, nothing created.
    let o = f.run(&["led-set", "tpacpi::lid_logo_dot", "1"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(!f.p("/sys/class/leds/tpacpi::lid_logo_dot").exists());
    // A brightness file replaced by a symlink is not written through.
    let g = Fixture::new();
    let victim = g.write("/etc/sudoers", "root ALL\n");
    g.write("/sys/class/leds/tpacpi::power/max_brightness", "255\n");
    symlink(&victim, g.p("/sys/class/leds/tpacpi::power/brightness")).expect("symlink");
    let o = g.run(&["led-set", "tpacpi::power", "1"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(g.read("/etc/sudoers"), "root ALL\n");
}

#[test]
fn the_policy_has_one_action_per_subcommand() {
    let policy = include_str!("../data/org.margo.mshell.privileged.policy");
    // The packages install the helper where the code runs it (Nix
    // rewrites both to the store path).
    if option_env!("MSHELL_PRIVILEGED_HELPER").is_none() {
        assert_eq!(
            mshell_privileged::HELPER,
            "/usr/lib/mshell/mshell-privileged"
        );
    }
    let repo = |rel: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(rel),
        )
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    for (file, needle) in [
        ("PKGBUILD", "\"$pkgdir/usr/lib/mshell/mshell-privileged\""),
        (
            "PKGBUILD",
            "/usr/share/polkit-1/actions/org.margo.mshell.privileged.policy",
        ),
        ("install.sh", "\"/usr/lib/mshell/mshell-privileged\""),
        (
            "install.sh",
            "/usr/share/polkit-1/actions/org.margo.mshell.privileged.policy",
        ),
        (
            "nix/default.nix",
            "MSHELL_PRIVILEGED_HELPER=\"$out/lib/mshell/mshell-privileged\"",
        ),
        (
            "nix/default.nix",
            "$out/share/polkit-1/actions/org.margo.mshell.privileged.policy",
        ),
    ] {
        assert!(repo(file).contains(needle), "{file} lost {needle}");
    }
    for sub in mshell_privileged::SUBCOMMANDS {
        let id = format!("<action id=\"{}{sub}\">", mshell_privileged::ACTION_PREFIX);
        let start = policy
            .find(&id)
            .unwrap_or_else(|| panic!("no action for {sub}"));
        let block = &policy[start..start + policy[start..].find("</action>").expect("closed")];
        assert!(
            block.contains(&format!(
                "<annotate key=\"org.freedesktop.policykit.exec.path\">{}</annotate>",
                "/usr/lib/mshell/mshell-privileged"
            )),
            "{sub}: exec.path"
        );
        assert!(
            block.contains(&format!(
                "<annotate key=\"org.freedesktop.policykit.exec.argv1\">{sub}</annotate>"
            )),
            "{sub}: exec.argv1"
        );
        assert!(block.contains("<message>"), "{sub}: message");
        assert!(
            !block.contains("<allow_active>yes"),
            "{sub}: never without authentication"
        );
    }
    assert_eq!(
        policy.matches("<action id=").count(),
        mshell_privileged::SUBCOMMANDS.len()
    );
}

/// A kernel thread (PF_KTHREAD) visible in this /proc, with its start time.
fn visible_kernel_thread() -> Option<(i32, u64)> {
    const PF_KTHREAD: u64 = 0x0020_0000;
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: i32 = e.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
        let (start, flags) = mshell_privileged::procsig::parse_stat(&stat)?;
        (flags & PF_KTHREAD != 0).then_some((pid, start))
    })
}
