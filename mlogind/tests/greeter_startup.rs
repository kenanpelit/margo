//! The GUI greeter's startup command is `mlogind __greeter-startup <mgreet>
//! <mctl> [<mkeys>]`, run by margo with the greeter socket on
//! `$MLOGIND_SOCK_FD`. It must hand that socket to mgreet and to nothing else:
//! not to mkeys, not to `mctl dispatch quit`, and not keep a copy of its own
//! once mgreet has it. This drives the real binary with stand-in scripts that
//! record which descriptors they hold.

use std::fs;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Shell that lists every descriptor target of the running process into `out`.
fn list_fds(out: &Path) -> String {
    format!(
        "for f in /proc/$$/fd/*; do readlink \"$f\"; done > {o}.tmp; echo \"env=$MLOGIND_SOCK_FD\" >> {o}.tmp; mv {o}.tmp {o}",
        o = out.display()
    )
}

fn wait_for(path: &Path) -> String {
    for _ in 0..500 {
        if let Ok(text) = fs::read_to_string(path) {
            return text;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{} was never written", path.display());
}

fn run(with_osk: bool) {
    let dir = std::env::temp_dir().join(format!(
        "mlogind-greeter-startup-{}-{with_osk}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let mgreet_out = dir.join("mgreet.out");
    let mkeys_out = dir.join("mkeys.out");
    let mctl_out = dir.join("mctl.out");
    // With an OSK, mgreet waits (bounded) until mkeys has recorded its fds:
    // the launcher stops the OSK the moment mgreet exits.
    let wait_osk = if with_osk {
        format!(
            "i=0; while [ ! -e {} ] && [ $i -lt 500 ]; do sleep 0.01; i=$((i+1)); done\n",
            mkeys_out.display()
        )
    } else {
        String::new()
    };
    let mgreet = script(
        &dir,
        "mgreet",
        &format!("{wait_osk}{}", list_fds(&mgreet_out)),
    );
    // The OSK lives until it is told to go; `exec` so the TERM reaches sleep.
    let mkeys = script(
        &dir,
        "mkeys",
        &format!("{}\nexec sleep 30", list_fds(&mkeys_out)),
    );
    let mctl = script(
        &dir,
        "mctl",
        &format!(
            "{}\necho \"args=$*\" >> {}",
            list_fds(&mctl_out),
            mctl_out.display()
        ),
    );

    let (mut runner, greeter) = UnixStream::pair().unwrap();
    let greeter: OwnedFd = greeter.into();
    let raw = greeter.as_raw_fd();
    let socket = fs::read_link(format!("/proc/self/fd/{raw}"))
        .unwrap()
        .display()
        .to_string();

    // As margo hands it over: inheritable in this one child, number in the env.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mlogind"));
    cmd.arg("__greeter-startup").arg(&mgreet).arg(&mctl);
    if with_osk {
        cmd.arg(&mkeys);
    }
    cmd.env("MLOGIND_SOCK_FD", raw.to_string());
    // SAFETY: one fcntl on a descriptor this test owns.
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let status = cmd.status().unwrap();
    drop(greeter);
    assert!(status.success(), "the launcher exits with mgreet's status");

    let mgreet_fds = wait_for(&mgreet_out);
    assert!(
        mgreet_fds.contains(&socket),
        "mgreet must get the greeter socket: {mgreet_fds}"
    );
    assert!(mgreet_fds.contains(&format!("env={raw}")), "{mgreet_fds}");

    let mctl_seen = wait_for(&mctl_out);
    assert!(
        !mctl_seen.contains(&socket),
        "mctl must not get it: {mctl_seen}"
    );
    assert!(mctl_seen.contains("env=\n"), "nor its number: {mctl_seen}");
    assert!(mctl_seen.contains("args=dispatch quit"), "{mctl_seen}");

    if with_osk {
        let mkeys_seen = wait_for(&mkeys_out);
        assert!(
            !mkeys_seen.contains(&socket),
            "mkeys must not get it: {mkeys_seen}"
        );
        assert!(
            mkeys_seen.contains("env=\n"),
            "nor its number: {mkeys_seen}"
        );
    }

    // Every copy of the greeter's end is gone — the launcher's own, and the
    // OSK's (it was stopped, not left running) — so the runner reads EOF.
    runner
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = Vec::new();
    let read = runner.read_to_end(&mut buf);
    assert!(
        matches!(read, Ok(0)),
        "the runner must see EOF once mgreet is gone, got {read:?}"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn only_mgreet_receives_the_greeter_socket() {
    run(false);
}

#[test]
fn the_osk_and_the_quit_never_see_the_greeter_socket() {
    run(true);
}
