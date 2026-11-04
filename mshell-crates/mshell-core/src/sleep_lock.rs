//! Lock-before-sleep.
//!
//! Holds a logind **`delay`** sleep inhibitor and, the moment the system
//! is about to suspend / hibernate, locks the screen with `mlock` first —
//! then drops the inhibitor so the sleep proceeds. This catches *every*
//! suspend path (power menu, lid switch, `systemctl suspend`, logind's
//! own idle), not just the shell's own idle-timer suspend in
//! `idle_manager`. Without it the machine could sleep unlocked and resume
//! straight onto the desktop.
//!
//! The inhibitor is a `delay` lock (not `block`), so logind still sleeps
//! after at most `InhibitDelayMaxSec` even if `mlock` somehow stalls.
//!
//! "Locked" means the compositor confirmed the ext-session-lock (margo's
//! `session_locked` in `watch state`, mirrored here into
//! `mshell_session`), not merely that a process named `mlock` exists.

use mshell_services::margo_service;
use mshell_session::session_lock::{
    lock_session, session_locked, set_compositor_locked, wait_until_locked,
};
use std::time::Duration;
use tracing::{info, warn};
use zbus::zvariant::OwnedFd;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

async fn take_inhibitor(mgr: &Login1ManagerProxy<'_>) -> Option<OwnedFd> {
    match mgr
        .inhibit("sleep", "margo", "Lock screen before sleep", "delay")
        .await
    {
        Ok(fd) => Some(fd),
        Err(e) => {
            warn!("sleep-lock: failed to take the delay inhibitor: {e}");
            None
        }
    }
}

/// How long to wait for the compositor to confirm the lock. Inside logind's
/// default 5 s `InhibitDelayMaxSec`, leaving room for the settle below.
const LOCK_CONFIRM_LIMIT: Duration = Duration::from_secs(3);

/// Wait — within the inhibitor's grace window — for the compositor to
/// confirm the lock, then a short settle so mlock's lock surface is painted
/// before the system is allowed to sleep. Bounded so a failed locker never
/// blocks suspend.
async fn wait_for_lock() {
    if !wait_until_locked(LOCK_CONFIRM_LIMIT).await {
        warn!("sleep-lock: lock not confirmed within {LOCK_CONFIRM_LIMIT:?}; sleeping anyway");
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
}

/// Spawn the lock-before-sleep task. Idempotent enough to call once at
/// shell start; reconnects with a short backoff if the system bus drops.
/// Also mirrors margo's lock state into `mshell_session` for every
/// `session_locked()` caller.
pub fn spawn() {
    tokio::spawn(async {
        use futures::StreamExt;
        let mut locked = margo_service().session_locked.watch();
        while let Some(state) = locked.next().await {
            set_compositor_locked(state);
        }
    });
    tokio::spawn(async {
        loop {
            if let Err(e) = run().await {
                warn!("sleep-lock: task exited ({e}); retrying in 5s");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn run() -> zbus::Result<()> {
    use futures::StreamExt;

    let conn = zbus::Connection::system().await?;
    let mgr = Login1ManagerProxy::new(&conn).await?;
    let mut inhibitor = take_inhibitor(&mgr).await;
    let mut stream = mgr.receive_prepare_for_sleep().await?;
    info!("sleep-lock: armed (holding logind delay inhibitor)");

    while let Some(signal) = stream.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.start {
            // About to sleep: lock first, then release the inhibitor.
            if !session_locked() {
                info!("sleep-lock: locking with mlock before sleep");
                lock_session();
                wait_for_lock().await;
            }
            // Dropping the fd releases the delay → the system sleeps.
            inhibitor.take();
        } else {
            // Resumed: re-arm the inhibitor for the next sleep.
            inhibitor = take_inhibitor(&mgr).await;
        }
    }

    Ok(())
}
