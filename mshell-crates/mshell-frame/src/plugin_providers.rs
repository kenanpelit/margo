//! Wayle-backed [`MediaInfoSource`] + [`SystemInfoSource`] for the WASM
//! plugin runtime. The plugin host stays GTK-/wayle-free; the shell hands
//! it concrete implementations at construction time so guests calling
//! `media-now-playing` / `system-state` see the same data the bar widgets
//! see.
//!
//! Reads are always `get_untracked` — we're called from a guest's `view`
//! or `update` on the GTK main thread, outside any reactive scope, so
//! we want a one-shot snapshot, never a subscription.
#![cfg(feature = "wasm-plugins")]

use mshell_plugin_host::{MediaInfo, MediaInfoSource, SystemInfo, SystemInfoSource};
use mshell_services::{battery_service, media_service, network_service};
use wayle_battery::types::DeviceState;
use wayle_media::types::PlaybackState;
use wayle_network::types::connectivity::ConnectionType;

pub struct WayleMediaProvider;

impl MediaInfoSource for WayleMediaProvider {
    fn snapshot(&self) -> MediaInfo {
        let svc = media_service();
        // Prefer a player that's actually *playing* (so "what's playing" wins
        // even when wayle's active player is a different, paused one — e.g.
        // Spotify playing while a paused browser tab is active); then the
        // active player; then any player at all.
        let list = svc.player_list.get();
        let player = list
            .iter()
            .find(|p| p.playback_state.get() == PlaybackState::Playing)
            .cloned()
            .or_else(|| svc.active_player.get())
            .or_else(|| list.into_iter().next());
        let Some(p) = player else {
            return MediaInfo::default();
        };
        MediaInfo {
            // The playerctl-style name (bus suffix), e.g. "spotify" — lets a
            // guest target this exact player with `playerctl -p <name> …`.
            player: p
                .id
                .bus_name()
                .strip_prefix("org.mpris.MediaPlayer2.")
                .unwrap_or_else(|| p.id.bus_name())
                .to_string(),
            title: p.metadata.title.get(),
            artist: p.metadata.artist.get(),
            album: p.metadata.album.get(),
            art_url: p.metadata.cover_art.get().unwrap_or_default(),
            status: match p.playback_state.get() {
                PlaybackState::Playing => "playing".to_string(),
                PlaybackState::Paused => "paused".to_string(),
                PlaybackState::Stopped => "stopped".to_string(),
            },
            // Wire the live position + track length (were hardcoded 0, which
            // froze synced-lyrics highlights and sent duration=0 to lrclib).
            position_ms: p.position.get().as_millis() as u64,
            length_ms: p
                .metadata
                .length
                .get()
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

pub struct WayleSystemProvider;

impl SystemInfoSource for WayleSystemProvider {
    fn snapshot(&self) -> SystemInfo {
        let bat = battery_service();
        let battery_pct = if bat.device.is_present.get() {
            let pct = bat.device.percentage.get();
            pct.round().clamp(0.0, 100.0) as u8
        } else {
            255
        };
        let battery_status = match bat.device.state.get() {
            DeviceState::Charging => "charging".to_string(),
            DeviceState::Discharging => "discharging".to_string(),
            DeviceState::FullyCharged => "full".to_string(),
            _ => "unknown".to_string(),
        };
        // Primary connection kind via wayle-network; SSID via the wifi
        // device when wifi is the primary. Idle in ms isn't exposed by
        // mshell-idle (it's a stage manager, not a counter) — leave 0
        // until that surface grows.
        let net = network_service();
        let (network_kind, network_ssid) = match net.primary.get() {
            ConnectionType::Wifi => {
                let ssid = net
                    .wifi
                    .get()
                    .and_then(|w| w.ssid.get())
                    .unwrap_or_default();
                ("wifi".to_string(), ssid)
            }
            ConnectionType::Wired => ("ethernet".to_string(), String::new()),
            ConnectionType::Bluetooth => ("bluetooth".to_string(), String::new()),
            _ => ("none".to_string(), String::new()),
        };
        SystemInfo {
            battery_pct,
            battery_status,
            network_kind,
            network_ssid,
            idle_ms: 0,
        }
    }
}
