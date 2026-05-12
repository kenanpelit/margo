//! D-Bus screensaver/session-manager/portal **inhibit eavesdropping**.
//!
//! Ported (and trimmed) from stasis's `services/dbus.rs`. The idea:
//! browsers and media players already tell the system "don't idle me"
//! via well-known D-Bus interfaces while they're playing video.
//! Process-name matching on `/proc` cannot tell whether Helium is
//! actually playing a YouTube video or just sitting on a tab, so it
//! either over- or under-inhibits. Asking D-Bus is the correct
//! signal.
//!
//! Three protocols are tracked:
//!
//! * `org.freedesktop.ScreenSaver.Inhibit(reason, app_id)` /
//!   `UnInhibit(cookie)`  — legacy XDG, used by Firefox, Chromium,
//!   Spotify, mpv, totem, …
//! * `org.gnome.SessionManager.Inhibit(...)` / `Uninhibit(cookie)`
//!   — same shape, different bus name.
//! * `org.freedesktop.portal.Inhibit.Inhibit(...)` →
//!   `org.freedesktop.portal.Request.Close()` — the xdg-desktop-portal
//!   way (Flatpak / Wayland-portal-aware browsers).
//!
//! Per-sender state is kept in a `DbusInhibitTracker`. We become a
//! D-Bus monitor (read-only firehose of all session-bus traffic) via
//! the `org.freedesktop.DBus.Monitoring.BecomeMonitor` call, then
//! correlate method-calls with their returns by serial number so we
//! can capture the cookie/handle that uniquely identifies each
//! inhibitor. A `NameOwnerChanged` signal with an empty new owner
//! (sender disconnect) clears its rows so a crashing browser doesn't
//! leave us inhibiting forever.
//!
//! Emits `WaylandEvent::DbusInhibit(active)` only on **edges** —
//! 0→N and N→0 — so the manager doesn't get spammed.

use anyhow::{Context, Result};
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};
use zbus::Connection;

use crate::daemon::WaylandEvent;

#[derive(Debug, Default)]
struct Tracker {
    /// Active inhibitors keyed by D-Bus unique sender (":1.42").
    active: HashMap<String, SenderInhibits>,
    /// Method-call serials we're still waiting on a return for —
    /// legacy ScreenSaver/SessionManager.
    pending_legacy: HashMap<String, HashSet<u32>>,
    /// Same, for portal Inhibit calls (returns a Request handle).
    pending_portal: HashMap<String, HashSet<u32>>,
}

#[derive(Debug, Default)]
struct SenderInhibits {
    legacy_cookies: HashSet<u32>,
    portal_handles: HashSet<String>,
}

impl Tracker {
    fn is_active(&self) -> bool {
        !self.active.is_empty()
    }

    fn mark_legacy_call(&mut self, sender: &str, serial: u32) {
        self.pending_legacy
            .entry(sender.to_string())
            .or_default()
            .insert(serial);
    }

    fn mark_portal_call(&mut self, sender: &str, serial: u32) {
        self.pending_portal
            .entry(sender.to_string())
            .or_default()
            .insert(serial);
    }

    /// Returns Some(true) if a 0→N edge was crossed.
    fn confirm_legacy_cookie(
        &mut self,
        sender: &str,
        reply_serial: u32,
        cookie: u32,
    ) -> EdgeChange {
        let pending = self
            .pending_legacy
            .get_mut(sender)
            .map(|set| set.remove(&reply_serial))
            .unwrap_or(false);
        if !pending {
            return EdgeChange::None;
        }
        let was_active = self.is_active();
        self.active
            .entry(sender.to_string())
            .or_default()
            .legacy_cookies
            .insert(cookie);
        if !was_active && self.is_active() {
            EdgeChange::Activated
        } else {
            EdgeChange::None
        }
    }

    fn confirm_portal_handle(
        &mut self,
        sender: &str,
        reply_serial: u32,
        handle: &str,
    ) -> EdgeChange {
        let pending = self
            .pending_portal
            .get_mut(sender)
            .map(|set| set.remove(&reply_serial))
            .unwrap_or(false);
        if !pending {
            return EdgeChange::None;
        }
        let was_active = self.is_active();
        self.active
            .entry(sender.to_string())
            .or_default()
            .portal_handles
            .insert(handle.to_string());
        if !was_active && self.is_active() {
            EdgeChange::Activated
        } else {
            EdgeChange::None
        }
    }

    fn clear_legacy_cookie(&mut self, sender: &str, cookie: u32) -> EdgeChange {
        let removed = self
            .active
            .get_mut(sender)
            .map(|s| s.legacy_cookies.remove(&cookie))
            .unwrap_or(false);
        if removed {
            self.gc_sender(sender);
            if !self.is_active() {
                return EdgeChange::Deactivated;
            }
        }
        EdgeChange::None
    }

    fn clear_portal_handle(&mut self, sender: &str, handle: &str) -> EdgeChange {
        let removed = self
            .active
            .get_mut(sender)
            .map(|s| s.portal_handles.remove(handle))
            .unwrap_or(false);
        if removed {
            self.gc_sender(sender);
            if !self.is_active() {
                return EdgeChange::Deactivated;
            }
        }
        EdgeChange::None
    }

    fn remove_sender(&mut self, sender: &str) -> EdgeChange {
        let was_active = self.is_active();
        self.active.remove(sender);
        self.pending_legacy.remove(sender);
        self.pending_portal.remove(sender);
        if was_active && !self.is_active() {
            EdgeChange::Deactivated
        } else {
            EdgeChange::None
        }
    }

