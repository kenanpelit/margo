#![allow(dead_code)]
//! `org.gnome.Mutter.ServiceChannel` D-Bus shim.
//!
//! Direct port of niri/src/dbus/mutter_service_channel.rs. Used by
//! xdg-desktop-portal-gnome to open a Wayland connection during
//! `RequestSession` / `Authenticate` handshake. Without this
//! interface registered xdp-gnome falls back to the
//! "no Mutter ServiceChannel" code path which then refuses to
//! continue the screencast handshake.
//!
//! The compositor side handles the inbound socket via
//! `MargoState::insert_client` (or equivalent) — same shape as
//! niri's `NewClient` channel.

use std::os::unix::net::UnixStream;

use tracing::warn;
use zbus::{fdo, interface, zvariant};

use super::Start;

/// One side of an open service-channel handshake. The D-Bus side
/// keeps `client` (the compositor's end of the socketpair) and
/// hands the other end back to xdp-gnome over D-Bus. The
/// compositor inserts `client` into its Wayland display so the
/// portal can be its own client.
pub struct NewClient {
    pub client: UnixStream,
    pub restricted: bool,
    pub credentials_unknown: bool,
}

pub struct ServiceChannel {
    to_compositor: calloop::channel::Sender<NewClient>,
}

#[interface(name = "org.gnome.Mutter.ServiceChannel")]
impl ServiceChannel {
    async fn open_wayland_service_connection(
        &mut self,
        service_client_type: u32,
    ) -> fdo::Result<zvariant::OwnedFd> {
        if service_client_type != 1 {
            return Err(fdo::Error::InvalidArgs(
                "Invalid service client type".to_owned(),
            ));
        }

        let (sock1, sock2) = UnixStream::pair().unwrap();
        let client = NewClient {
            client: sock2,
            restricted: false,
            // FIXME: maybe you can get the PID from D-Bus somehow?
            credentials_unknown: true,
        };
        if let Err(err) = self.to_compositor.send(client) {
            warn!("error sending NewClient to compositor: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        Ok(zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(sock1)))
    }
}

impl ServiceChannel {
    pub fn new(to_compositor: calloop::channel::Sender<NewClient>) -> Self {
        Self { to_compositor }
    }
}

impl Start for ServiceChannel {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::connection::Builder::session()?
            .name("org.gnome.Mutter.ServiceChannel")?
            .serve_at("/org/gnome/Mutter/ServiceChannel", self)?
            .build()?;
        Ok(conn)
    }
}
