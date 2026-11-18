//! Global menu — the focused window's application menu in the bar.
//!
//! Off by default (`global_menu.enabled`). While on, one owner task on the
//! shared tokio runtime:
//!
//! * serves `com.canonical.AppMenu.Registrar` on the session bus (apps call
//!   `RegisterWindow(windowId, menuPath)`; the sender's unique name is the
//!   menu's service). When another registrar already owns the name (a KDE
//!   session's kded), its `GetMenus` is polled instead.
//! * follows margo's focused window and picks its registration
//!   ([`matcher::pick`]: pid, then executable name ↔ app id);
//! * reads that menu with the `com.canonical.dbusmenu` client below
//!   (`GetLayout`, re-read on `LayoutUpdated`) and publishes it.
//!
//! The bar widget renders the top level and opens a popover per menu after
//! `AboutToShow` + a fresh `GetLayout` of that submenu; a click sends
//! `Event(id, "clicked")`.
//!
//! Limits (see docs/widgets.md): only apps that register with the AppMenu
//! registrar are covered — in practice XWayland clients (GTK 3 with
//! appmenu-gtk-module under GDK_BACKEND=x11, Qt with the KDE / appmenu
//! platform theme under QT_QPA_PLATFORM=xcb, Chromium/Electron with
//! --ozone-platform=x11). Native Wayland Qt apps use KDE's
//! org_kde_kwin_appmenu protocol and GTK 4 / GApplication menubars export
//! org.gtk.Menus; neither is implemented.

pub(crate) mod matcher;

use futures::StreamExt;
use matcher::Registration;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use wayle_systray::types::menu::{MenuItem, RawMenuLayout};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const REGISTRAR_NAME: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";

#[zbus::proxy(interface = "com.canonical.dbusmenu", gen_blocking = false)]
trait DBusMenu {
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
    #[zbus(no_reply)]
    fn event(&self, id: i32, event_id: &str, data: OwnedValue, timestamp: u32) -> zbus::Result<()>;
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::Result<RawMenuLayout>;
    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "com.canonical.AppMenu.Registrar",
    default_service = "com.canonical.AppMenu.Registrar",
    default_path = "/com/canonical/AppMenu/Registrar",
    gen_blocking = false
)]
trait ExternalRegistrar {
    fn get_menus(&self) -> zbus::Result<Vec<(u32, String, OwnedObjectPath)>>;
}

/// Who serves the registrar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RegistrarMode {
    #[default]
    Off,
    Own,
    /// Another process owns the name; its menus are polled.
    External,
    /// No session bus / the name could not be served.
    Failed,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GlobalMenuState {
    pub(crate) enabled: bool,
    pub(crate) mode: RegistrarMode,
    /// The focused window's app id (empty: nothing focused).
    pub(crate) app_id: String,
    /// `(service, path)` of its menu.
    pub(crate) service: Option<(String, String)>,
    pub(crate) menu: Option<Arc<MenuItem>>,
    /// How many windows registered a menu (for the tooltip).
    pub(crate) registered: usize,
}

static STATE: LazyLock<watch::Sender<GlobalMenuState>> =
    LazyLock::new(|| watch::channel(GlobalMenuState::default()).0);

pub(crate) fn subscribe() -> watch::Receiver<GlobalMenuState> {
    STATE.subscribe()
}

thread_local! {
    static RUNNING: std::cell::RefCell<Option<CancellationToken>> = const { std::cell::RefCell::new(None) };
}

/// Start / stop from `global_menu.enabled` (main thread).
pub(crate) fn configure(enabled: bool) {
    RUNNING.with(|r| {
        let mut r = r.borrow_mut();
        match (enabled, r.is_some()) {
            (true, false) => {
                let tok = CancellationToken::new();
                mshell_services::tokio_rt_spawn(run(tok.clone()));
                *r = Some(tok);
            }
            (false, true) => {
                if let Some(t) = r.take() {
                    t.cancel();
                }
            }
            _ => {}
        }
    });
}

// ── Registrar ───────────────────────────────────────────────────────────

#[derive(Default)]
struct Shared {
    regs: Mutex<BTreeMap<u32, Registration>>,
    changed: Notify,
}

struct Registrar(Arc<Shared>);