    fn gc_sender(&mut self, sender: &str) {
        let drop_it = self
            .active
            .get(sender)
            .is_some_and(|s| s.legacy_cookies.is_empty() && s.portal_handles.is_empty());
        if drop_it {
            self.active.remove(sender);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum EdgeChange {
    None,
    Activated,
    Deactivated,
}

pub async fn dbus_inhibit_loop(tx: mpsc::Sender<WaylandEvent>) {
    if let Err(e) = run(tx).await {
        warn!("dbus_inhibit loop ended: {e:#}");
    }
}

async fn run(tx: mpsc::Sender<WaylandEvent>) -> Result<()> {
    let monitor = Connection::session()
        .await
        .context("connect to session D-Bus")?;

    // Become a passive monitor — read-only firehose of all messages
    // on the session bus. Empty match-rule list means "everything".
    monitor
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Monitoring"),
            "BecomeMonitor",
            &(&[] as &[&str], 0u32),
        )
        .await
        .context("BecomeMonitor on session bus")?;

    info!("dbus inhibit monitor armed (session bus)");

    let tracker: std::sync::Arc<Mutex<Tracker>> = std::sync::Arc::new(Mutex::new(Tracker::default()));
    let mut stream = zbus::MessageStream::from(monitor);

    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let header = msg.header();

        let iface = header
            .interface()
            .map(|i| i.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let member = header
            .member()
            .map(|m| m.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        match msg.message_type() {
            zbus::message::Type::MethodCall => {
                let Some(sender) = header.sender() else {
                    continue;
                };
                let sender = sender.to_string();

                let is_legacy_iface = iface == "org.freedesktop.screensaver"
                    || iface == "org.gnome.sessionmanager";

                // Legacy Inhibit (ScreenSaver / SessionManager) — body
                // is (s,s) and the return carries the cookie u32.
                if is_legacy_iface && member == "inhibit" {
                    let serial = header.primary().serial_num().get();
                    tracker.lock().await.mark_legacy_call(&sender, serial);
                    continue;
                }

                // Legacy UnInhibit — body is just the cookie u32.
                if is_legacy_iface && member == "uninhibit" {
                    let Ok(cookie) = msg.body().deserialize::<u32>() else {
                        continue;
                    };
                    let edge = tracker.lock().await.clear_legacy_cookie(&sender, cookie);
                    emit_edge(&tx, edge, "legacy uninhibit").await;
                    continue;
                }

                // Portal Inhibit — returns an object path (request handle).
                if iface == "org.freedesktop.portal.inhibit" && member == "inhibit" {
                    let serial = header.primary().serial_num().get();
                    tracker.lock().await.mark_portal_call(&sender, serial);
                    continue;
                }

                // Portal Request.Close — closes a previously-issued inhibit.
                if iface == "org.freedesktop.portal.request" && member == "close" {
                    let Some(path) = header.path() else { continue };
                    let edge = tracker
                        .lock()
                        .await
                        .clear_portal_handle(&sender, path.as_str());
                    emit_edge(&tx, edge, "portal request close").await;
                }
            }

            zbus::message::Type::MethodReturn => {
                let Some(reply_serial) = header.reply_serial() else {
                    continue;
                };
                let Some(dest) = header.destination() else {
                    continue;
                };
                let sender = dest.as_str().to_string();
                let reply_serial = reply_serial.get();

                // Legacy returns: u32 cookie.
                if let Ok(cookie) = msg.body().deserialize::<u32>() {
                    let edge = tracker
                        .lock()
                        .await
                        .confirm_legacy_cookie(&sender, reply_serial, cookie);
                    emit_edge(&tx, edge, "legacy inhibit return").await;
                    continue;
                }

                // Portal returns: object path.
                if let Ok(handle) =
                    msg.body().deserialize::<zbus::zvariant::OwnedObjectPath>()
                {
                    let edge = tracker.lock().await.confirm_portal_handle(
                        &sender,
                        reply_serial,
                        handle.as_str(),
                    );
                    emit_edge(&tx, edge, "portal inhibit return").await;
                }
            }

            // Sender disconnect: peel its rows out so a crashed
            // browser doesn't keep inhibiting us forever.
            zbus::message::Type::Signal
                if iface == "org.freedesktop.dbus" && member == "nameownerchanged" =>
            {
                let Ok((name, _old, new_owner)) =
                    msg.body().deserialize::<(String, String, String)>()
                else {
                    continue;
                };
                if name.starts_with(':') && new_owner.is_empty() {
                    let edge = tracker.lock().await.remove_sender(&name);
                    emit_edge(&tx, edge, "sender disconnect").await;
                }
            }

            _ => {}
        }
    }

    Ok(())
}

async fn emit_edge(tx: &mpsc::Sender<WaylandEvent>, edge: EdgeChange, reason: &str) {
    match edge {
        EdgeChange::Activated => {
            debug!("dbus_inhibit: 0 -> active ({reason})");
            let _ = tx.send(WaylandEvent::DbusInhibit(true)).await;
        }
        EdgeChange::Deactivated => {
            debug!("dbus_inhibit: active -> 0 ({reason})");
            let _ = tx.send(WaylandEvent::DbusInhibit(false)).await;
        }
        EdgeChange::None => {}
    }
}
