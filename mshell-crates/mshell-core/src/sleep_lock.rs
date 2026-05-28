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

use mshell_session::session_lock::{lock_session, session_locked};
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

/// Wait — within the inhibitor's grace window — for `mlock` to come up,
/// then a short settle so its lock surface is mapped before the system
/// is allowed to sleep. Bounded so a failed locker never blocks suspend.
async fn wait_for_lock() {
    for _ in 0..30 {
        if session_locked() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
}

/// Spawn the lock-before-sleep task. Idempotent enough to call once at
/// shell start; reconnects with a short backoff if the system bus drops.
pub fn spawn() {
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
