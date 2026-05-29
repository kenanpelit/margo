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
pub fn read(plugin_key: &str, setting_key: &str) -> Option<String> {
    match entry(plugin_key, setting_key).and_then(|e| e.get_password()) {
        Ok(s) => Some(s),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            tracing::warn!(
                plugin = plugin_key,
                setting = setting_key,
                "keyring read failed: {e}"
            );
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
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        entry.set_password(value)
    }
}

/// Drop a plugin's secret on uninstall, ignoring "not present".
pub fn delete(plugin_key: &str, setting_key: &str) {
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
