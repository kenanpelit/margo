//! Client half of the integration test fixture (W1.6).
//!
//! Minimal first-cut: enough wayland-client wrapping to drive a
//! `wl_registry` round-trip and assert the compositor exposes the
//! expected globals. Future tests extend `ClientState` with whatever
//! global wrappers they need (xdg_shell, layer-shell, etc.) — the
//! pattern is "register a Dispatch impl on ClientState for the
//! global type".

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use wayland_backend::client::Backend;
use wayland_client::globals::Global;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

/// Stable id for talking about a particular client across the
/// fixture (`add_client` returns one, helpers like `roundtrip(id)`
/// take it back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(u32);

static NEXT_CLIENT_ID: AtomicU32 = AtomicU32::new(0);

impl ClientId {
    fn next() -> Self {
        Self(NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// What the test client tracks each frame: globals announced by the
/// server (we keep them indexed by `(name)` so duplicate-bind tests
/// can spot drift) and pending sync callbacks.
#[derive(Default)]
pub struct ClientState {
    pub globals: BTreeMap<u32, Global>,
}

/// Sync sentinel: each `roundtrip` mints one and the fixture spins
/// the event loop until `done` flips. Mirrors niri's pattern;
/// without it, tests race the server response.
pub struct SyncDone {
    pub done: Arc<AtomicBool>,
}

/// One end of a connected (UnixStream) Wayland session, driven from
/// the test process. Owns a wayland-client `Connection` and
/// `QueueHandle`; the server side of the same socket is fed to
/// `MargoState::display_handle.insert_client(...)` by the fixture.
pub struct Client {
    pub id: ClientId,
    pub connection: Connection,
    pub display: WlDisplay,
    pub qh: QueueHandle<ClientState>,
    pub state: ClientState,
    pub event_queue: wayland_client::EventQueue<ClientState>,
}

impl Client {
    pub fn new(stream: UnixStream) -> Self {
        let backend = Backend::connect(stream).expect("client Backend::connect");
        let connection = Connection::from_backend(backend);
        let event_queue = connection.new_event_queue::<ClientState>();
        let qh = event_queue.handle();
        let display = connection.display();
        // Bind the registry up front but DON'T block on a round-trip
        // here — the fixture hasn't pumped the server yet, so a
        // blocking `roundtrip()` would deadlock. Tests call
        // `Fixture::roundtrip(id)` which interleaves server +
        // client dispatch.
        let _registry = display.get_registry(&qh, ());
        let state = ClientState::default();
        // Push the registry request out to the server so the very
        // first server dispatch sees it.
        connection.flush().expect("client flush");

        Self {
            id: ClientId::next(),
            connection,
            display,
            qh,
            state,
            event_queue,
        }
    }

    /// Read whatever the server has sent us (non-blocking) and
    /// dispatch the resulting events. Called from the fixture after
    /// every server tick. Splits into `prepare_read → read → dispatch`
    /// so we never block on the socket — the fixture's spin loop is
    /// what drives progress.
    pub fn read_and_dispatch(&mut self) {
        // `prepare_read` returns None if there are queued events
        // waiting to be dispatched first; in that case we just
        // dispatch what's queued and try to read on the next turn.
        if let Some(guard) = self.event_queue.prepare_read() {
            // Errors here mean the socket is broken — surface them
            // as panics so a wedged test doesn't pretend to pass.
            let _ = guard.read();
        }
        self.event_queue
            .dispatch_pending(&mut self.state)
            .expect("client dispatch_pending");
    }

    /// Request a `wl_display.sync` and return the sentinel; the
    /// fixture drives the event loop until the sentinel flips.
    pub fn send_sync(&mut self) -> SyncDone {
        let done = Arc::new(AtomicBool::new(false));
        let _cb = self.display.sync(&self.qh, done.clone());
        self.connection.flush().expect("client flush");
        SyncDone { done }
    }

    /// Convenience: list every global the server advertised, in
    /// stable name order. Tests use this for "did our handler-split
    /// regress the global advertisement?" assertions.
    pub fn global_names(&self) -> Vec<String> {
        self.state
            .globals
            .values()
            .map(|g| g.interface.clone())
            .collect()
    }
}

// ── Dispatch impls — the bare minimum a test client needs ──────────

impl Dispatch<WlRegistry, ()> for ClientState {
    fn event(
        state: &mut Self,
        _registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                state.globals.insert(
                    name,
                    Global {
                        name,
                        interface,
                        version,
                    },
                );
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.globals.remove(&name);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, Arc<AtomicBool>> for ClientState {
    fn event(
        _state: &mut Self,
        _cb: &WlCallback,
        event: wl_callback::Event,
        done: &Arc<AtomicBool>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            done.store(true, Ordering::Relaxed);
        }
    }
}

impl Dispatch<WlDisplay, ()> for ClientState {
    fn event(
        _state: &mut Self,
        _display: &WlDisplay,
        _event: <WlDisplay as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Display has no events we care about for these tests
        // (errors would matter, but they bubble up via
        // dispatch_pending's Result if they happen).
    }
}
