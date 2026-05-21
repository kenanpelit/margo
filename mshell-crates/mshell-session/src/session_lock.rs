use gtk4_session_lock as session_lock;
use std::cell::RefCell;
use tracing::{error, info};

thread_local! {
    static SESSION_LOCK: RefCell<Option<session_lock::Instance>> = const { RefCell::new(None) };
}

/// The shell's own gtk4-session-lock client. Retained for the now-dormant
/// in-process `LockScreenManager`; the canonical way to lock is
/// [`lock_session`], which delegates to the standalone `mlock` binary.
pub fn session_lock() -> session_lock::Instance {
    SESSION_LOCK.with(|lock| {
        let mut lock = lock.borrow_mut();
        lock.get_or_insert_with(session_lock::Instance::new).clone()
    })
}

/// Lock the screen via the standalone `mlock` binary — a dedicated,
/// isolated process that holds the `ext-session-lock` and survives a
/// shell crash (the whole reason a locker is a separate process). This
/// is the single screen-locker; the old in-process GTK lock no longer
/// activates. No-op if `mlock` is already running, since a second
/// ext-session-lock client would just be rejected by the compositor.
pub fn lock_session() {
    if mlock_running() {
        info!("lock_session: mlock already running — ignoring");
        return;
    }
    match std::process::Command::new("mlock").spawn() {
        Ok(mut child) => {
            // Reap on a detached thread so the finished locker doesn't
            // linger as a zombie after the user unlocks.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => error!("lock_session: failed to spawn mlock: {e}"),
    }
}

/// Whether the screen is currently locked — i.e. an `mlock` process is
/// live. The shell's own `session_lock` client is no longer the locker,
/// so its `is_locked()` can't answer this.
pub fn session_locked() -> bool {
    mlock_running()
}

/// True if any process named exactly `mlock` is running. Scans `/proc`
/// directly to avoid a hard dependency on `pgrep`.
fn mlock_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(pid) = fname.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
            continue;
        };
        if comm.trim() != "mlock" {
            continue;
        }
        // Skip a zombie (`<defunct>`): an mlock that has exited but not
        // yet been reaped still carries comm == "mlock", yet it isn't
        // locking anything. Counting it would wedge `lock_session`
        // forever ("already running" → never spawns a real lock), which
        // is exactly the "mshellctl lock does nothing but check says
        // locked" failure. /proc/<pid>/stat is `PID (comm) STATE …`;
        // comm may contain ')', so read the state after the LAST ')'.
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && let Some((_, rest)) = stat.rsplit_once(')')
            && rest.trim_start().starts_with('Z')
        {
            continue;
        }
        return true;
    }
    false
}
