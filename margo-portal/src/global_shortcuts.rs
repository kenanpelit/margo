//! `org.freedesktop.impl.portal.GlobalShortcuts` — app-registered
//! global hotkeys (Discord push-to-talk, OBS record toggles) without
//! the app holding a keyboard grab.
//!
//! Split of responsibilities: this daemon owns the D-Bus surface
//! (sessions, BindShortcuts/ListShortcuts, Activated/Deactivated
//! signals); margo owns the actual keys. Registration goes to the
//! compositor over the `MARGO_SOCKET` control socket
//! (`dispatch global_shortcuts_bind …`), activation events come back
//! on a persistent `watch shortcuts` connection and are re-emitted
//! here as portal signals. Ids/triggers are percent-encoded on the
//! wire so the line protocol stays whitespace/comma-clean.
//!
//! Config binds always win inside margo — an app can never shadow a
//! user keybind.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info, warn};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_ERROR: u32 = 2;

/// One bound shortcut as the portal spec models it.
#[derive(Clone)]
struct Shortcut {
    id: String,
    description: String,
    trigger: String,
}

#[derive(Default)]
struct SessionState {
    shortcuts: Vec<Shortcut>,
}

type Sessions = Arc<Mutex<HashMap<String, SessionState>>>;

pub struct GlobalShortcutsBackend {
    sessions: Sessions,
}

impl GlobalShortcutsBackend {
    pub fn new() -> Self {
        Self {
            sessions: Sessions::default(),
        }
    }
}

/// Percent-encode everything outside `[A-Za-z0-9_.-]` so ids and
/// triggers survive margo's whitespace/comma-split line protocol.
/// (`+` in triggers is encoded too; margo decodes before parsing.)
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn as_str(v: &OwnedValue) -> Option<String> {
    String::try_from(v.clone()).ok()
}

/// Fire-and-forget one dispatch line at margo's control socket.
async fn margo_dispatch(line: String) {
    let Some(path) = std::env::var_os("MARGO_SOCKET") else {
        warn!("MARGO_SOCKET unset — global shortcut not delivered to compositor");
        return;
    };
    match tokio::net::UnixStream::connect(&path).await {
        Ok(mut stream) => {
            if let Err(e) = stream.write_all(line.as_bytes()).await {
                warn!(error = %e, "margo dispatch write failed");
            }
        }
        Err(e) => warn!(error = %e, "margo socket connect failed"),
    }
}

/// Wire format for one session's bind line.
fn bind_line(session: &str, shortcuts: &[Shortcut]) -> String {
    let entries: Vec<String> = shortcuts
        .iter()
        .map(|s| format!("{}:{}", enc(&s.id), enc(&s.trigger)))
        .collect();
    format!(
        "dispatch global_shortcuts_bind {} {}\n",
        enc(session),
        entries.join(",")
    )
}

