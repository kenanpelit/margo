//! logind's `Unlock` signal for margo's session → `MargoState::logind_unlock`.
//!
//! `loginctl unlock-session` (run from a TTY or over ssh, i.e. after
//! authenticating outside the locked seat) makes logind emit
//! `org.freedesktop.login1.Session.Unlock` on the session. That is the one
//! way to end a session lock without the locker, for a locker that can't be
//! restarted (see `state::lock_guard`); `force_unlock` only restarts it.
//!
//! A blocking zbus signal iterator runs on its own thread and forwards each
//! signal over a calloop channel to the compositor thread. Best-effort: no
//! system bus or no logind session just means the escape isn't available,
//! which is logged once.

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const LOGIN1: &str = "org.freedesktop.login1";

/// Start the listener thread; every `Unlock` signal sends `()` on `tx`.
pub fn start(tx: calloop::channel::Sender<()>) {
    let spawned = std::thread::Builder::new()
        .name("logind-unlock".into())
        .spawn(move || {
            if let Err(e) = listen(&tx) {
                tracing::warn!(
                    error = %e,
                    "logind: no Unlock listener, `loginctl unlock-session` will not unlock margo"
                );
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "logind: could not start the Unlock listener thread");
    }
}

fn listen(tx: &calloop::channel::Sender<()>) -> anyhow::Result<()> {
    let conn = Connection::system()?;
    let manager = Proxy::new(
        &conn,
        LOGIN1,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    let path = session_path(&manager)?;
    tracing::info!(session = %path.as_str(), "logind: listening for Unlock");
    let session = Proxy::new(&conn, LOGIN1, path, "org.freedesktop.login1.Session")?;
    for _signal in session.receive_signal("Unlock")? {
        if tx.send(()).is_err() {
            // The compositor side is gone (shutdown).
            break;
        }
    }
    Ok(())
}

/// margo's logind session: the one its process belongs to, else
/// `$XDG_SESSION_ID` (margo started outside the session scope, e.g. from a
/// user service).
fn session_path(manager: &Proxy<'_>) -> anyhow::Result<OwnedObjectPath> {
    let by_pid: zbus::Result<OwnedObjectPath> =
        manager.call("GetSessionByPID", &std::process::id());
    match by_pid {
        Ok(path) => Ok(path),
        Err(pid_err) => {
            let id = std::env::var("XDG_SESSION_ID").map_err(|_| {
                anyhow::anyhow!("not in a logind session ({pid_err}) and no XDG_SESSION_ID")
            })?;
            Ok(manager.call("GetSession", &id.as_str())?)
        }
    }
}

#[cfg(test)]
mod tests {
    /// Read-only check against the real system bus: margo's session
    /// resolves the way `listen` looks it up. Ignored by default (CI has no
    /// logind session); run with `--ignored` inside a desktop session.
    #[test]
    #[ignore]
    fn session_path_resolves_on_a_real_system_bus() {
        let conn = zbus::blocking::Connection::system().expect("system bus");
        let manager = zbus::blocking::Proxy::new(
            &conn,
            super::LOGIN1,
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .expect("manager proxy");
        let path = super::session_path(&manager).expect("session path");
        assert!(
            path.as_str()
                .starts_with("/org/freedesktop/login1/session/")
        );
        let session = zbus::blocking::Proxy::new(
            &conn,
            super::LOGIN1,
            path,
            "org.freedesktop.login1.Session",
        )
        .expect("session proxy");
        // Subscribing must succeed (match rule accepted); nothing is sent.
        let _signals = session.receive_signal("Unlock").expect("subscribe");
    }
}
