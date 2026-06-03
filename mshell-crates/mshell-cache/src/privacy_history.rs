//! Persisted access log for the Privacy indicator pill.
//!
//! A small reactive singleton (mirrors [`crate::hidden_apps`]) holding the
//! most recent microphone / camera / screen-share start & stop events, so
//! the pill's panel can show "what touched my sensors and when" across
//! shell restarts. The detection engine (in `mshell-frame`) records edges
//! here; the panel widget reads the store reactively and offers a clear.
//!
//! The caller supplies the pre-formatted `time` string and `timestamp`
//! (epoch seconds) so this crate stays free of a date/time dependency.

use reactive_graph::prelude::{ReadUntracked, Update};
use reactive_stores::{ArcStore, Store};
use relm4::gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

/// One sensor access event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyEvent {
    /// Application name that used the sensor.
    pub app: String,
    /// Sensor kind — `"Microphone"`, `"Camera"`, or `"Screen"`.
    pub kind: String,
    /// `"started"` or `"stopped"`.
    pub action: String,
    /// Human time-of-day for display (e.g. `"14:07:32"`).
    pub time: String,
    /// Epoch seconds — used only for ordering / pruning.
    pub timestamp: i64,
}

impl PrivacyEvent {
    /// Symbolic icon name for the event's sensor kind.
    pub fn icon_name(&self) -> &'static str {
        match self.kind.as_str() {
            "Camera" => "camera-video-symbolic",
            "Screen" => "video-display-symbolic",
            _ => "microphone-sensitivity-high-symbolic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Store)]
pub struct PrivacyHistoryState {
    /// Newest event first.
    pub events: Vec<PrivacyEvent>,
}

static PRIVACY_HISTORY: LazyLock<ArcStore<PrivacyHistoryState>> = LazyLock::new(|| {
    ArcStore::new(PrivacyHistoryState {
        events: load_history(),
    })
});

pub fn privacy_history_store() -> ArcStore<PrivacyHistoryState> {
    PRIVACY_HISTORY.clone()
}

/// Prepend an event, cap the log at `limit`, and persist. `limit == 0`
/// disables history entirely (also clears any existing log on disk).
pub fn push_event(event: PrivacyEvent, limit: usize) {
    let store = privacy_history_store();
    if limit == 0 {
        if !store.read_untracked().events.is_empty() {
            store.update(|s| s.events.clear());
            persist();
        }
        return;
    }
    store.update(|s| {
        s.events.insert(0, event);
        if s.events.len() > limit {
            s.events.truncate(limit);
        }
    });
    persist();
}

pub fn clear_history() {
    let store = privacy_history_store();
    if store.read_untracked().events.is_empty() {
        return;
    }
    store.update(|s| s.events.clear());
    persist();
}

fn persist() {
    let events = privacy_history_store().read_untracked().events.clone();
    if let Err(e) = save_history(&events) {
        tracing::warn!("privacy_history: failed to save: {e}");
    }
}

fn history_path() -> PathBuf {
    glib::user_cache_dir()
        .join("mshell")
        .join("privacy_history.json")
}

fn load_history() -> Vec<PrivacyEvent> {
    match fs::read_to_string(history_path()) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_history(events: &[PrivacyEvent]) -> std::io::Result<()> {
    let path = history_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(events).map_err(std::io::Error::other)?;
    fs::write(&path, json)
}