/// Portal-shaped `a(sa{sv})` result payload for a session's shortcuts.
fn shortcuts_value(shortcuts: &[Shortcut]) -> Option<OwnedValue> {
    let vec: Vec<(String, HashMap<String, Value<'_>>)> = shortcuts
        .iter()
        .map(|s| {
            let mut props: HashMap<String, Value<'_>> = HashMap::new();
            props.insert("description".into(), Value::from(s.description.clone()));
            props.insert("trigger_description".into(), Value::from(s.trigger.clone()));
            (s.id.clone(), props)
        })
        .collect();
    Value::new(vec).try_to_owned().ok()
}

/// Parse the frontend's `a(sa{sv})` shortcut list.
fn parse_shortcuts(raw: Vec<(String, HashMap<String, OwnedValue>)>) -> Vec<Shortcut> {
    raw.into_iter()
        .map(|(id, props)| Shortcut {
            id,
            description: props
                .get("description")
                .and_then(as_str)
                .unwrap_or_default(),
            trigger: props
                .get("preferred_trigger")
                .and_then(as_str)
                .unwrap_or_default(),
        })
        .collect()
}

#[interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcutsBackend {
    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        1
    }

    async fn create_session(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        // The spec allows binding at create time via options["shortcuts"].
        let initial: Vec<Shortcut> = options
            .get("shortcuts")
            .and_then(|v| <Vec<(String, HashMap<String, OwnedValue>)>>::try_from(v.clone()).ok())
            .map(parse_shortcuts)
            .unwrap_or_default();
        if !initial.is_empty() {
            margo_dispatch(bind_line(&key, &initial)).await;
        }
        self.sessions
            .lock()
            .unwrap()
            .insert(key.clone(), SessionState { shortcuts: initial });

        let session_obj = ShortcutsSession {
            handle: key.clone(),
            sessions: self.sessions.clone(),
        };
        if let Err(e) = server.at(session_handle.as_ref(), session_obj).await {
            warn!(error = %e, "create_session: failed to export Session object");
            self.sessions.lock().unwrap().remove(&key);
            return (RESPONSE_ERROR, HashMap::new());
        }
        info!(session = %key, app = %app_id, "global-shortcuts session created");
        (RESPONSE_SUCCESS, HashMap::new())
    }

    async fn bind_shortcuts(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        let parsed = parse_shortcuts(shortcuts);
        debug!(session = %key, count = parsed.len(), "bind_shortcuts");
        margo_dispatch(bind_line(&key, &parsed)).await;

        let mut results = HashMap::new();
        if let Some(v) = shortcuts_value(&parsed) {
            results.insert("shortcuts".to_string(), v);
        }
        match self.sessions.lock().unwrap().get_mut(&key) {
            Some(sess) => sess.shortcuts = parsed,
            None => return (RESPONSE_ERROR, HashMap::new()),
        }
        (RESPONSE_SUCCESS, results)
    }

    async fn list_shortcuts(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        let mut results = HashMap::new();
        if let Some(v) = self
            .sessions
            .lock()
            .unwrap()
            .get(&key)
            .and_then(|sess| shortcuts_value(&sess.shortcuts))
        {
            results.insert("shortcuts".to_string(), v);
        }
        (RESPONSE_SUCCESS, results)
    }

    #[zbus(signal)]
    pub async fn activated(
        emitter: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
}

/// Minimal `org.freedesktop.impl.portal.Session`; Close unbinds the
/// compositor-side registration.
struct ShortcutsSession {
    handle: String,
    sessions: Sessions,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl ShortcutsSession {
    async fn close(&self) {
        self.sessions.lock().unwrap().remove(&self.handle);
        margo_dispatch(format!(
            "dispatch global_shortcuts_unbind {}\n",
            enc(&self.handle)
        ))
        .await;
        debug!(session = %self.handle, "global-shortcuts session closed");
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Long-lived `watch shortcuts` listener: reconnects with backoff so
/// a compositor restart doesn't strand the portal, and re-emits every
/// activation frame as the matching portal signal.
pub async fn run_event_listener(conn: Connection, object_path: &'static str) {
    loop {
        let Some(path) = std::env::var_os("MARGO_SOCKET") else {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            continue;
        };
        match tokio::net::UnixStream::connect(&path).await {
            Ok(mut stream) => {
                if stream.write_all(b"watch shortcuts\n").await.is_err() {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
                info!("global-shortcuts: watching margo activation stream");
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    let Some(sc) = v.get("shortcut") else {
                        continue; // initial summary frame
                    };
                    let session = sc.get("session").and_then(|s| s.as_str()).unwrap_or("");
                    let id = sc.get("id").and_then(|s| s.as_str()).unwrap_or("");
                    let activated = sc.get("state").and_then(|s| s.as_str()) == Some("activated");
                    let ts = sc.get("timestamp_ms").and_then(|t| t.as_u64()).unwrap_or(0);
                    let Ok(session_path) = ObjectPath::try_from(session.to_string()) else {
                        continue;
                    };
                    let Ok(emitter) = SignalEmitter::new(&conn, object_path) else {
                        continue;
                    };
                    let res = if activated {
                        GlobalShortcutsBackend::activated(
                            &emitter,
                            session_path,
                            id,
                            ts,
                            HashMap::new(),
                        )
                        .await
                    } else {
                        GlobalShortcutsBackend::deactivated(
                            &emitter,
                            session_path,
                            id,
                            ts,
                            HashMap::new(),
                        )
                        .await
                    };
                    if let Err(e) = res {
                        warn!(error = %e, "global-shortcuts signal emission failed");
                    }
                }
                warn!("global-shortcuts: margo socket stream ended, reconnecting");
            }
            Err(e) => {
                debug!(error = %e, "global-shortcuts: margo socket unavailable, retrying");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}
