//! Top-level integration test fixture (W1.6).
//!
//! Drives a [`super::server::Server`] (real `MargoState`) and zero
//! or more [`super::client::Client`]s connected over `UnixStream`
//! pairs. Tests use the helper API:
//!
//! ```ignore
//! let mut fx = Fixture::new();
//! let id = fx.add_client();
//! fx.roundtrip(id);
//! assert!(fx.client(id).global_names().contains(&"wl_compositor".into()));
//! ```
//!
//! # Why this isn't a unit test
//!
//! Unit-testing the protocol handlers in isolation tells you they
//! compile. Driving them through a real Wayland connection — where
//! `wl_registry.bind`, `wl_display.sync`, and the server-side
//! dispatch order all matter — tells you they *work together*. niri
//! ships ~5,280 snapshot fixtures *and* a fixture harness like this
//! because the two catch different classes of bug. Margo had only
//! the snapshot half until W1.6.
//!
//! # Synchronisation model
//!
//! A naive `dispatch()` once isn't enough: client requests are read
//! into the server, but server responses (configure events,
//! global-bind acks) sit in the server's send buffer until
//! `flush_clients()`. The client then has to dispatch its own
//! events to observe them. `roundtrip(id)` does this: it sends a
//! `wl_display.sync`, then loops `server.dispatch()` +
//! `client.dispatch_pending()` until the sync `done` event arrives
//! on the client.

use std::os::unix::net::UnixStream;
use std::sync::Arc;

use margo_config::Config;

use super::client::{Client, ClientId};
use super::server::Server;
use crate::state::MargoClientData;

pub struct Fixture {
    pub server: Server,
    pub clients: Vec<Client>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    pub fn with_config(config: Config) -> Self {
        Self {
            server: Server::new(config),
            clients: Vec::new(),
        }
    }

    pub fn dispatch(&mut self) {
        self.server.dispatch();
        for client in &mut self.clients {
            client.read_and_dispatch();
        }
    }

    /// Add a new wayland-client-side `Client` connected to the
    /// running `Server` over a fresh `UnixStream` pair. Returns the
    /// stable [`ClientId`] used by other helpers.
    pub fn add_client(&mut self) -> ClientId {
        let (server_side, client_side) =
            UnixStream::pair().expect("UnixStream::pair");
        // Server registers our peer as a regular Wayland client —
        // `MargoClientData::default()` matches what main.rs hands
        // freshly-connected clients from the listening socket.
        self.server
            .state
            .display_handle
            .insert_client(server_side, Arc::new(MargoClientData::default()))
            .expect("insert_client");

        // Client::new emits `get_registry` and flushes; the round-
        // trip below interleaves server + client dispatch until the
        // global advertisement bursts have been consumed.
        let client = Client::new(client_side);
        let id = client.id;
        self.clients.push(client);
        self.roundtrip(id);
        id
    }

    pub fn client(&mut self, id: ClientId) -> &mut Client {
        self.clients
            .iter_mut()
            .find(|c| c.id == id)
            .expect("client by id")
    }

    /// Round-trip via `wl_display.sync` — drive the loop until the
    /// callback fires on the client. Cap iterations to avoid
    /// hanging a broken test forever.
    pub fn roundtrip(&mut self, id: ClientId) {
        let sync = self.client(id).send_sync();
        self.spin_until_sync_done(&sync.done);
    }

    fn spin_until_sync_done(&mut self, done: &Arc<std::sync::atomic::AtomicBool>) {
        // 200 turns at zero-timeout each is a generous upper bound
        // for any test that doesn't need a real timer to fire.
        // Hitting it means the round-trip got wedged — an assertion
        // beats a hang.
        for _ in 0..200 {
            if done.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            self.dispatch();
        }
        panic!("roundtrip timed out after 200 dispatch turns");
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
