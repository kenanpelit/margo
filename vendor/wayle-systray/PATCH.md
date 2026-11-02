# vendor/wayle-systray — margo patch

Upstream: [`wayle-systray` 0.1.4](https://crates.io/crates/wayle-systray/0.1.4)
(crates.io). This directory is a verbatim copy of that release plus the one
change below; the root `Cargo.toml` swaps it in via `[patch.crates-io]`.
`Cargo.toml` and everything not listed here are byte-identical to upstream.

## What the patch does

The `NewIcon` handler in `src/core/item/monitoring.rs` re-reads
`IconThemePath` before the icon name. Electron/Chromium apps (Ferdium,
Discord, …) write each tray icon into a fresh temp dir and only emit
`NewIcon`, never `PropertiesChanged`, so the cached theme path pointed at a
deleted dir and the tray showed the missing-icon glyph. Upstream-PR-able; drop
the vendored copy once wayle refreshes the theme path on `NewIcon`.

## Refreshing this diff

```sh
curl -sSfL https://static.crates.io/crates/wayle-systray/wayle-systray-0.1.4.crate | tar xz
rm wayle-systray-0.1.4/{Cargo.toml.orig,.cargo_vcs_info.json}
diff -ruN wayle-systray-0.1.4 vendor/wayle-systray   # minus this file
```

## Diff against upstream 0.1.4

```diff
diff -ruN a/src/core/item/monitoring.rs b/src/core/item/monitoring.rs
--- a/src/core/item/monitoring.rs
+++ b/src/core/item/monitoring.rs
@@ -322,6 +322,15 @@
 
             Some(_) = new_icon.next() => {
                 debug!("NewIcon signal received");
+                // margo patch: Electron/Chromium tray icons live in a fresh temp
+                // dir per update and never emit PropertiesChanged, so re-read
+                // IconThemePath here — before the name — or the cached path
+                // points at a deleted dir. Refreshing it first keeps the
+                // (path, name) pair consistent for watchers.
+                if let Ok(path) = item_proxy.icon_theme_path().await {
+                    let icon_theme_path = if path.is_empty() { None } else { Some(path) };
+                    tray_item.icon_theme_path.set(icon_theme_path);
+                }
                 if new_icon_has_name_property {
                     match item_proxy.icon_name().await {
                         Ok(name) => {
```