fn proc_identities(pid: u32) -> Vec<String> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let argv0 = std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|b| {
            b.split(|c| *c == 0)
                .next()
                .map(|a| String::from_utf8_lossy(a).into_owned())
        });
    matcher::identities(exe.as_deref(), argv0.as_deref())
}

async fn sender_pid(conn: &zbus::Connection, sender: &str) -> Option<u32> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(sender).ok()?;
    dbus.get_connection_unix_process_id(name).await.ok()
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    async fn register_window(
        &self,
        window_id: u32,
        menu_object_path: OwnedObjectPath,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) {
        let Some(sender) = hdr.sender().map(|s| s.to_string()) else {
            return;
        };
        let pid = sender_pid(conn, &sender).await;
        let reg = Registration {
            window_id,
            sender,
            path: menu_object_path.to_string(),
            pid,
            identities: pid.map(proc_identities).unwrap_or_default(),
        };
        tracing::debug!(?reg, "global menu: window registered");
        if let Ok(mut r) = self.0.regs.lock() {
            r.insert(window_id, reg);
        }
        self.0.changed.notify_one();
    }

    async fn unregister_window(&self, window_id: u32) {
        if let Ok(mut r) = self.0.regs.lock() {
            r.remove(&window_id);
        }
        self.0.changed.notify_one();
    }

    async fn get_menu_for_window(
        &self,
        window_id: u32,
    ) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let reg = self
            .0
            .regs
            .lock()
            .ok()
            .and_then(|r| r.get(&window_id).cloned())
            .ok_or_else(|| zbus::fdo::Error::Failed("no menu for this window".into()))?;
        let path = OwnedObjectPath::try_from(reg.path)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok((reg.sender, path))
    }

    async fn get_menus(&self) -> Vec<(u32, String, OwnedObjectPath)> {
        self.0
            .regs
            .lock()
            .map(|r| {
                r.values()
                    .filter_map(|g| {
                        Some((
                            g.window_id,
                            g.sender.clone(),
                            OwnedObjectPath::try_from(g.path.clone()).ok()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Serve the registrar, or report that someone else does.
async fn start_registrar(conn: &zbus::Connection, shared: Arc<Shared>) -> RegistrarMode {
    if conn
        .object_server()
        .at(REGISTRAR_PATH, Registrar(shared))
        .await
        .is_err()
    {
        return RegistrarMode::Failed;
    }
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    match conn.request_name_with_flags(REGISTRAR_NAME, flags).await {
        Ok(
            zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner,
        ) => RegistrarMode::Own,
        Ok(_) | Err(zbus::Error::NameTaken) => RegistrarMode::External,
        Err(e) => {
            tracing::warn!(error = %e, "global menu: cannot own the AppMenu registrar name");
            RegistrarMode::External
        }
    }
}

async fn poll_external(conn: &zbus::Connection, shared: &Shared) {
    let Ok(p) = ExternalRegistrarProxy::new(conn).await else {
        return;
    };
    let Ok(menus) = p.get_menus().await else {
        return;
    };
    let mut fresh = BTreeMap::new();
    for (id, sender, path) in menus {
        let old = shared.regs.lock().ok().and_then(|r| r.get(&id).cloned());
        let reg = match old {
            Some(o) if o.sender == sender => o,
            _ => {
                let pid = sender_pid(conn, &sender).await;
                Registration {
                    window_id: id,
                    sender,
                    path: path.to_string(),
                    pid,
                    identities: pid.map(proc_identities).unwrap_or_default(),
                }
            }
        };
        fresh.insert(id, reg);
    }
    if let Ok(mut r) = shared.regs.lock()
        && *r != fresh
    {
        *r = fresh;
        shared.changed.notify_one();
    }
}

// ── dbusmenu client ─────────────────────────────────────────────────────

async fn menu_proxy(
    conn: &zbus::Connection,
    service: &(String, String),
) -> zbus::Result<DBusMenuProxy<'static>> {
    DBusMenuProxy::builder(conn)
        .destination(service.0.clone())?
        .path(service.1.clone())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

async fn layout(proxy: &DBusMenuProxy<'_>, parent: i32) -> Option<MenuItem> {
    match proxy.get_layout(parent, -1, Vec::new()).await {
        Ok(raw) => Some(MenuItem::from(raw)),
        Err(e) => {
            tracing::debug!(error = %e, "global menu: GetLayout failed");
            None
        }
    }
}

/// `AboutToShow` then a fresh layout of the submenu `id` (the popover).
pub(crate) async fn open_submenu(service: (String, String), id: i32) -> Option<MenuItem> {
    let conn = zbus::Connection::session().await.ok()?;
    let p = menu_proxy(&conn, &service).await.ok()?;
    let _ = p.about_to_show(id).await;
    layout(&p, id).await
}

fn now_ts() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

/// Send `Event(id, "clicked")` to the menu's owner.
pub(crate) fn activate(service: (String, String), id: i32) {
    mshell_services::tokio_rt_spawn(async move {
        let Ok(conn) = zbus::Connection::session().await else {
            return;
        };
        let Ok(p) = menu_proxy(&conn, &service).await else {
            return;
        };
        let data = OwnedValue::try_from(Value::from(0i32)).ok();
        if let Some(data) = data
            && let Err(e) = p.event(id, "clicked", data, now_ts()).await
        {
            tracing::warn!(error = %e, "global menu: Event failed");
        }
    });
}

// ── Owner task ──────────────────────────────────────────────────────────

fn regs_vec(shared: &Shared) -> Vec<Registration> {
    shared
        .regs
        .lock()
        .map(|r| r.values().cloned().collect())
        .unwrap_or_default()
}

async fn run(stop: CancellationToken) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "global menu: no session bus");
            STATE.send_replace(GlobalMenuState {
                enabled: true,
                mode: RegistrarMode::Failed,
                ..Default::default()
            });
            return;
        }
    };
    let shared = Arc::new(Shared::default());
    let mode = start_registrar(&conn, shared.clone()).await;
    tracing::info!(?mode, "global menu: registrar");

    let margo = mshell_services::margo_service();
    let mut focus = margo.focused_client.watch();
    let mut owners = match zbus::fdo::DBusProxy::new(&conn).await {
        Ok(p) => p.receive_name_owner_changed().await.ok(),
        Err(_) => None,
    };
    let mut poll = tokio::time::interval(Duration::from_secs(3));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut app_id = String::new();
    let mut pid: Option<u32> = None;
    let mut current: Option<(String, String)> = None;
    let mut updates: Option<LayoutUpdatedStream> = None;
    let mut menu: Option<Arc<MenuItem>> = None;

    loop {
        let mut refetch = false;
        tokio::select! {
            () = stop.cancelled() => break,
            c = focus.next() => {
                let Some(c) = c else { break };
                app_id = c.as_ref().map(|c| c.class.get()).unwrap_or_default();
                pid = c.as_ref().and_then(|c| u32::try_from(c.pid.get()).ok());
            }
            () = shared.changed.notified() => {}
            ev = async {
                match owners.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                // An app quit: drop its registrations.
                if let Some(ev) = ev
                    && let Ok(a) = ev.args()
                    && a.new_owner().is_none()
                {
                    let gone = a.name().to_string();
                    if let Ok(mut r) = shared.regs.lock() {
                        r.retain(|_, g| g.sender != gone);
                    }
                }
            }
            _ = poll.tick() => {
                if mode == RegistrarMode::External {
                    poll_external(&conn, &shared).await;
                }
                continue;
            }
            u = async {
                match updates.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if u.is_none() {
                    updates = None;
                }
                refetch = true;
            }
        }
        let regs = regs_vec(&shared);
        let picked = matcher::pick(&regs, &app_id, pid).map(|r| (r.sender.clone(), r.path.clone()));
        if picked != current {
            current = picked;
            updates = None;
            menu = None;
            refetch = true;
            if let Some(svc) = &current
                && let Ok(p) = menu_proxy(&conn, svc).await
            {
                updates = p.receive_layout_updated().await.ok();
            }
        }
        if refetch && let Some(svc) = &current {
            menu = match menu_proxy(&conn, svc).await {
                Ok(p) => layout(&p, 0).await.map(Arc::new),
                Err(_) => None,
            };
        }
        STATE.send_replace(GlobalMenuState {
            enabled: true,
            mode,
            app_id: app_id.clone(),
            service: current.clone(),
            menu: menu.clone(),
            registered: regs.len(),
        });
    }
    if mode == RegistrarMode::Own {
        let _ = conn.release_name(REGISTRAR_NAME).await;
    }
    let _ = conn
        .object_server()
        .remove::<Registrar, _>(REGISTRAR_PATH)
        .await;
    STATE.send_replace(GlobalMenuState::default());
}
