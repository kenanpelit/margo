//! `org.freedesktop.impl.portal.ScreenCast` backend.
//!
//! Implements the impl-portal ScreenCast contract that
//! xdg-desktop-portal's frontend calls, and fulfils it by driving
//! margo's own `org.gnome.Mutter.ScreenCast` shim (see [`crate::mutter`])
//! — which already owns the PipeWire stream + per-window/per-output
//! capture. We return the PipeWire node id; the portal frontend opens
//! the PipeWire remote itself, so this crate links no PipeWire code.
//!
//! Flow per session:
//!   CreateSession  → export an impl.portal.Session object.
//!   SelectSources  → remember requested source types + cursor mode.
//!   Start          → pick a source, drive the Mutter shim
//!                    (CreateSession → RecordWindow/RecordMonitor →
//!                    PipeWireStreamAdded → Session.Start), return the
//!                    node id in `streams`.

use crate::mutter::{MutterScreenCastProxy, MutterSessionProxy, MutterStreamProxy};
use crate::picker::{self, Source};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

/// Portal `response` codes (org.freedesktop.impl.portal.Request).
const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_ERROR: u32 = 2;

/// SourceType bitmask (org.freedesktop.impl.portal.ScreenCast).
const SOURCE_MONITOR: u32 = 1;
const SOURCE_WINDOW: u32 = 2;
/// CursorMode bitmask.
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
const CURSOR_METADATA: u32 = 4;

/// Per-portal-session state, keyed by the frontend's session handle.
#[derive(Default)]
struct SessionState {
    /// SourceTypes the app asked for (monitor / window bitmask).
    requested_types: u32,
    /// Cursor mode the app asked for.
    cursor_mode: u32,
}

pub struct ScreenCastBackend {
    /// Session-bus connection (used to reach margo's Mutter shim).
    conn: Connection,
    sessions: Mutex<HashMap<String, SessionState>>,
}

impl ScreenCastBackend {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Drive margo's Mutter shim to start capturing `source`, returning
    /// the PipeWire node id once the stream is live.
    async fn capture(&self, source: &Source, cursor_mode: u32) -> anyhow::Result<u32> {
        let sc = MutterScreenCastProxy::new(&self.conn).await?;
        let mutter_session_path = sc.create_session(HashMap::new()).await?;
        let session = MutterSessionProxy::builder(&self.conn)
            .path(mutter_session_path.clone())?
            .build()
            .await?;

        // Cursor mode → Mutter's enum (Hidden=0, Embedded=1, Metadata=2).
        let mutter_cursor: u32 = if cursor_mode & CURSOR_METADATA != 0 {
            2
        } else if cursor_mode & CURSOR_EMBEDDED != 0 {
            1
        } else {
            0
        };
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("cursor-mode", Value::from(mutter_cursor));

        let stream_path: OwnedObjectPath = match source {
            Source::Window { id } => {
                let mut p = props.clone();
                p.insert("window-id", Value::from(*id));
                session.record_window(p).await?
            }
            Source::Monitor { connector } => session.record_monitor(connector, props).await?,
        };

        // Subscribe to the node-added signal *before* starting so we
        // don't miss it.
        let stream = MutterStreamProxy::builder(&self.conn)
            .path(stream_path)?
            .build()
            .await?;
        let mut added = stream.receive_pipe_wire_stream_added().await?;

        session.start().await?;

        use futures_util::StreamExt as _;
        let node_id = match added.next().await {
            Some(sig) => sig.args()?.node_id,
            None => anyhow::bail!("Mutter stream closed before PipeWireStreamAdded"),
        };
        info!(node_id, ?source, "margo-portal: capture started");
        Ok(node_id)
    }
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCastBackend {
    #[zbus(property, name = "AvailableSourceTypes")]
    async fn available_source_types(&self) -> u32 {
        SOURCE_MONITOR | SOURCE_WINDOW
    }

    #[zbus(property, name = "AvailableCursorModes")]
    async fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED | CURSOR_METADATA
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        // Match the gnome backend's advertised version so meeting
        // clients negotiate the same source-type tabs.
        4
    }

    async fn create_session(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        self.sessions
            .lock()
            .unwrap()
            .insert(key.clone(), SessionState::default());

        // Export a minimal impl.portal.Session object so the frontend
        // can Close it.
        let session_obj = PortalSession {
            handle: key.clone(),
        };
        if let Err(e) = server.at(session_handle.as_ref(), session_obj).await {
            warn!(error = %e, "create_session: failed to export Session object");
            self.sessions.lock().unwrap().remove(&key);
            return (RESPONSE_ERROR, HashMap::new());
        }
        debug!(session = %key, "create_session");
        (RESPONSE_SUCCESS, HashMap::new())
    }

    async fn select_sources(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        let types = options
            .get("types")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(SOURCE_MONITOR | SOURCE_WINDOW);
        let cursor = options
            .get("cursor_mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(CURSOR_HIDDEN);

        if let Some(state) = self.sessions.lock().unwrap().get_mut(&key) {
            state.requested_types = types;
            state.cursor_mode = cursor;
        }
        debug!(session = %key, types, cursor, "select_sources");
        (RESPONSE_SUCCESS, HashMap::new())
    }

    async fn start(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let key = session_handle.to_string();
        let (types, cursor) = {
            let map = self.sessions.lock().unwrap();
            let s = map.get(&key);
            (
                s.map(|s| s.requested_types).unwrap_or(SOURCE_WINDOW),
                s.map(|s| s.cursor_mode).unwrap_or(CURSOR_HIDDEN),
            )
        };

        // Ask the user which window / output to share.
        let source = match picker::pick(&self.conn, types).await {
            Ok(Some(src)) => src,
            Ok(None) => {
                info!(session = %key, "start: user cancelled source pick");
                return (RESPONSE_CANCELLED, HashMap::new());
            }
            Err(e) => {
                warn!(error = %e, "start: source pick failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };

        let node_id = match self.capture(&source, cursor).await {
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, "start: capture failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };

        // results = { streams: a(ua{sv}) } — one (node_id, props).
        let mut stream_props: HashMap<String, OwnedValue> = HashMap::new();
        stream_props.insert("source_type".into(), owned(source.source_type()));
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> = vec![(node_id, stream_props)];

        let mut results: HashMap<String, OwnedValue> = HashMap::new();
        match Value::from(streams).try_to_owned() {
            Ok(v) => {
                results.insert("streams".into(), v);
            }
            Err(e) => {
                warn!(error = %e, "start: failed to encode streams");
                return (RESPONSE_ERROR, HashMap::new());
            }
        }
        (RESPONSE_SUCCESS, results)
    }
}

/// Minimal `org.freedesktop.impl.portal.Session` so the frontend can
/// close the session.
struct PortalSession {
    handle: String,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl PortalSession {
    async fn close(&self) {
        debug!(session = %self.handle, "session closed");
    }

    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// `u32` → `OwnedValue` (infallible for a scalar).
fn owned(v: u32) -> OwnedValue {
    Value::from(v).try_to_owned().expect("u32 → OwnedValue")
}
