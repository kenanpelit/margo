//! `margo-portal` — margo's native xdg-desktop-portal backend.
//!
//! Currently serves `org.freedesktop.impl.portal.ScreenCast` so browser
//! meeting clients can share a **window** (or monitor) without the
//! GNOME portal backend. Capture itself is margo's own
//! (PipeWire + per-window/per-output via the compositor's
//! `org.gnome.Mutter.ScreenCast` shim); this daemon only orchestrates
//! the portal handshake and hands back the PipeWire node id.
//!
//! Activated lazily by D-Bus on first portal call; stays up for the
//! session. Selected as the ScreenCast backend via
//! `margo-portals.conf` (`org.freedesktop.impl.portal.ScreenCast=margo`).

mod global_shortcuts;
mod mutter;
mod picker;
mod screencast;
mod screenshot;

use anyhow::Result;
use global_shortcuts::GlobalShortcutsBackend;
use screencast::ScreenCastBackend;
use screenshot::ScreenshotBackend;
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::connection;

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.margo";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // One session-bus connection: it both hosts our portal interface
    // and reaches margo's Mutter shim.
    let conn = connection::Builder::session()?.build().await?;
    conn.object_server()
        .at(OBJECT_PATH, ScreenCastBackend::new(conn.clone()))
        .await?;
    conn.object_server()
        .at(OBJECT_PATH, ScreenshotBackend::new(conn.clone()))
        .await?;
    conn.object_server()
        .at(OBJECT_PATH, GlobalShortcutsBackend::new())
        .await?;
    conn.request_name(BUS_NAME).await?;

    // Persistent margo-socket listener re-emitting shortcut
    // activations as portal Activated/Deactivated signals.
    tokio::spawn(global_shortcuts::run_event_listener(
        conn.clone(),
        OBJECT_PATH,
    ));

    info!("margo-portal: serving {BUS_NAME} at {OBJECT_PATH}");
    std::future::pending::<()>().await;
    Ok(())
}
