# vendor/wayle-notification — margo patch

Upstream: [`wayle-notification` 0.1.3](https://crates.io/crates/wayle-notification/0.1.3)
(crates.io). This directory is a verbatim copy of that release plus the
changes below; the root `Cargo.toml` swaps it in via `[patch.crates-io]`.
`Cargo.toml` and everything not listed here are byte-identical to upstream.

## What the patch does

1. **Inline reply (KDE-style).** Advertises the `inline-reply` capability,
   adds `Notification::reply(text)` and emits the
   `NotificationReplied(id, text)` signal, so mshell can render a reply entry
   for notifications carrying an `"inline-reply"` action
   (`src/core/controls.rs`, `src/core/notification.rs`, `src/types/mod.rs`,
   `src/types/notification.rs`, `src/daemon.rs`).
2. **`sound` capability.** Advertised in `src/daemon.rs`: the shell plays
   notification sounds and honours the `sound-file` / `suppress-sound` hints.
3. **Untrusted `image-data` hardening.** `src/image_cache.rs` rejects
   zero/negative geometry, a `channels * width` overflow and a rowstride
   smaller than one row before caching the image. Any process on the session
   bus can send that hint; upstream's `data.chunks(rowstride)` panics on a
   zero rowstride and takes the notification daemon thread down.
4. **Lossless DND changes.** `NotificationService::dnd_changes()` returns a
   receiver that gets every value passed to `set_dnd`, in order
   (`src/service.rs`, `src/builder.rs`). `dnd.watch()` only yields the
   latest value, so a quick off→on collapses into one "on"; mshell's
   auto-DND needs each write to tell that the user took DND over.

All four are upstream-PR-able; drop the vendored copy once wayle ships them.

## Refreshing this diff

```sh
curl -sSfL https://static.crates.io/crates/wayle-notification/wayle-notification-0.1.3.crate | tar xz
rm wayle-notification-0.1.3/{Cargo.toml.orig,.cargo_vcs_info.json}
diff -ruN wayle-notification-0.1.3 vendor/wayle-notification   # minus this file
```

## Diff against upstream 0.1.3

```diff
diff -ruN a/src/builder.rs b/src/builder.rs
--- a/src/builder.rs
+++ b/src/builder.rs
@@ -151,6 +151,7 @@
             popups,
             popup_duration: self.popup_duration,
             dnd: self.dnd,
+            dnd_events: Default::default(),
             remove_expired: self.remove_expired,
             blocklist: self.blocklist,
             popup_timers,
diff -ruN a/src/core/controls.rs b/src/core/controls.rs
--- a/src/core/controls.rs
+++ b/src/core/controls.rs
@@ -30,4 +30,23 @@
 
         Ok(())
     }
+
+    /// Emit the KDE-style `NotificationReplied(id, text)` signal — the
+    /// delivery half of the `inline-reply` capability. Clients that sent
+    /// an `"inline-reply"` action listen for this to receive the text the
+    /// user typed into the notification (margo vendor extension).
+    #[instrument(skip(connection, text), fields(notification_id = %id), err)]
+    pub(super) async fn reply(connection: &Connection, id: &u32, text: &str) -> Result<(), Error> {
+        connection
+            .emit_signal(
+                None::<()>,
+                SERVICE_PATH,
+                SERVICE_INTERFACE,
+                Signal::NotificationReplied.as_str(),
+                &(id, text),
+            )
+            .await?;
+
+        Ok(())
+    }
 }
diff -ruN a/src/core/notification.rs b/src/core/notification.rs
--- a/src/core/notification.rs
+++ b/src/core/notification.rs
@@ -140,6 +140,26 @@
         Ok(())
     }
 
+    /// Send an inline reply: emits `NotificationReplied(id, text)` so the
+    /// client receives the typed text (KDE-style `inline-reply`
+    /// capability; margo vendor extension).
+    ///
+    /// The notification is dismissed if it is not a resident, mirroring
+    /// [`invoke`](Self::invoke).
+    ///
+    /// # Errors
+    /// Returns error if the D-Bus signal emission fails.
+    #[instrument(skip(self, text), fields(notification_id = %self.id), err)]
+    pub async fn reply(&self, text: &str) -> Result<(), Error> {
+        NotificationControls::reply(&self.zbus_connection, &self.id, text).await?;
+        if !self.is_resident.get() {
+            let _ = self
+                .notif_tx
+                .send(NotificationEvent::Remove(self.id, ClosedReason::Closed));
+        }
+        Ok(())
+    }
+
     #[allow(clippy::too_many_lines)]
     fn from_props(
         props: NotificationProps,
diff -ruN a/src/daemon.rs b/src/daemon.rs
--- a/src/daemon.rs
+++ b/src/daemon.rs
@@ -116,6 +116,12 @@
             Capabilities::Actions.to_string(),
             Capabilities::IconStatic.to_string(),
             Capabilities::Persistence.to_string(),
+            // margo additions: the shell plays notification sounds (and
+            // honours sound-file / suppress-sound hints) and renders a
+            // KDE-style reply entry for "inline-reply" actions, answering
+            // with the NotificationReplied(id, text) signal.
+            Capabilities::Sound.to_string(),
+            Capabilities::InlineReply.to_string(),
         ]
     }
 
diff -ruN a/src/image_cache.rs b/src/image_cache.rs
--- a/src/image_cache.rs
+++ b/src/image_cache.rs
@@ -38,6 +38,29 @@
 ) -> Option<String> {
     let color_type = png_color_type(bits_per_sample, channels)?;
 
+    // width / height / rowstride / data all come straight from an untrusted
+    // `image-data` D-Bus hint — any app on the session bus can send a
+    // crafted one. Reject degenerate geometry before it reaches
+    // strip_rowstride_padding, whose `data.chunks(rowstride)` panics on
+    // rowstride == 0 and whose `channels * width` can overflow i32. Either
+    // would take down the whole notification daemon thread — a remote DoS
+    // from any unprivileged process.
+    if width <= 0 || height <= 0 || rowstride <= 0 {
+        warn!(width, height, rowstride, "invalid image-data geometry, skipping cache");
+        return None;
+    }
+    let Some(row_bytes) = channels.checked_mul(width).filter(|rb| *rb > 0) else {
+        warn!(width, channels, "image-data row size overflow, skipping cache");
+        return None;
+    };
+    if rowstride < row_bytes {
+        warn!(
+            rowstride,
+            row_bytes, "image-data rowstride smaller than one row, skipping cache"
+        );
+        return None;
+    }
+
     let dir = cache_dir();
     let path = dir.join(format!("{}.png", content_hash(data)));
 
diff -ruN a/src/service.rs b/src/service.rs
--- a/src/service.rs
+++ b/src/service.rs
@@ -1,4 +1,4 @@
-use std::sync::Arc;
+use std::sync::{Arc, Mutex};
 
 use derive_more::Debug;
 use tokio::sync::broadcast;
@@ -33,6 +33,8 @@
     pub popup_duration: Property<u32>,
     /// Do Not Disturb mode; suppresses popups when true.
     pub dnd: Property<bool>,
+    #[debug(skip)]
+    pub(crate) dnd_events: DndEvents,
     /// Auto-remove expired notifications.
     pub remove_expired: Property<bool>,
     /// Glob patterns for blocking notifications by app name.
@@ -81,7 +83,17 @@
     /// When enabled, new notifications will not appear as popups but will
     /// still be added to the notification list.
     pub fn set_dnd(&self, dnd: bool) {
-        self.dnd.set(dnd)
+        self.dnd.set(dnd);
+        self.dnd_events.send(dnd);
+    }
+
+    /// Every value passed to [`Self::set_dnd`] from now on, in order.
+    ///
+    /// Unlike `dnd.watch()`, which only yields the latest value, nothing is
+    /// coalesced: a quick off→on still arrives as `false`, `true`. The
+    /// receiver must be drained; dropping it unsubscribes.
+    pub fn dnd_changes(&self) -> tokio::sync::mpsc::UnboundedReceiver<bool> {
+        self.dnd_events.subscribe()
     }
 
     /// Sets the duration for how long popup notifications are displayed.
@@ -116,8 +128,57 @@
     }
 }
 
+/// Subscribers to [`NotificationService::dnd_changes`].
+#[derive(Default)]
+pub(crate) struct DndEvents(Mutex<Vec<tokio::sync::mpsc::UnboundedSender<bool>>>);
+
+impl DndEvents {
+    fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<bool> {
+        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
+        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
+        rx
+    }
+
+    /// Queue `value` for every live subscriber; drop the closed ones.
+    fn send(&self, value: bool) {
+        self.0
+            .lock()
+            .unwrap_or_else(|e| e.into_inner())
+            .retain(|tx| tx.send(value).is_ok());
+    }
+}
+
 impl Drop for NotificationService {
     fn drop(&mut self) {
         self.cancellation_token.cancel();
     }
 }
+
+#[cfg(test)]
+mod dnd_event_tests {
+    use super::DndEvents;
+
+    #[test]
+    fn fast_off_on_is_queued_in_order_for_every_subscriber() {
+        let events = DndEvents::default();
+        let mut a = events.subscribe();
+        let mut b = events.subscribe();
+        for value in [true, false, true] {
+            events.send(value);
+        }
+        for rx in [&mut a, &mut b] {
+            assert_eq!(rx.try_recv(), Ok(true));
+            assert_eq!(rx.try_recv(), Ok(false));
+            assert_eq!(rx.try_recv(), Ok(true));
+            assert!(rx.try_recv().is_err());
+        }
+    }
+
+    #[test]
+    fn dropped_subscriber_is_forgotten() {
+        let events = DndEvents::default();
+        drop(events.subscribe());
+        events.send(true);
+        assert!(events.0.lock().expect("not poisoned").is_empty());
+    }
+}
diff -ruN a/src/types/mod.rs b/src/types/mod.rs
--- a/src/types/mod.rs
+++ b/src/types/mod.rs
@@ -22,6 +22,9 @@
     ActionInvoked,
     #[allow(dead_code)]
     ActivationToken,
+    /// KDE-style inline reply: `NotificationReplied(id: u32, text: String)`,
+    /// paired with the `inline-reply` capability (margo vendor extension).
+    NotificationReplied,
 }
 
 impl Signal {
@@ -30,6 +33,7 @@
             Signal::NotificationClosed => "NotificationClosed",
             Signal::ActionInvoked => "ActionInvoked",
             Signal::ActivationToken => "ActivationToken",
+            Signal::NotificationReplied => "NotificationReplied",
         }
     }
 }
diff -ruN a/src/types/notification.rs b/src/types/notification.rs
--- a/src/types/notification.rs
+++ b/src/types/notification.rs
@@ -94,6 +94,12 @@
     ///
     /// When present, the server also supports the "sound-file" and "suppress-sound" hints.
     Sound,
+    /// KDE-style inline reply: the server renders a text entry for
+    /// notifications carrying an `"inline-reply"` action and emits the
+    /// `NotificationReplied(id, text)` signal with the typed text.
+    ///
+    /// margo vendor extension (matches KDE Plasma / SwayNC behaviour).
+    InlineReply,
     /// Vendor-specific capability.
     Vendor(String),
 }
@@ -113,6 +119,7 @@
             "icon-static" => Self::IconStatic,
             "persistence" => Self::Persistence,
             "sound" => Self::Sound,
+            "inline-reply" => Self::InlineReply,
             s if s.starts_with("x-") => Self::Vendor(s.to_string()),
             _ => Self::Vendor(format!("x-unknown-{s}")),
         })
@@ -133,6 +140,7 @@
             Self::IconStatic => "icon-static",
             Self::Persistence => "persistence",
             Self::Sound => "sound",
+            Self::InlineReply => "inline-reply",
             Self::Vendor(s) => s,
         }
     }
```
