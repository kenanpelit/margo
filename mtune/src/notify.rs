// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop notifications via `org.freedesktop.Notifications`.
//!
//! mtune already holds the MPRIS server's session-bus connection, so
//! toasts ride on it — no `notify-send` subprocess and no extra
//! dependency. A running notification daemon (margo's mshell) renders
//! them as corner toasts.

use std::collections::HashMap;

use zbus::zvariant::Value;

const BUS: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

/// Post (or replace) a toast and return the daemon-assigned id.
///
/// Pass the previous id back as `replaces_id` so repeats coalesce into
/// one toast instead of stacking; `0` starts a fresh one. `transient`
/// asks the daemon to skip its persistent history entry — used for the
/// quick setting-change confirmations. Returns `0` on any failure (no
/// daemon, malformed reply); the caller just stores that as "no toast
/// to replace next time".
pub async fn notify(
    conn: &zbus::Connection,
    replaces_id: u32,
    summary: &str,
    body: &str,
    transient: bool,
    timeout_ms: i32,
) -> u32 {
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("category", Value::from("x-gnome.music"));
    if transient {
        hints.insert("transient", Value::from(true));
    }
    let actions: Vec<&str> = Vec::new();

    let res = conn
        .call_method(
            Some(BUS),
            PATH,
            Some(IFACE),
            "Notify",
            &(
                "Tune",
                replaces_id,
                "org.margo.Tune",
                summary,
                body,
                actions,
                hints,
                timeout_ms,
            ),
        )
        .await;

    match res {
        Ok(reply) => reply.body().deserialize::<u32>().unwrap_or(0),
        Err(e) => {
            log::warn!("mtune: notification failed: {e}");
            0
        }
    }
}
