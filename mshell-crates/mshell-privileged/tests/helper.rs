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
            input.write_all(b).expect("feed");
        }
        drop(input);
        child.wait_with_output().expect("wait")
    }
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

#[test]
fn kill_rechecks_the_start_time() {
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
    // kthreadd: a kernel thread is refused even with its true start time.
    let kstat = std::fs::read_to_string("/proc/2/stat").expect("kthreadd");
    let kstart = mshell_privileged::procsig::parse_stat(&kstat)
        .expect("parse")
        .0
        .to_string();
    assert_eq!(code(&f.run(&["kill", "2", &kstart, "KILL"])), 3);

    let o = f.run(&["kill", &p, &s, "TERM"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!child.wait().expect("wait").success(), "terminated");
}

#[test]
fn smart_read_runs_smartctl_per_validated_drive() {
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

#[test]
fn the_policy_has_one_action_per_subcommand() {
    let policy = include_str!("../data/org.margo.mshell.privileged.policy");
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
