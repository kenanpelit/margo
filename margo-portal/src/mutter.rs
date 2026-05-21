//! Proxies onto margo's own `org.gnome.Mutter.ScreenCast` /
//! `org.gnome.Shell.Introspect` D-Bus shims (served by the compositor,
//! see `margo/src/dbus/`). The portal drives these to do the actual
//! per-window / per-output PipeWire capture — margo already implements
//! the capture + stream, we only orchestrate it and hand the resulting
//! PipeWire node id back to xdg-desktop-portal.

use std::collections::HashMap;
use zbus::proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// `org.gnome.Mutter.ScreenCast` — the capture session factory.
#[proxy(
    interface = "org.gnome.Mutter.ScreenCast",
    default_service = "org.gnome.Mutter.ScreenCast",
    default_path = "/org/gnome/Mutter/ScreenCast"
)]
pub trait MutterScreenCast {
    /// Mint a capture session; returns its object path.
    fn create_session(
        &self,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn version(&self) -> zbus::Result<i32>;
}

/// `org.gnome.Mutter.ScreenCast.Session` — bound to a dynamic path
/// returned by `create_session`.
#[proxy(
    interface = "org.gnome.Mutter.ScreenCast.Session",
    default_service = "org.gnome.Mutter.ScreenCast"
)]
pub trait MutterSession {
    /// Capture a whole output by connector name. Returns the Stream path.
    fn record_monitor(
        &self,
        connector: &str,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Capture one window (`window-id` in `properties`). Returns the
    /// Stream path.
    fn record_window(
        &self,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn closed(&self) -> zbus::Result<()>;
}

/// `org.gnome.Mutter.ScreenCast.Stream` — bound to a dynamic path.
#[proxy(
    interface = "org.gnome.Mutter.ScreenCast.Stream",
    default_service = "org.gnome.Mutter.ScreenCast"
)]
pub trait MutterStream {
    /// Fires once the PipeWire node backing this stream exists. The
    /// `node_id` is what we return to the portal frontend.
    #[zbus(signal)]
    fn pipe_wire_stream_added(&self, node_id: u32) -> zbus::Result<()>;
}

/// `org.gnome.Shell.Introspect` — window enumeration for the picker.
#[proxy(
    interface = "org.gnome.Shell.Introspect",
    default_service = "org.gnome.Shell.Introspect",
    default_path = "/org/gnome/Shell/Introspect"
)]
pub trait Introspect {
    /// `window-id → { "app-id": s, "title": s, … }`.
    fn get_windows(&self) -> zbus::Result<HashMap<u64, HashMap<String, OwnedValue>>>;
}

/// `com.mshell.Shell` — the desktop shell. We call its `Screenshare`
/// picker (the DESIGN.md-styled source chooser) so the user gets the
/// same UI the rest of margo uses. Blocks until the user picks; returns
/// `[SELECTION]/window:<id>` | `/screen:<connector>` | `` (cancel).
#[proxy(
    interface = "com.mshell.Shell",
    default_service = "com.mshell.Shell",
    default_path = "/com/mshell/Shell"
)]
pub trait Shell {
    fn screenshare(&self, payload: &str) -> zbus::Result<String>;
}
