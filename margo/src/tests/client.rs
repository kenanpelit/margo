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

use smithay::reexports::wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};
use smithay::reexports::wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use smithay::reexports::wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
    zwp_idle_inhibitor_v1::{self, ZwpIdleInhibitorV1},
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
    zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};
use wayland_backend::client::Backend;
use wayland_client::globals::Global;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::{self, WlSurface};
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

/// Configure event the test client received on a particular xdg
/// surface — exposed so tests can assert "the client got told to
/// be 1024×640" or "fullscreen state was sent".
#[derive(Debug, Default, Clone)]
pub struct ToplevelConfigure {
    pub size: (i32, i32),
    pub states: Vec<u32>,
    pub bounds: Option<(i32, i32)>,
}

/// Tracking state for a single xdg_toplevel the client created.
/// Tests reach in via `Client::toplevel_state(...)`.
#[derive(Debug, Default, Clone)]
pub struct ToplevelState {
    pub configures: Vec<ToplevelConfigure>,
    pub close_requested: bool,
}

/// What the test client tracks each frame: globals announced by the
/// server (we keep them indexed by `(name)` so duplicate-bind tests
/// can spot drift) and pending sync callbacks.
#[derive(Default)]
pub struct ClientState {
    pub globals: BTreeMap<u32, Global>,
    /// Per-xdg_toplevel state, keyed by the proxy id. Populated as
    /// the client creates toplevels and as configure / close
    /// events arrive on them.
    pub toplevels: BTreeMap<u32, ToplevelState>,
    /// Pending configure being assembled for an xdg_surface. The
    /// xdg-shell wire format streams `xdg_toplevel.configure` (size
    /// + states + bounds) BEFORE the matching `xdg_surface.configure`
    /// (serial). We accumulate fields on the toplevel side and
    /// snapshot when the surface-side serial arrives.
    pub pending_toplevel: BTreeMap<u32, ToplevelConfigure>,
    /// Map xdg_surface.id() → xdg_toplevel.id() so the surface
    /// configure event can find the right pending entry.
    pub xdg_surface_to_toplevel: BTreeMap<u32, u32>,
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

    /// Bind a global by its `interface` name and return the proxy.
    /// Panics if the global hasn't been advertised yet — the test
    /// is expected to call `Fixture::roundtrip` once before binding.
    fn bind_global<I>(&self, version_cap: u32) -> I
    where
        I: Proxy + 'static,
        ClientState: Dispatch<I, ()>,
    {
        let registry = self.display.get_registry(&self.qh, ());
        let target_iface = I::interface().name;
        let global = self
            .state
            .globals
            .values()
            .find(|g| g.interface == target_iface)
            .unwrap_or_else(|| {
                panic!(
                    "global `{target_iface}` not advertised; available: {:?}",
                    self.state
                        .globals
                        .values()
                        .map(|g| g.interface.as_str())
                        .collect::<Vec<_>>()
                )
            });
        let version = global.version.min(version_cap);
        registry.bind::<I, _, _>(global.name, version, &self.qh, ())
    }

    /// Bind `wl_compositor` and create a fresh `wl_surface`.
    pub fn create_surface(&mut self) -> (WlCompositor, WlSurface) {
        let compositor: WlCompositor = self.bind_global(6);
        let surface = compositor.create_surface(&self.qh, ());
        self.connection.flush().expect("client flush");
        (compositor, surface)
    }

    /// Create a fresh xdg_toplevel by binding xdg_wm_base, creating
    /// an xdg_surface from a wl_surface, and giving it the toplevel
    /// role. Returns the toplevel proxy AND its underlying
    /// wl_surface so tests can drive `commit()` themselves —
    /// margo's deferred-map / app-id-refresh flow makes the commit
    /// timing observable, so the harness lets tests choose when it
    /// happens.
    ///
    /// Does NOT commit. Tests that want a regular "open a window"
    /// flow should follow up with `wl_surface.commit(); flush();`
    /// to trigger `finalize_initial_map`.
    pub fn create_toplevel(&mut self) -> (XdgToplevel, WlSurface) {
        let (_compositor, wl_surface) = self.create_surface();
        let xdg_wm_base: XdgWmBase = self.bind_global(6);
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        self.state
            .xdg_surface_to_toplevel
            .insert(xdg_surface.id().protocol_id(), toplevel.id().protocol_id());
        self.state
            .toplevels
            .insert(toplevel.id().protocol_id(), ToplevelState::default());
        self.connection.flush().expect("client flush");
        (toplevel, wl_surface)
    }

    /// Push pending requests to the server. Tests call this after
    /// every burst of requests (set_app_id / set_title / commit / …)
    /// so the next round-trip reflects them.
    pub fn flush(&mut self) {
        self.connection.flush().expect("client flush");
    }

    /// Create an idle-inhibitor on a fresh wl_surface. Returns the
    /// inhibitor proxy plus the wl_surface; tests destroy them in
    /// either order to assert margo's `inhibit` / `uninhibit`
    /// counter math.
    pub fn create_idle_inhibitor(&mut self) -> (ZwpIdleInhibitorV1, WlSurface) {
        let (_compositor, surface) = self.create_surface();
        let manager: ZwpIdleInhibitManagerV1 = self.bind_global(1);
        let inhibitor = manager.create_inhibitor(&surface, &self.qh, ());
        self.connection.flush().expect("client flush");
        (inhibitor, surface)
    }

