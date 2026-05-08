//! `org.gnome.Shell.Introspect` D-Bus shim.
//!
//! Direct port of niri/src/dbus/gnome_shell_introspect.rs.
//! xdp-gnome calls `GetWindows` here when the user clicks the
//! Window tab in the screencast chooser dialog — we return a
//! `HashMap<window_id, WindowProperties>` so the chooser knows
//! what's available.

use std::collections::HashMap;

use tracing::warn;
use zbus::fdo::{self, RequestNameFlags};
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{SerializeDict, Type, Value};

use super::Start;

pub struct Introspect {
    to_compositor: calloop::channel::Sender<IntrospectToCompositor>,
    from_compositor: async_channel::Receiver<CompositorToIntrospect>,
}

pub enum IntrospectToCompositor {
    GetWindows,
}

pub enum CompositorToIntrospect {
    Windows(HashMap<u64, WindowProperties>),
}

#[derive(Debug, SerializeDict, Type, Value)]
#[zvariant(signature = "dict")]
pub struct WindowProperties {
    /// Window title.
    pub title: String,
    /// Window app ID. Strictly the .desktop-file basename in
    /// gnome-shell terms; here we ship the wl_surface app_id
    /// directly because margo doesn't track .desktop mappings.
    /// Side effect: xdp-gnome's window list is missing icons —
    /// niri has the same caveat.
    #[zvariant(rename = "app-id")]
    pub app_id: String,
}

#[interface(name = "org.gnome.Shell.Introspect")]
impl Introspect {
    async fn get_windows(&self) -> fdo::Result<HashMap<u64, WindowProperties>> {
        if let Err(err) = self.to_compositor.send(IntrospectToCompositor::GetWindows) {
            warn!("error sending GetWindows to compositor: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        match self.from_compositor.recv().await {
            Ok(CompositorToIntrospect::Windows(windows)) => Ok(windows),
            Err(err) => {
                warn!("error receiving Windows from compositor: {err:?}");
                Err(fdo::Error::Failed("internal error".to_owned()))
            }
        }
    }

    // FIXME: emit on window changes once render-loop hooks land.
    #[zbus(signal)]
    pub async fn windows_changed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl Introspect {
    pub fn new(
        to_compositor: calloop::channel::Sender<IntrospectToCompositor>,
        from_compositor: async_channel::Receiver<CompositorToIntrospect>,
    ) -> Self {
        Self {
            to_compositor,
            from_compositor,
        }
    }
}

impl Start for Introspect {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Shell/Introspect", self)?;
        conn.request_name_with_flags("org.gnome.Shell.Introspect", flags)?;

        Ok(conn)
    }
}
