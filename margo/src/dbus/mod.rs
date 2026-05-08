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
//! `org.gnome.Mutter.ScreenCast`, `org.gnome.Mutter.DisplayConfig`,
//! `org.gnome.Shell.Introspect`, and `org.gnome.Shell.Screenshot`.
//! On gnome-shell-less compositors (margo, niri, sway, …) those
//! interfaces don't exist and xdp-gnome silently fails.
//!
//! niri solved this by **implementing those Mutter D-Bus interfaces
//! itself** — the compositor binary registers itself on the bus
//! under the gnome-shell well-known names and serves the ScreenCast
//! requests directly into its own render path. xdp-gnome doesn't
//! know it's not talking to real gnome-shell.
//!
//! This module ports niri's pattern. License is fine: niri is
//! GPL-3.0-or-later, margo is GPL-3.0-or-later. Source provenance
//! comments live at the top of each ported file.
//!
//! ## Module layout
//!
//! * [`mutter_screen_cast`] — `org.gnome.Mutter.ScreenCast`. The
//!   primary interface; serves session/stream creation, routes
//!   frames to PipeWire via `crate::screencasting`.
//! * [`mutter_display_config`] — `org.gnome.Mutter.DisplayConfig`.
//!   xdp-gnome cross-references this when enumerating monitors
//!   for the ScreenCast chooser dialog.
//! * [`mutter_service_channel`] — `org.gnome.Mutter.ServiceChannel`.
//!   Service-channel handshake xdp-gnome runs on bind.
//! * [`gnome_shell_introspect`] — `org.gnome.Shell.Introspect`.
//!   Reports the running compositor identity to xdp-gnome.
//! * [`gnome_shell_screenshot`] — `org.gnome.Shell.Screenshot`.
//!   Backs the Screenshot portal (separate from ScreenCast).
//!
//! ## Bring-up
//!
//! [`DBusServers::start`] is called once from `main.rs` after the
//! Wayland display is up. Each interface gets its own zbus
//! `Connection` so failures in one (e.g. ScreenCast can't open
//! PipeWire) don't take the others down. Connections are stored on
//! `MargoState::dbus_servers` for lifetime management.

#![cfg(any())] // Phase A scaffold — actual impl ports in Phase B.

pub mod gnome_shell_introspect;
pub mod gnome_shell_screenshot;
pub mod mutter_display_config;
pub mod mutter_screen_cast;
pub mod mutter_service_channel;

use zbus::blocking::Connection;

#[derive(Default)]
pub struct DBusServers {
    pub conn_service_channel: Option<Connection>,
    pub conn_display_config: Option<Connection>,
    pub conn_screen_shot: Option<Connection>,
    pub conn_introspect: Option<Connection>,
    pub conn_screen_cast: Option<Connection>,
}
