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
use mshell_services::{battery_service, media_service};
use wayle_battery::types::DeviceState;
use wayle_media::types::PlaybackState;

pub struct WayleMediaProvider;

impl MediaInfoSource for WayleMediaProvider {
    fn snapshot(&self) -> MediaInfo {
        let svc = media_service();
        // Prefer wayle's active player; otherwise the first one playing in
        // the list; otherwise any player at all.
        let player = svc.active_player.get().or_else(|| {
            let list = svc.player_list.get();
            list.iter()
                .find(|p| p.playback_state.get() == PlaybackState::Playing)
                .cloned()
                .or_else(|| list.into_iter().next())
        });
        let Some(p) = player else {
            return MediaInfo::default();
        };
        MediaInfo {
            player: String::new(),
            title: p.metadata.title.get(),
            artist: p.metadata.artist.get(),
            album: p.metadata.album.get(),
            art_url: p.metadata.cover_art.get().unwrap_or_default(),
            status: match p.playback_state.get() {
                PlaybackState::Playing => "playing".to_string(),
                PlaybackState::Paused => "paused".to_string(),
                PlaybackState::Stopped => "stopped".to_string(),
            },
            position_ms: 0,
            length_ms: 0,
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
        // network kind/SSID + idle: wayle exposes both but the field surface
        // is heavier to thread through; leave them as conservative defaults
        // for now (a follow-up wires them properly).
        SystemInfo {
            battery_pct,
            battery_status,
            network_kind: "none".to_string(),
            network_ssid: String::new(),
            idle_ms: 0,
        }
    }
}