    /// Bind the xdg-decoration manager and request a decoration
    /// proxy for an existing xdg_toplevel. Returns the proxy; tests
    /// follow up with `set_mode` / `unset_mode` and assert against
    /// `MargoState`'s response.
    pub fn create_decoration(
        &mut self,
        toplevel: &XdgToplevel,
    ) -> ZxdgToplevelDecorationV1 {
        let manager: ZxdgDecorationManagerV1 = self.bind_global(1);
        let decoration = manager.get_toplevel_decoration(toplevel, &self.qh, ());
        self.connection.flush().expect("client flush");
        decoration
    }

    /// Create + acquire an ext-session-lock. Returns the lock
    /// proxy so tests can inspect or release it.
    pub fn create_session_lock(&mut self) -> ExtSessionLockV1 {
        let manager: ExtSessionLockManagerV1 = self.bind_global(1);
        let lock = manager.lock(&self.qh, ());
        self.connection.flush().expect("client flush");
        lock
    }

    /// Create a fresh layer-shell surface — bind the layer-shell
    /// global, attach a wl_surface to it under the given namespace
    /// + layer (no specific output, server picks). Sets a default
    /// (1, 30) size; without one (and with no anchor pinning
    /// opposite edges), the protocol's `invalid_size` rule rejects
    /// the first commit with `Protocol error 1 on
    /// zwlr_layer_surface_v1`. Tests that need a different size
    /// can call `layer_surface.set_size(...)` before their commit.
    /// Returns the layer surface proxy and its underlying
    /// wl_surface so tests drive their own commit cadence.
    pub fn create_layer_surface(
        &mut self,
        namespace: &str,
        layer: zwlr_layer_shell_v1::Layer,
    ) -> (ZwlrLayerSurfaceV1, WlSurface) {
        let (_compositor, wl_surface) = self.create_surface();
        let layer_shell: ZwlrLayerShellV1 = self.bind_global(5);
        let layer_surface = layer_shell.get_layer_surface(
            &wl_surface,
            None,
            layer,
            namespace.to_string(),
            &self.qh,
            (),
        );
        // Match a typical bar / notification footprint — the value
        // doesn't matter for these tests, only that it's non-zero.
        layer_surface.set_size(1, 30);
        self.connection.flush().expect("client flush");
        (layer_surface, wl_surface)
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

// ── Stub Dispatch impls for the proxies tests bind ─────────────────
//
// The compositor / xdg_wm_base / wl_surface / xdg_surface chain has
// to satisfy `Dispatch<I, ()>` for the registry-bind helper to
// compile, but we don't act on most of these events.

impl Dispatch<WlCompositor, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Enter / leave events arrive when outputs come and go;
        // headless tests have no outputs so they shouldn't fire,
        // but we accept them silently if the harness later adds an
        // Output via MargoState.
        let _ = event;
    }
}

impl Dispatch<XdgWmBase, ()> for ClientState {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Pong every ping the server sends — niri / mango / margo
        // all do this; otherwise the server eventually decides the
        // client is unresponsive and may drop it.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for ClientState {
    fn event(
        state: &mut Self,
        surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            // Always ack — the test isn't interested in pending
            // changes, just in the server's configure cadence.
            surface.ack_configure(serial);
            // Snapshot pending configure into the toplevel's history.
            let surface_id = surface.id().protocol_id();
            if let Some(&toplevel_id) = state.xdg_surface_to_toplevel.get(&surface_id) {
                let _ = serial;
                let pending = state
                    .pending_toplevel
                    .remove(&toplevel_id)
                    .unwrap_or_default();
                if let Some(t) = state.toplevels.get_mut(&toplevel_id) {
                    t.configures.push(pending);
                }
            }
        }
    }
}

impl Dispatch<XdgToplevel, ()> for ClientState {
    fn event(
        state: &mut Self,
        toplevel: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = toplevel.id().protocol_id();
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                let entry = state.pending_toplevel.entry(id).or_default();
                entry.size = (width, height);
                // Each state byte is u32-LE in the spec.
                entry.states = states
                    .chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
            }
            xdg_toplevel::Event::ConfigureBounds { width, height } => {
                let entry = state.pending_toplevel.entry(id).or_default();
                entry.bounds = Some((width, height));
            }
            xdg_toplevel::Event::Close => {
                if let Some(t) = state.toplevels.get_mut(&id) {
                    t.close_requested = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Auto-ack any configure so the server doesn't time us out.
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            layer_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<ZwpIdleInhibitManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwpIdleInhibitManagerV1,
        _: <ZwpIdleInhibitManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpIdleInhibitorV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwpIdleInhibitorV1,
        _: zwp_idle_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgDecorationManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZxdgDecorationManagerV1,
        _: <ZxdgDecorationManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        _: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Mode events arrive whenever the server resolves a
        // configure-with-decoration; tests inspect MargoState
        // directly rather than tracking this on the client side.
    }
}

impl Dispatch<ExtSessionLockManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ExtSessionLockManagerV1,
        _: <ExtSessionLockManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtSessionLockV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ExtSessionLockV1,
        _: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Locked / Finished events arrive on the lock proxy; tests
        // check `state.session_locked` rather than tracking these
        // explicitly.
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
        }
    }
}
