//! Integration tests for the start-margo supervisor: they run the real
//! `start-margo` binary against a fake `margo` shell script so the spawn /
//! restart / crash-budget / signal-forwarding paths are exercised end-to-end
//! (the unit tests in `main.rs` only cover the pure helpers).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A unique scratch directory under the system temp dir, cleaned on drop.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("start-margo-it-{}-{}", std::process::id(), n));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// Write an executable fake-margo script and return its path.
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A `start-margo` invocation pre-wired for tests: fake margo via `--path`, no
/// mctl preflight, no sd_notify, and logging redirected into the scratch dir.
fn supervisor(scratch: &Scratch, fake_margo: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_start-margo"));
    cmd.arg("--path")
        .arg(fake_margo)
        .arg("--no-preflight")
        .arg("--no-notify")
        // Keep the test snappy: don't wait the 20s readiness fallback.
        .arg("--ready-timeout-secs")
        .arg("0")
        .env("XDG_STATE_HOME", scratch.dir.join("state"))
        .env_remove("NOTIFY_SOCKET")
        .env_remove("WATCHDOG_USEC");
    cmd
}

#[test]
fn clean_exit_returns_zero() {
    let scratch = Scratch::new();
    let fake = scratch.script("margo", "#!/bin/sh\nexit 0\n");
    let status = supervisor(&scratch, &fake).status().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "clean margo exit should propagate 0"
    );
}

#[test]
fn crash_budget_exhausts_to_69() {
    let scratch = Scratch::new();
    // Always crashes immediately.
    let fake = scratch.script("margo", "#!/bin/sh\nexit 1\n");
    let status = supervisor(&scratch, &fake)
        .arg("--max-crashes")
        .arg("2")
        .arg("--restart-window-secs")
        .arg("60")
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(69),
        "exhausted crash budget should exit EX_UNAVAILABLE (69)"
    );
}

#[test]
fn no_restart_propagates_child_code() {
    let scratch = Scratch::new();
    let fake = scratch.script("margo", "#!/bin/sh\nexit 17\n");
    let status = supervisor(&scratch, &fake)
        .arg("--no-restart")
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(17),
        "--no-restart should propagate margo's own exit code"
    );
}

#[test]
fn safe_config_makes_a_final_attempt() {
    let scratch = Scratch::new();
    // The primary config always crashes; the safe config records that it ran
    // (then also exits non-zero so the supervisor still terminates promptly).
    let marker = scratch.path("safe-ran");
    let fake = scratch.script(
        "margo",
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = \"{}\" ]; then\n    : > \"{}\"\n  fi\ndone\nexit 1\n",
            scratch.path("safe.conf").display(),
            marker.display()
        ),
    );
    let status = supervisor(&scratch, &fake)
        .arg("--max-crashes")
        .arg("2")
        .arg("--safe-config")
        .arg(scratch.path("safe.conf"))
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(69),
        "safe mode still gives up eventually"
    );
    assert!(
        marker.exists(),
        "safe-config should have been tried with -c <safe.conf>"
    );
}

#[test]
fn sigterm_is_forwarded_for_graceful_teardown() {
    let scratch = Scratch::new();
    let marker = scratch.path("term-received");
    // Fake margo: signal it's up, trap SIGTERM to record graceful teardown,
    // then idle until told to stop. SIGKILL (the escalation path) would skip
    // the trap and leave no marker.
    let fake = scratch.script(
        "margo",
        &format!(
            "#!/bin/sh\ntrap ': > \"{}\"; exit 0' TERM\nif [ -n \"$MARGO_READY_FD\" ]; then\n  printf 'READY=1\\n' >&\"$MARGO_READY_FD\"\nfi\nwhile true; do sleep 0.1; done\n",
            marker.display()
        ),
    );

    let mut child = supervisor(&scratch, &fake).spawn().unwrap();

    // Wait for start-margo to actually spawn the fake margo (the trap must be
    // installed before we signal) — poll for the child process to settle.
    std::thread::sleep(Duration::from_millis(400));

    // SAFETY: send SIGTERM to the supervisor we just spawned.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }

    // It should exit promptly (well under the 5s shutdown timeout).
    let deadline = Instant::now() + Duration::from_secs(4);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("start-margo did not exit after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    assert!(
        marker.exists(),
        "margo should have received the forwarded SIGTERM (graceful teardown), not SIGKILL"
    );
    assert_eq!(
        status.code(),
        Some(0),
        "graceful child exit during shutdown propagates its code"
    );
}
