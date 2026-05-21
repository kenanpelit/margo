//! Source selection for a ScreenCast.Start.
//!
//! Enumerate windows from margo's `org.gnome.Shell.Introspect` shim,
//! then hand the list to mshell's existing `Screenshare` picker (the
//! DESIGN.md-styled chooser — same UI the rest of the shell uses, which
//! also offers Screen / Region tabs). The chooser blocks until the user
//! picks and echoes the window-id back, so it round-trips straight into
//! the Mutter shim's `RecordWindow`.

use crate::mutter::{IntrospectProxy, ShellProxy};
use std::collections::HashMap;
use tracing::{debug, warn};
use zbus::Connection;
use zbus::zvariant::OwnedValue;

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

/// Show the picker and return the chosen source (`Ok(None)` = cancel).
pub async fn pick(conn: &Connection, _types: u32) -> anyhow::Result<Option<Source>> {
    // Build the window list payload from Introspect (best-effort: if it
    // fails the picker still offers Screen / Region).
    let payload = match IntrospectProxy::new(conn).await {
        Ok(introspect) => match introspect.get_windows().await {
            Ok(windows) => build_payload(&windows),
            Err(e) => {
                warn!(error = %e, "picker: GetWindows failed");
                String::new()
            }
        },
        Err(e) => {
            warn!(error = %e, "picker: Introspect proxy failed");
            String::new()
        }
    };

    let shell = ShellProxy::new(conn).await?;
    let reply = shell.screenshare(&payload).await?;
    debug!(reply = %reply, "picker: shell reply");
    parse_reply(&reply)
}

/// xdph `XDPH_WINDOW_SHARING_LIST` format: per window
/// `<id>[HC>]<class>[HT>]<title>[HE>]`, joined by `[HA>]`.
fn build_payload(windows: &HashMap<u64, HashMap<String, OwnedValue>>) -> String {
    windows
        .iter()
        .map(|(id, props)| {
            let class = str_prop(props, "app-id");
            let title = str_prop(props, "title");
            format!("{id}[HC>]{class}[HT>]{title}[HE>]")
        })
        .collect::<Vec<_>>()
        .join("[HA>]")
}

fn parse_reply(reply: &str) -> anyhow::Result<Option<Source>> {
    let sel = reply.trim();
    let sel = sel.strip_prefix("[SELECTION]/").unwrap_or(sel);
    if sel.is_empty() {
        return Ok(None);
    }
    if let Some(id) = sel.strip_prefix("window:") {
        return Ok(Some(Source::Window {
            id: id.trim().parse()?,
        }));
    }
    if let Some(name) = sel.strip_prefix("screen:") {
        return Ok(Some(Source::Monitor {
            connector: name.trim().to_string(),
        }));
    }
    // `region:…` isn't expressible through the Mutter shim's
    // window/monitor record verbs — treat as cancel for now.
    warn!(%reply, "picker: unsupported selection (region?)");
    Ok(None)
}

fn str_prop(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or_default()
        .to_string()
}
