//! Publishes a tiny "lock info" sidecar the standalone locker (`mlock`)
//! reads — notification count, current weather, and now-playing media —
//! so the lock screen can show live desktop context it can't compute on
//! its own (it's a separate process with no service access).
//!
//! Written to `<cache>/margo/lock-info` as hand-parsed key=value (mlock
//! stays serde-free; see `mlock/src/sidecar.rs`). Refreshed on a short
//! timer while the session is locked, rather than via per-service reactive
//! effects — simpler; mlock re-reads it every couple of seconds while
//! locked. The timer is armed by margo's pushed lock state, so nothing
//! runs while unlocked.

use std::time::Duration;

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_services::{media_service, notification_service, weather_service};
use mshell_session::session_lock::{compositor_locked, session_locked};
use mshell_utils::weather::get_temperature_string;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use wayle_media::types::PlaybackState;
use wayle_weather::TemperatureUnit;

const REFRESH: Duration = Duration::from_secs(3);

/// When the refresh loop should next run, given the compositor's lock state.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Unlocked: nothing reads the sidecar, sleep until the state changes.
    WaitForChange,
    /// Locked, or unknown (an older margo; `session_locked()` then falls
    /// back to looking for our mlock): refresh, then again after [`REFRESH`].
    Refresh,
}

fn next_step(compositor_locked: Option<bool>) -> Next {
    match compositor_locked {
        Some(false) => Next::WaitForChange,
        Some(true) | None => Next::Refresh,
    }
}

/// Start the refresh loop. Call once at shell startup (it owns its own
/// state). It only wakes while the session is locked: only the lock screen
/// (mlock) reads this sidecar, and the lock state is pushed by margo, so
/// the unlocked case costs nothing.
pub fn start() {
    glib::spawn_future_local(async move {
        let mut state = compositor_locked();
        let mut last = String::new();
        loop {
            let reported = *state.borrow_and_update();
            match next_step(reported) {
                Next::WaitForChange => {
                    if state.changed().await.is_err() {
                        return;
                    }
                }
                Next::Refresh => {
                    if session_locked() {
                        let body = build();
                        if last != body {
                            if let Err(e) = write(&body) {
                                tracing::debug!("lock_info: write failed: {e}");
                            }
                            last = body;
                        }
                    }
                    let _ = glib::future_with_timeout(REFRESH, state.changed()).await;
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_refreshes_only_while_locked_or_unknown() {
        assert_eq!(next_step(Some(false)), Next::WaitForChange);
        assert_eq!(next_step(Some(true)), Next::Refresh);
        assert_eq!(next_step(None), Next::Refresh);
    }
}
