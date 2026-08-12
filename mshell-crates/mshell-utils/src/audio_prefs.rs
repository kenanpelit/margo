//! Persistent per-device audio preferences: display alias, hidden-from-
//! cycling flag, and a Bluetooth keybind number.
//!
//! Small JSON file at `$XDG_CACHE_HOME/margo/audio_device_prefs.json`,
//! keyed by device *name* (the same stable string identifier
//! `pick_device`/`next_index`/`routable_outputs` already key by — wayle-
//! audio's `DeviceKey` is a WirePlumber object id that changes across
//! reconnects, `name` is what survives). Mirrors mshell-launcher's
//! `HiddenStore`: atomic temp+rename JSON writes, best-effort.
//!
//! A device with every field at its default is pruned from the map on
//! write, so the file only ever grows with devices someone actually
//! customized.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevicePrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bt_number: Option<u8>,
}

impl DevicePrefs {
    fn is_default(&self) -> bool {
        *self == DevicePrefs::default()
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Disk {
    #[serde(default)]
    devices: BTreeMap<String, DevicePrefs>,
}

pub struct AudioPrefsStore {
    path: PathBuf,
    map: BTreeMap<String, DevicePrefs>,
}

impl AudioPrefsStore {
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Disk>(&raw).ok())
            .map(|d| d.devices)
            .unwrap_or_default();
        Self { path, map }
    }

    /// Preferences for `device_name`, or the all-defaults value if it's
    /// never been customized.
    pub fn get(&self, device_name: &str) -> DevicePrefs {
        self.map.get(device_name).cloned().unwrap_or_default()
    }

    pub fn set_alias(&mut self, device_name: &str, alias: Option<String>) {
        self.update(device_name, |p| p.alias = alias);
    }

    pub fn set_hidden(&mut self, device_name: &str, hidden: bool) {
        self.update(device_name, |p| p.hidden = hidden);
    }

    pub fn set_bt_number(&mut self, device_name: &str, number: Option<u8>) {
        self.update(device_name, |p| p.bt_number = number);
    }

    /// The device name assigned to `number`, if any.
    pub fn device_for_bt_number(&self, number: u8) -> Option<String> {
        self.map
            .iter()
            .find(|(_, p)| p.bt_number == Some(number))
            .map(|(name, _)| name.clone())
    }

    /// Smallest positive number not already assigned to another device —
    /// what a device gets auto-assigned on its first successful connect.
    pub fn next_free_bt_number(&self) -> u8 {
        let used: std::collections::BTreeSet<u8> =
            self.map.values().filter_map(|p| p.bt_number).collect();
        (1..=u8::MAX).find(|n| !used.contains(n)).unwrap_or(1)
    }

    fn update(&mut self, device_name: &str, f: impl FnOnce(&mut DevicePrefs)) {
        let mut prefs = self.get(device_name);
        f(&mut prefs);
        if prefs.is_default() {
            self.map.remove(device_name);
        } else {
            self.map.insert(device_name.to_string(), prefs);
        }
        self.flush();
    }

    fn flush(&self) {
        if let Some(parent) = self.path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(path = %parent.display(), error = %err, "audio_device_prefs: mkdir failed");
            return;
        }
        let disk = Disk {
            devices: self.map.clone(),
        };
        let json = match serde_json::to_string_pretty(&disk) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "audio_device_prefs: serialize failed");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, &json) {
            tracing::warn!(path = %tmp.display(), error = %err, "audio_device_prefs: tmp write failed");
            return;
        }
        if let Err(err) = std::fs::rename(&tmp, &self.path) {
            tracing::warn!(from = %tmp.display(), to = %self.path.display(), error = %err, "audio_device_prefs: rename failed");
        }
    }
}

fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("audio_device_prefs.json")
}

static STORE: OnceLock<Mutex<AudioPrefsStore>> = OnceLock::new();

