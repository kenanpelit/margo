//! D-Bus service shims so `xdg-desktop-portal-gnome` can serve the
//! ScreenCast / Screenshot / RemoteDesktop portals on margo
//! sessions without needing gnome-shell.
//!
//! ## Why this exists
//!
//! Browser meeting clients (Helium, Chromium-based, Firefox) only
//! light up the **Window** and **Entire Screen** tabs in their
//! share dialog when xdg-desktop-portal serves a working ScreenCast
//! interface. xdp-wlr advertises ext-image-copy-capture but
//! Chromium's screen-share UI doesn't enable Window/Screen selection
//! against the wlr backend — it only does so against the gnome
//! backend.
//!
//! xdg-desktop-portal-gnome talks to gnome-shell over D-Bus on
//! `org.gnome.Mutter.ScreenCast`, `.DisplayConfig`,
//! `org.gnome.Shell.Introspect`, and `.Screenshot`. On
//! gnome-shell-less compositors (margo, niri, sway, …) those
//! interfaces don't exist and xdp-gnome silently fails.
//!
//! Niri solves this by **implementing those Mutter D-Bus interfaces
//! itself** — the compositor binary registers itself on the bus
//! under the gnome-shell well-known names and serves the ScreenCast
//! requests directly into its own render path. xdp-gnome doesn't
//! know it's not talking to real gnome-shell.
//!
//! This module ports niri's pattern. License is fine: niri is
//! GPL-3.0-or-later, margo is GPL-3.0-or-later. Source provenance
//! comments live at the top of each ported file.

pub mod cast_ids;
pub mod gnome_shell_introspect;
pub mod gnome_shell_screenshot;
pub mod ipc_output;
pub mod mutter_display_config;
pub mod mutter_screen_cast;
pub mod mutter_service_channel;

use zbus::blocking::Connection;
use zbus::object_server::Interface;

/// Common bring-up trait for each shim. Niri pattern: each
/// interface owns its own zbus blocking connection so failures in
/// one (e.g. ScreenCast can't open PipeWire) don't take the others
/// down.
pub(crate) trait Start: Interface {
    fn start(self) -> anyhow::Result<Connection>;
}

/// Container for every shim connection. Stored on `MargoState` for
/// lifetime management — connections close when the field is
/// dropped (compositor shutdown).
#[derive(Default)]
pub struct DBusServers {
    pub conn_service_channel: Option<Connection>,
    pub conn_display_config: Option<Connection>,
    pub conn_screen_shot: Option<Connection>,
    pub conn_introspect: Option<Connection>,
    pub conn_screen_cast: Option<Connection>,
}
