//! Source selection for a ScreenCast.Start.
//!
//! First cut: enumerate windows via margo's `org.gnome.Shell.Introspect`
//! shim and pick programmatically. The real interactive picker (driving
//! mshell's existing `Screenshare` menu UI) is the next iteration —
//! tracked so this stays a single, reviewable change.

use crate::mutter::IntrospectProxy;
use tracing::debug;
use zbus::Connection;

#[derive(Debug)]
pub enum Source {
    Window { id: u64 },
    Monitor { connector: String },
}

impl Source {
    /// org.freedesktop.impl.portal.ScreenCast SourceType bit.
    pub fn source_type(&self) -> u32 {
        match self {
            Source::Window { .. } => 2,
            Source::Monitor { .. } => 1,
        }
    }
}

/// Choose a capture source matching `types` (MONITOR=1 | WINDOW=2).
/// Returns `Ok(None)` when the user cancels (no source available is
/// treated as cancel for now).
pub async fn pick(conn: &Connection, types: u32) -> anyhow::Result<Option<Source>> {
    // Prefer a window when the app accepts windows.
    if types & 2 != 0 {
        let introspect = IntrospectProxy::new(conn).await?;
        let windows = introspect.get_windows().await?;
        // TODO(next): present mshell's Screenshare picker UI and let the
        // user choose; for now take the first enumerated window so the
        // capture path is exercised end-to-end.
        if let Some((&id, props)) = windows.iter().next() {
            let title = props
                .get("title")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default();
            debug!(id, %title, "picker: selected window (first; UI picker TODO)");
            return Ok(Some(Source::Window { id }));
        }
    }

    // Monitor capture needs the output connector; full enumeration via
    // Mutter DisplayConfig lands with the picker UI. Until then a
    // window-only first cut returns "no source" for monitor-only asks.
    if types == 1 {
        debug!("picker: monitor-only request — output picker not wired yet");
    }
    Ok(None)
}
