//! Publishes a tiny "lock info" sidecar the standalone locker (`mlock`)
//! reads — notification count, current weather, and now-playing media —
//! so the lock screen can show live desktop context it can't compute on
//! its own (it's a separate process with no service access).
//!
//! Written to `<cache>/margo/lock-info` as hand-parsed key=value (mlock
//! stays serde-free; see `mlock/src/sidecar.rs`). Refreshed on a short
//! timer rather than via per-service reactive effects — simpler, and the
//! file only needs to be current *by the time the screen locks*; mlock
//! then re-reads it every couple of seconds while locked.

use std::cell::RefCell;
use std::time::Duration;

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_services::{media_service, notification_service, weather_service};
use mshell_utils::weather::get_temperature_string;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use wayle_media::types::PlaybackState;
use wayle_weather::TemperatureUnit;

const REFRESH_SECS: u64 = 3;

/// Install the refresh timer. Idempotent-safe to call once at shell
/// startup (it owns its own state).
pub fn start() {
    let last = RefCell::new(String::new());
    glib::timeout_add_local(Duration::from_secs(REFRESH_SECS), move || {
        let body = build();
        if *last.borrow() != body {
            if let Err(e) = write(&body) {
                tracing::debug!("lock_info: write failed: {e}");
            }
            *last.borrow_mut() = body;
        }
        glib::ControlFlow::Continue
    });
}

fn build() -> String {
    let mut out = String::new();

    // Notifications — history count (0 when the service is unavailable).
    let n = notification_service()
        .map(|s| s.notifications.get().len())
        .unwrap_or(0);
    out.push_str(&format!("notifications={n}\n"));

    // Weather — current temperature (only when the service has loaded a
    // reading; otherwise the locker simply shows nothing).
    if let Some(weather) = weather_service().weather.get() {
        let unit: TemperatureUnit = config_manager()
            .config()
            .general()
            .temperature_unit()
            .get_untracked()
            .into();
        let temp = get_temperature_string(&weather.current.temperature, &unit);
        if !temp.is_empty() {
            out.push_str(&format!("weather={temp}\n"));
        }
    }

    // Now-playing — prefer a Playing player, else any non-stopped one.
    let players = media_service().player_list.get();
    let active = players
        .iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .or_else(|| {
            players
                .iter()
                .find(|p| p.playback_state.get() != PlaybackState::Stopped)
        });
    if let Some(p) = active {
        let title = sanitize(&p.metadata.title.get());
        let artist = sanitize(&p.metadata.artist.get());
        let playing = p.playback_state.get() == PlaybackState::Playing;
        if !title.is_empty() {
            out.push_str(&format!("media_title={title}\n"));
        }
        if !artist.is_empty() {
            out.push_str(&format!("media_artist={artist}\n"));
        }
        out.push_str(&format!("media_playing={}\n", if playing { 1 } else { 0 }));
    }

    out
}

/// Strip newlines so a track title can't inject extra key=value lines.
fn sanitize(s: &str) -> String {
    s.replace(['\n', '\r'], " ").trim().to_string()
}

fn write(body: &str) -> std::io::Result<()> {
    let path = glib::user_cache_dir().join("margo").join("lock-info");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}
