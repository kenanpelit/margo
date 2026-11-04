use gtk4_session_lock as session_lock;
use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info};

thread_local! {
    static SESSION_LOCK: RefCell<Option<session_lock::Instance>> = const { RefCell::new(None) };
}

/// The compositor's lock state, as margo publishes it in `watch state`
/// (`session_locked`). `None` until the first snapshot, or when the running
/// margo predates the field; then [`session_locked`] falls back to looking
/// for our own `mlock` process.
static COMPOSITOR_LOCKED: LazyLock<watch::Sender<Option<bool>>> =
    LazyLock::new(|| watch::channel(None).0);

/// An `mlock` we spawned is still running (it may not have locked yet).
static OWN_LOCKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// The shell's own gtk4-session-lock client. Retained for the now-dormant
/// in-process `LockScreenManager`; the canonical way to lock is
/// [`lock_session`], which delegates to the standalone `mlock` binary.
pub fn session_lock() -> session_lock::Instance {
    SESSION_LOCK.with(|lock| {
        let mut lock = lock.borrow_mut();
        lock.get_or_insert_with(session_lock::Instance::new).clone()
    })
}

/// Feed the compositor's lock state (the margo service's `session_locked`).
/// The shell calls this whenever that changes.
pub fn set_compositor_locked(locked: Option<bool>) {
    COMPOSITOR_LOCKED.send_if_modified(|cur| {
        let changed = *cur != locked;
        *cur = locked;
        changed
    });
}

/// Subscribe to the compositor's lock state (see [`set_compositor_locked`]).
pub fn compositor_locked() -> watch::Receiver<Option<bool>> {
    COMPOSITOR_LOCKED.subscribe()
}

/// Lock the screen via the standalone `mlock` binary — a dedicated,
/// isolated process that holds the `ext-session-lock` and survives a
/// shell crash (the whole reason a locker is a separate process). This
/// is the single screen-locker; the old in-process GTK lock no longer
/// activates. No-op while the session is locked or an `mlock` we started
/// is still coming up, since a second ext-session-lock client would just
/// be rejected by the compositor.
pub fn lock_session() {
    if !should_spawn_locker(session_locked(), OWN_LOCKER_RUNNING.load(Ordering::SeqCst)) {
        info!("lock_session: already locked or locking — ignoring");
        return;
    }
    OWN_LOCKER_RUNNING.store(true, Ordering::SeqCst);
    match std::process::Command::new("mlock").spawn() {
        Ok(mut child) => {
            // Reap on a detached thread so the finished locker doesn't
            // linger as a zombie after the user unlocks.
            std::thread::spawn(move || {
                let _ = child.wait();
                OWN_LOCKER_RUNNING.store(false, Ordering::SeqCst);
            });
        }
        Err(e) => {
            OWN_LOCKER_RUNNING.store(false, Ordering::SeqCst);
            error!("lock_session: failed to spawn mlock: {e}");
        }
    }
}

/// Whether the screen is currently locked. The compositor's own state when
/// it reports one (a cheap read, fine on the main thread); otherwise (an
/// older margo) whether an `mlock` owned by this user is running. The
/// shell's own `session_lock` client is not the locker, so its
/// `is_locked()` can't answer this.
pub fn session_locked() -> bool {
    let reported = *COMPOSITOR_LOCKED.borrow();
    locked_from(reported, own_mlock_running)
}

/// Wait until the compositor confirms the lock (it then no longer shows
/// the desktop), at most `limit`. `true` once locked. Without compositor
/// state it polls for our own `mlock` instead.
pub async fn wait_until_locked(limit: Duration) -> bool {
    wait_locked_on(compositor_locked(), limit, own_mlock_running).await
}

fn locked_from(reported: Option<bool>, scan: impl FnOnce() -> bool) -> bool {
    reported.unwrap_or_else(scan)
}

fn should_spawn_locker(locked: bool, own_locker_running: bool) -> bool {
    !locked && !own_locker_running
}

async fn wait_locked_on(
    mut rx: watch::Receiver<Option<bool>>,
    limit: Duration,
    scan: impl Fn() -> bool,
) -> bool {
    const POLL: Duration = Duration::from_millis(50);
    let wait = async {
        loop {
            let reported = *rx.borrow_and_update();
            match reported {
                Some(true) => return,
                Some(false) => {
                    if rx.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                }
                None => {
                    if scan() {
                        return;
                    }
                    let _ = tokio::time::timeout(POLL, rx.changed()).await;
                }
            }
        }
    };
    tokio::time::timeout(limit, wait).await.is_ok()
}

fn own_mlock_running() -> bool {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    mlock_running_in(Path::new("/proc"), uid)
}

/// True if a live process named exactly `mlock` and owned by `uid` is
/// running under `proc_root`. Only the fallback for a margo that doesn't
/// report its lock state. Another user's `mlock` (any binary renamed so)
/// must not count: it would make lock-before-sleep and "lock now" skip
/// locking this session.
fn mlock_running_in(proc_root: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return false;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(pid) = fname.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if entry.metadata().map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let dir = entry.path();
        let Ok(comm) = std::fs::read_to_string(dir.join("comm")) else {
            continue;
        };
        if comm.trim() != "mlock" {
            continue;
        }
        let stat = std::fs::read_to_string(dir.join("stat")).ok();
        if is_live_mlock(&comm, stat.as_deref()) {
            return true;
        }
    }
    false
}

