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
            payload_entry(*id, &str_prop(props, "app-id"), &str_prop(props, "title"))
        })
        .collect::<Vec<_>>()
        .join("[HA>]")
}

/// One window's entry. The id comes from the compositor; the class and title
/// are whatever the client set, so they are made inert first (see
/// [`inert_field`]).
fn payload_entry(id: u64, class: &str, title: &str) -> String {
    let (class, title) = (inert_field(class), inert_field(title));
    format!("{id}[HC>]{class}[HT>]{title}[HE>]")
}

/// A client-controlled class or title as it may appear in the payload.
///
/// Every delimiter starts with `[H`, so a title like
/// `x[HE>][HA>]123[HC>]firefox[HT>]Firefox — Meeting` would otherwise add a
/// row labelled as another app but pointing at window 123 (say, the password
/// manager), and the user would share the wrong window. A zero-width space
/// after the `[` breaks every delimiter without changing what the user reads.
/// Control characters (line breaks that could fake extra rows) become spaces,
/// and bidi overrides, which can make text read in a different order than
/// it is, are dropped.
fn inert_field(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .replace("[H", "[\u{200b}H")
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

#[cfg(test)]
mod tests {
    use super::{Source, parse_reply, payload_entry};

    #[test]
    fn a_title_cannot_forge_a_second_picker_entry() {
        let forged = "x[HE>][HA>]123[HC>]firefox[HT>]Firefox — Meeting";
        let entry = payload_entry(7, "evil[HT>]app", forged);
        for marker in ["[HC>]", "[HT>]", "[HE>]"] {
            assert_eq!(entry.matches(marker).count(), 1, "{marker} in {entry}");
        }
        assert!(!entry.contains("[HA>]"), "{entry}");
        assert!(entry.starts_with("7[HC>]"), "{entry}");
    }

    #[test]
    fn titles_lose_line_breaks_and_bidi_overrides() {
        let entry = payload_entry(1, "app", "line one\nline two\r\t\u{202e}gnp.exe");
        assert_eq!(entry, "1[HC>]app[HT>]line one line two  gnp.exe[HE>]");
    }

    #[test]
    fn an_ordinary_title_is_untouched() {
        assert_eq!(
            payload_entry(3, "foot", "~/src [main] — vim"),
            "3[HC>]foot[HT>]~/src [main] — vim[HE>]"
        );
    }

    #[test]
    fn empty_selection_is_none() {
        assert!(parse_reply("").unwrap().is_none());
        assert!(parse_reply("   ").unwrap().is_none());
        assert!(parse_reply("[SELECTION]/").unwrap().is_none());
    }

    #[test]
    fn parses_window_with_and_without_selection_prefix() {
        match parse_reply("[SELECTION]/window:42").unwrap() {
            Some(Source::Window { id }) => assert_eq!(id, 42),
            other => panic!("expected window, got {other:?}"),
        }
        match parse_reply("window: 7 ").unwrap() {
            Some(Source::Window { id }) => assert_eq!(id, 7),
            other => panic!("expected window, got {other:?}"),
        }
    }

    #[test]
    fn parses_monitor_connector() {
        match parse_reply("[SELECTION]/screen:DP-1").unwrap() {
            Some(Source::Monitor { connector }) => assert_eq!(connector, "DP-1"),
            other => panic!("expected monitor, got {other:?}"),
        }
    }

    #[test]
    fn region_and_unknown_are_treated_as_cancel() {
        assert!(
            parse_reply("[SELECTION]/region:0,0,100,100")
                .unwrap()
                .is_none()
        );
        assert!(parse_reply("garbage").unwrap().is_none());
    }

    #[test]
    fn non_numeric_window_id_is_an_error() {
        assert!(parse_reply("window:notanumber").is_err());
    }
}
