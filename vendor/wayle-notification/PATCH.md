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

All three are upstream-PR-able; drop the vendored copy once wayle ships them.

## Refreshing this diff

```sh
curl -sSfL https://static.crates.io/crates/wayle-notification/wayle-notification-0.1.3.crate | tar xz
rm wayle-notification-0.1.3/{Cargo.toml.orig,.cargo_vcs_info.json}
diff -ruN wayle-notification-0.1.3 vendor/wayle-notification   # minus this file
```

## Diff against upstream 0.1.3

```diff
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
