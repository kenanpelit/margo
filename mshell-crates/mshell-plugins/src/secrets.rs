//! System-keyring storage for `type = "secret"` plugin settings.
//!
//! Plugin manifests can mark a setting as `type = "secret"` (e.g. an API
//! key). On disk in `plugins.toml` those values used to live in plaintext
//! beside the source list — the file was 0600 but any tool reading it (a
//! backup, dotfiles repo, screen share) leaked the secret.
//!
//! With this module, secret values live in the Secret Service (gnome-keyring
//! / kde-wallet) instead. `plugins.toml` never sees them. The other settings
//! (model name, terminal choice, …) still round-trip through TOML as before.
//!
//! Failures degrade loudly — reading falls back to `None` with a warning,
//! writing returns an error the caller surfaces — so a misconfigured
//! keyring is visible, not silent.

use keyring::Entry;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Values already read from (or written to) the keyring, by
/// `(plugin, setting)`. `None` = known to be unset. Config loads and saves
/// read secrets from here only ([`cached`]) — a Secret Service round trip
/// (and a possible unlock prompt) on the GTK main thread on every settings
/// toggle was the cost before.
type Cache = RwLock<HashMap<(String, String), Option<String>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Record a value in the cache (what [`read`] / [`write`] do; exposed for a
/// prefetch that already holds the value).
pub fn remember(plugin_key: &str, setting_key: &str, value: Option<String>) {
    cache()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert((plugin_key.to_string(), setting_key.to_string()), value);
}

/// The cached value without touching the keyring: `None` when this secret
/// was never read, `Some(None)` when it is known to be unset.
pub fn cached(plugin_key: &str, setting_key: &str) -> Option<Option<String>> {
    cache()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(plugin_key.to_string(), setting_key.to_string()))
        .cloned()
}

/// The service name we identify ourselves with in the Secret Service.
/// Visible in `seahorse` / similar tools so the user can audit what's stored.
const SERVICE: &str = "mshell-plugin";

fn entry(plugin_key: &str, setting_key: &str) -> Result<Entry, keyring::Error> {
    // `user` follows `<plugin-composite-key>/<setting-key>` so secrets cluster
    // by plugin in the user's keyring browser.
    Entry::new(SERVICE, &format!("{plugin_key}/{setting_key}"))
}

/// Read a secret from the keyring. `None` for "not present" *or* any read
/// error (logged at `warn`) so the caller treats both as "no value yet".
///
/// Blocking (a D-Bus round trip): call it off the GTK main thread. The
/// result is cached for [`cached`].
pub fn read(plugin_key: &str, setting_key: &str) -> Option<String> {
    match entry(plugin_key, setting_key).and_then(|e| e.get_password()) {
        Ok(s) => {
            remember(plugin_key, setting_key, Some(s.clone()));
            Some(s)
        }
        Err(keyring::Error::NoEntry) => {
            remember(plugin_key, setting_key, None);
            None
        }
        Err(e) => {
            tracing::warn!(
                plugin = plugin_key,
                setting = setting_key,
                "keyring read failed: {e}"
            );
            // Cached as unset so a broken keyring isn't retried on every
            // config load; `write` (Settings) refreshes it.
            remember(plugin_key, setting_key, None);
            None
        }
    }
}

/// Persist a secret value to the keyring. Empty string deletes the entry
/// (so clearing the field in Settings actually removes the secret).
pub fn write(plugin_key: &str, setting_key: &str, value: &str) -> Result<(), keyring::Error> {
    let entry = entry(plugin_key, setting_key)?;
    if value.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e),
        }
        remember(plugin_key, setting_key, None);
    } else {
        entry.set_password(value)?;
        remember(plugin_key, setting_key, Some(value.to_string()));
    }
    Ok(())
}

/// Drop a plugin's secret on uninstall, ignoring "not present".
pub fn delete(plugin_key: &str, setting_key: &str) {
    remember(plugin_key, setting_key, None);
    if let Ok(entry) = entry(plugin_key, setting_key) {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => tracing::warn!(
                plugin = plugin_key,
                setting = setting_key,
                "keyring delete failed: {e}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_distinguishes_unknown_from_unset() {
        assert_eq!(cached("test/p", "never-read"), None);
        remember("test/p", "k", Some("v".into()));
        assert_eq!(cached("test/p", "k"), Some(Some("v".into())));
        remember("test/p", "k", None);
        assert_eq!(cached("test/p", "k"), Some(None));
    }
}
