//! `org.freedesktop.impl.portal.Screenshot` backend.
//!
//! Fulfilled by driving margo's own `org.gnome.Shell.Screenshot` shim
//! (compositor screencopy → PNG + native colour pick), same pattern as
//! the ScreenCast backend. This lets `margo-portals.conf` route
//! `Screenshot=margo` and drop the GNOME backend entirely.
//!
//! `Screenshot.interactive` (region select) isn't expressible through
//! the shim's full-frame verb yet — we take a full-screen shot for now;
//! the in-shell region selector is the follow-up.

use crate::mutter::ShellScreenshotProxy;
use std::collections::HashMap;
use tracing::{info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_ERROR: u32 = 2;

pub struct ScreenshotBackend {
    conn: Connection,
}

impl ScreenshotBackend {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenshotBackend {
    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    async fn screenshot(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let shim = match ShellScreenshotProxy::new(&self.conn).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "screenshot: Shell.Screenshot proxy failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };
        // The compositor picks the output path; the passed filename is a
        // hint and the used path is what comes back.
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let hint = format!("{runtime}/margo-portal-shot.png");
        match shim.screenshot(false, false, &hint).await {
            Ok((true, used)) => {
                let mut results = HashMap::new();
                results.insert("uri".into(), owned_str(format!("file://{used}")));
                info!(%used, "screenshot: captured");
                (RESPONSE_SUCCESS, results)
            }
            Ok((false, _)) => {
                warn!("screenshot: shim reported failure");
                (RESPONSE_ERROR, HashMap::new())
            }
            Err(e) => {
                warn!(error = %e, "screenshot: shim call failed");
                (RESPONSE_ERROR, HashMap::new())
            }
        }
    }

    async fn pick_color(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let shim = match ShellScreenshotProxy::new(&self.conn).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "pick_color: proxy failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };
        match shim.pick_color().await {
            Ok(map) => {
                // Shim returns `{ "color": (ddd) }`; pass it straight on.
                let mut results = HashMap::new();
                if let Some(c) = map.get("color").and_then(|v| v.try_clone().ok()) {
                    results.insert("color".into(), c);
                }
                (RESPONSE_SUCCESS, results)
            }
            Err(e) => {
                warn!(error = %e, "pick_color: shim call failed");
                (RESPONSE_ERROR, HashMap::new())
            }
        }
    }
}

fn owned_str(s: String) -> OwnedValue {
    Value::from(s).try_to_owned().expect("String → OwnedValue")
}