/// The shared store, loaded once on first access. Every audio UI
/// (dashboard, Settings → Sound, the BT-connect-by-number command) reads
/// and writes through this single instance so a rename/hide/number made
/// in one place is immediately visible everywhere else.
pub fn audio_prefs() -> &'static Mutex<AudioPrefsStore> {
    STORE.get_or_init(|| Mutex::new(AudioPrefsStore::load()))
}

/// Lock the shared store, recovering rather than panicking if some other
/// caller already panicked while holding it — a poisoned lock's *data* is
/// still perfectly usable for a small preferences map like this one, and
/// a UI callback is the wrong place to propagate an unrelated panic.
pub fn lock_prefs() -> std::sync::MutexGuard<'static, AudioPrefsStore> {
    audio_prefs().lock().unwrap_or_else(|e| e.into_inner())
}

/// True if `device_name` was hidden from cycling — the check every
/// switch/cycle/route command filters on. A poisoned lock (only possible
/// after some other caller already panicked mid-mutation) degrades to
/// "not hidden" rather than propagating the panic into an audio switch.
pub fn is_hidden(device_name: &str) -> bool {
    audio_prefs()
        .lock()
        .map(|s| s.get(device_name).hidden)
        .unwrap_or(false)
}

/// `device_name`'s alias if one is set, else `fallback` (typically the
/// device's own wayle-audio description).
pub fn display_alias(device_name: &str, fallback: &str) -> String {
    audio_prefs()
        .lock()
        .ok()
        .and_then(|s| s.get(device_name).alias)
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ephemeral() -> AudioPrefsStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_audio_prefs_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        AudioPrefsStore::load_from(path)
    }

    #[test]
    fn unknown_device_is_all_defaults() {
        let s = ephemeral();
        assert_eq!(s.get("Family 17h Analog"), DevicePrefs::default());
    }

    #[test]
    fn alias_round_trips() {
        let mut s = ephemeral();
        s.set_alias("Family 17h Analog", Some("Speakers".to_string()));
        assert_eq!(
            s.get("Family 17h Analog").alias.as_deref(),
            Some("Speakers")
        );
    }

    #[test]
    fn clearing_every_field_prunes_the_entry() {
        let mut s = ephemeral();
        s.set_alias("dev", Some("Name".to_string()));
        assert_eq!(s.map.len(), 1);
        s.set_alias("dev", None);
        assert_eq!(s.map.len(), 0, "an all-default entry must not linger");
    }

    #[test]
    fn hidden_survives_alongside_alias() {
        let mut s = ephemeral();
        s.set_alias("dev", Some("Name".to_string()));
        s.set_hidden("dev", true);
        let prefs = s.get("dev");
        assert_eq!(prefs.alias.as_deref(), Some("Name"));
        assert!(prefs.hidden);
    }

    #[test]
    fn bt_number_lookup_is_bidirectional() {
        let mut s = ephemeral();
        s.set_bt_number("headphones-mac", Some(3));
        assert_eq!(s.get("headphones-mac").bt_number, Some(3));
        assert_eq!(s.device_for_bt_number(3).as_deref(), Some("headphones-mac"));
        assert_eq!(s.device_for_bt_number(4), None);
    }

    #[test]
    fn next_free_bt_number_skips_taken_slots() {
        let mut s = ephemeral();
        s.set_bt_number("a", Some(1));
        s.set_bt_number("b", Some(2));
        assert_eq!(s.next_free_bt_number(), 3);
        s.set_bt_number("c", Some(3));
        s.set_bt_number("b", None); // free up 2
        assert_eq!(s.next_free_bt_number(), 2);
    }

    #[test]
    fn survives_reload() {
        let path = std::env::temp_dir().join(format!(
            "mshell_audio_prefs_reload_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        let mut s = AudioPrefsStore::load_from(path.clone());
        s.set_alias("dev", Some("Speakers".to_string()));
        s.set_hidden("other", true);
        drop(s);
        let s2 = AudioPrefsStore::load_from(path);
        assert_eq!(s2.get("dev").alias.as_deref(), Some("Speakers"));
        assert!(s2.get("other").hidden);
    }
}
