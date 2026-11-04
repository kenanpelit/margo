//! Minimal systemd queries over D-Bus, instead of spawning `systemctl`
//! (each spawn is a process plus a fresh bus connection with its own
//! Hello / AddMatch traffic). One shared connection per bus.

use tokio::sync::OnceCell;
use zbus::Connection;
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const DEST: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";

static SYSTEM: OnceCell<Connection> = OnceCell::const_new();
static SESSION: OnceCell<Connection> = OnceCell::const_new();

/// The shared system-bus connection (system units, e.g. `blocky.service`).
pub async fn system_bus() -> zbus::Result<Connection> {
    SYSTEM.get_or_try_init(Connection::system).await.cloned()
}

/// The shared session-bus connection (`systemctl --user` units).
pub async fn session_bus() -> zbus::Result<Connection> {
    SESSION.get_or_try_init(Connection::session).await.cloned()
}

/// Object path of `unit`, or `None` when systemd has no such unit loaded
/// (what `systemctl` reports as inactive / `MainPID=0`).
async fn unit_path(conn: &Connection, unit: &str) -> zbus::Result<Option<OwnedObjectPath>> {
    match conn
        .call_method(
            Some(DEST),
            MANAGER_PATH,
            Some(MANAGER_IFACE),
            "GetUnit",
            &(unit,),
        )
        .await
    {
        Ok(reply) => Ok(Some(reply.body().deserialize()?)),
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == NO_SUCH_UNIT => Ok(None),
        Err(e) => Err(e),
    }
}

async fn property(
    conn: &Connection,
    path: &OwnedObjectPath,
    iface: &'static str,
    name: &str,
) -> zbus::Result<OwnedValue> {
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(DEST)?
        .path(path.as_ref())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    Ok(props
        .get(InterfaceName::from_static_str_unchecked(iface), name)
        .await?)
}

/// `systemctl is-active --quiet <unit>`: `true` for an active (or
/// reloading) unit, `false` when it is anything else or not loaded.
pub async fn unit_active(conn: &Connection, unit: &str) -> zbus::Result<bool> {
    let Some(path) = unit_path(conn, unit).await? else {
        return Ok(false);
    };
    let state = property(conn, &path, "org.freedesktop.systemd1.Unit", "ActiveState").await?;
    let state: String = state.try_into()?;
    Ok(is_active_state(&state))
}

/// `systemctl show -p MainPID <unit>`: the service's main PID, `0` when it
/// is not running or not loaded.
pub async fn service_main_pid(conn: &Connection, unit: &str) -> zbus::Result<u32> {
    let Some(path) = unit_path(conn, unit).await? else {
        return Ok(0);
    };
    let pid = property(conn, &path, "org.freedesktop.systemd1.Service", "MainPID").await?;
    Ok(pid.try_into()?)
}

/// The `ActiveState` values `systemctl is-active` reports as success.
pub fn is_active_state(state: &str) -> bool {
    matches!(state, "active" | "reloading")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_and_reloading_count_as_active() {
        assert!(is_active_state("active"));
        assert!(is_active_state("reloading"));
        for s in ["inactive", "failed", "activating", "deactivating", ""] {
            assert!(!is_active_state(s), "{s:?}");
        }
    }

    /// Read-only queries against the live buses (`cargo test -- --ignored
    /// live_queries`): checks the calls and reply types. Not part of the
    /// normal run.
    #[tokio::test]
    #[ignore]
    async fn live_queries() {
        let session = session_bus().await.expect("session bus");
        let pid = service_main_pid(&session, "pipewire.service").await;
        let missing = service_main_pid(&session, "no-such-unit-xyz.service").await;
        eprintln!("pipewire MainPID {pid:?}, missing unit {missing:?}");
        assert_eq!(missing.ok(), Some(0));
        let system = system_bus().await.expect("system bus");
        let blocky = unit_active(&system, "blocky.service").await;
        let dbus = unit_active(&system, "systemd-journald.service").await;
        eprintln!("blocky active {blocky:?}, dbus active {dbus:?}");
        assert_eq!(dbus.ok(), Some(true));
    }
}
