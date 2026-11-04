//! Hot reload for the WASM plugin tier: watch the plugin dirs, send `PluginReload`.

use super::*;

/// File-watcher hot reload for the WASM plugin tier. Watches the plugins
/// root and each plugin dir, adding a watch when a plugin is installed
/// later (Settings → Plugins); when a plugin's `plugin.wasm` changes (e.g. `cargo build`
/// drops a new build) we send `PluginReload(key)` so the cached panel is
/// evicted and the next open instantiates from disk — no mshell restart, no
/// manual `mshellctl plugin reload`. Runs on its own named thread (it blocks
/// for the session) rather than parking one of the runtime's blocking-pool
/// threads, and sleeps in `recv()` while idle.
pub(super) fn spawn_plugin_watcher_task(tx: mpsc::UnboundedSender<IPCCommand>) {
    let spawned = std::thread::Builder::new()
        .name("plugin-watch".into())
        .spawn(move || {
            use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
            use std::collections::HashSet;
            use std::sync::mpsc as sync_mpsc;
            use std::time::{Duration, Instant};

            let root = mshell_plugins::PluginStore::new().plugins_dir();
            if let Err(e) = std::fs::create_dir_all(&root) {
                tracing::warn!(dir = %root.display(), "plugin watcher: cannot create the plugins dir: {e}");
                return;
            }
            let (notify_tx, notify_rx) =
                sync_mpsc::channel::<notify::Result<notify::Event>>();
            let mut watcher = match RecommendedWatcher::new(
                move |res| {
                    let _ = notify_tx.send(res);
                },
                Config::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("plugin watcher init failed; hot reload is off: {e}");
                    return;
                }
            };
            // The root (to see plugins installed later) and each plugin dir,
            // non-recursively: a recursive watch would also cover every
            // `.git` subdirectory of a git-installed plugin.
            if let Err(e) = watcher.watch(&root, RecursiveMode::NonRecursive) {
                tracing::warn!(dir = %root.display(), "plugin watcher: watch failed; hot reload is off: {e}");
                return;
            }
            let mut watch_plugin_dir = |dir: &std::path::Path| {
                if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                    tracing::warn!(dir = %dir.display(), "plugin watcher: watch failed: {e}");
                }
            };
            for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    watch_plugin_dir(&entry.path());
                }
            }

            // Debounce: editors / cargo write the wasm in multiple steps
            // (tmp + rename). Collect events for 300 ms after the last one,
            // then flush one reload per key.
            let debounce = Duration::from_millis(300);
            let mut pending: HashSet<String> = HashSet::new();
            let mut last_event: Option<Instant> = None;

            loop {
                let recv = match last_event {
                    Some(t) => notify_rx.recv_timeout(debounce.saturating_sub(t.elapsed())),
                    None => notify_rx
                        .recv()
                        .map_err(|_| sync_mpsc::RecvTimeoutError::Disconnected),
                };
                match recv {
                    Ok(Ok(ev)) => {
                        if !matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            continue;
                        }
                        // A plugin installed (or re-cloned) since startup.
                        for p in &ev.paths {
                            if p.parent() == Some(root.as_path()) && p.is_dir() {
                                watch_plugin_dir(p);
                            }
                        }
                        for key in ev.paths.iter().filter_map(|p| plugin_wasm_key(&root, p)) {
                            pending.insert(key);
                            last_event = Some(Instant::now());
                        }
                    }
                    Ok(Err(e)) => tracing::debug!("plugin watcher event err: {e}"),
                    Err(sync_mpsc::RecvTimeoutError::Timeout) => {
                        for key in pending.drain() {
                            tracing::info!(plugin = %key, "hot-reload: plugin.wasm changed");
                            let _ = tx.send(IPCCommand::PluginReload(key));
                        }
                        last_event = None;
                    }
                    Err(sync_mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("plugin watcher thread failed to start; hot reload is off: {e}");
    }
}

/// The plugin key for a changed path: `<root>/<key>/plugin.wasm` → `key`.
pub(super) fn plugin_wasm_key(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    if path.file_name()? != "plugin.wasm" {
        return None;
    }
    let dir = path.parent()?.strip_prefix(root).ok()?;
    let mut parts = dir.components();
    let key = parts.next()?.as_os_str().to_str()?.to_string();
    parts.next().is_none().then_some(key)
}

#[cfg(test)]
mod plugin_watch_tests {
    use super::plugin_wasm_key;
    use std::path::Path;

    #[test]
    fn maps_a_wasm_change_to_its_plugin_key() {
        let root = Path::new("/c/plugins");
        assert_eq!(
            plugin_wasm_key(root, Path::new("/c/plugins/assistant/plugin.wasm")),
            Some("assistant".to_string())
        );
        // A plugin installed after startup is matched the same way.
        assert_eq!(
            plugin_wasm_key(root, Path::new("/c/plugins/new-one/plugin.wasm")),
            Some("new-one".to_string())
        );
        assert_eq!(
            plugin_wasm_key(root, Path::new("/c/plugins/a/manifest.toml")),
            None
        );
        assert_eq!(
            plugin_wasm_key(root, Path::new("/c/plugins/a/sub/plugin.wasm")),
            None
        );
        assert_eq!(
            plugin_wasm_key(root, Path::new("/elsewhere/a/plugin.wasm")),
            None
        );
    }
}