/// Whether a process with these `/proc/<pid>/comm` and (if readable)
/// `/proc/<pid>/stat` contents is a live `mlock`.
fn is_live_mlock(comm: &str, stat: Option<&str>) -> bool {
    if comm.trim() != "mlock" {
        return false;
    }
    // Skip a zombie (`<defunct>`): an mlock that has exited but not
    // yet been reaped still carries comm == "mlock", yet it isn't
    // locking anything. Counting it would wedge `lock_session`
    // forever ("already running" → never spawns a real lock), which
    // is exactly the "mshellctl lock does nothing but check says
    // locked" failure. /proc/<pid>/stat is `PID (comm) STATE …`;
    // comm may contain ')', so read the state after the LAST ')'.
    if let Some(stat) = stat
        && let Some((_, rest)) = stat.rsplit_once(')')
        && rest.trim_start().starts_with('Z')
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_or_sleeping_mlock_is_live() {
        assert!(is_live_mlock(
            "mlock\n",
            Some("4242 (mlock) S 1 4242 4242 0 -1")
        ));
        assert!(is_live_mlock("mlock", Some("4242 (mlock) R 1")));
        assert!(
            is_live_mlock("mlock\n", None),
            "an unreadable stat still counts: better a no-op lock than two lockers"
        );
    }

    #[test]
    fn zombie_mlock_is_not_live() {
        assert!(!is_live_mlock(
            "mlock\n",
            Some("4242 (mlock) Z 1 4242 4242 0 -1")
        ));
        // The state is read after the last `)`.
        assert!(!is_live_mlock("mlock", Some("7 (a) b) Z 1")));
    }

    #[test]
    fn other_processes_are_not_mlock() {
        for comm in ["mlockd", "xmlock", "", "MLOCK", "mlock2"] {
            assert!(!is_live_mlock(comm, Some("1 (x) S")), "{comm:?}");
        }
    }

    #[test]
    fn malformed_stat_does_not_hide_a_live_locker() {
        assert!(is_live_mlock("mlock", Some("")));
        assert!(is_live_mlock("mlock", Some("garbage without parens")));
        assert!(is_live_mlock("mlock", Some("1 (mlock)")));
    }

    fn fake_proc(entries: &[(&str, &str, &str)]) -> tempfile::TempDir {
        let proc = tempfile::tempdir().unwrap();
        for (pid, comm, stat) in entries {
            let dir = proc.path().join(pid);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(dir.join("stat"), stat).unwrap();
        }
        proc
    }

    #[test]
    fn only_our_own_mlock_counts() {
        // SAFETY: getuid has no preconditions.
        let me = unsafe { libc::getuid() };
        let proc = fake_proc(&[
            ("10", "bash", "10 (bash) S"),
            ("11", "mlock", "11 (mlock) S"),
        ]);
        assert!(mlock_running_in(proc.path(), me));
        // The same process as seen by another user: someone else's `mlock`
        // (e.g. `cp /bin/sleep /tmp/mlock`) must not make us skip locking.
        assert!(!mlock_running_in(proc.path(), me.wrapping_add(1)));
        let zombie = fake_proc(&[("12", "mlock", "12 (mlock) Z 1")]);
        assert!(!mlock_running_in(zombie.path(), me));
        assert!(!mlock_running_in(&proc.path().join("missing"), me));
    }

    #[test]
    fn compositor_state_wins_over_the_process_scan() {
        let scan_panics = || -> bool { panic!("must not scan /proc when margo reports") };
        assert!(locked_from(Some(true), scan_panics));
        assert!(!locked_from(Some(false), scan_panics));
        assert!(locked_from(None, || true));
        assert!(!locked_from(None, || false));
    }

    #[test]
    fn a_locker_is_spawned_only_when_unlocked_and_none_is_pending() {
        assert!(should_spawn_locker(false, false));
        assert!(!should_spawn_locker(true, false));
        assert!(!should_spawn_locker(false, true));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_for_the_lock_returns_on_confirmation_only() {
        let never = || -> bool { panic!("no scan with compositor state") };
        let (tx, rx) = watch::channel(Some(false));
        let waiter = tokio::spawn(wait_locked_on(rx, Duration::from_secs(3), never));
        tokio::time::sleep(Duration::from_millis(400)).await;
        tx.send(Some(true)).unwrap();
        assert!(waiter.await.unwrap());

        // A locker that never confirms: bounded, reports failure.
        let (_tx, rx) = watch::channel(Some(false));
        assert!(!wait_locked_on(rx, Duration::from_secs(3), never).await);

        // Already locked: immediate.
        let (_tx, rx) = watch::channel(Some(true));
        assert!(wait_locked_on(rx, Duration::from_millis(1), never).await);
    }

    #[tokio::test(start_paused = true)]
    async fn without_compositor_state_the_wait_polls_the_fallback() {
        use std::sync::atomic::AtomicUsize;
        let (_tx, rx) = watch::channel(None);
        let calls = AtomicUsize::new(0);
        let up_on_third = || calls.fetch_add(1, Ordering::SeqCst) >= 2;
        assert!(wait_locked_on(rx, Duration::from_secs(3), up_on_third).await);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
