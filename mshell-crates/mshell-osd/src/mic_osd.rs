//! Microphone OSD — the input-side twin of [`crate::volume_osd`].
//!
//! Pops the same capsule when the default audio *source* (microphone)
//! mute/level changes — e.g. the `XF86AudioMicMute` key routed through
//! `mshellctl audio mic-mute`. The icon flips to the muted-mic glyph via
//! [`get_audio_in_icon`] and the value reads "Off", giving the toggle the same
//! visual feedback the volume keys already get. Watches the device
//! reactively, so it reflects changes from any source, not just our keys.

use crate::level_osd::{LevelOsdCommand, LevelOsdInit, LevelOsdModel, LevelSource, OsdLevel};
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, spawn_default_input_watcher, spawn_input_device_volume_mute_watcher,
};
use relm4::ComponentSender;
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;

/// Level source for the default input device.
pub struct MicSource;

pub type MicOsdModel = LevelOsdModel<MicSource>;
pub type MicOsdInit = LevelOsdInit;

/// A muted mic shows an empty bar and "Off" rather than the (still set)
/// capture volume, so the mute toggle reads unambiguously.
fn mic_level(icon: &'static str, muted: bool, average: f64, average_percent: f64) -> OsdLevel {
    if muted {
        OsdLevel {
            icon,
            fraction: 0.0,
            label: "Off".to_string(),
        }
    } else {
        crate::level_osd::audio_level(icon, average, average_percent)
    }
}

fn level(device: &Arc<InputDevice>) -> OsdLevel {
    let volume = device.volume.get();
    mic_level(
        get_audio_in_icon(device),
        device.muted.get(),
        volume.average(),
        volume.average_percentage(),
    )
}

impl LevelSource for MicSource {
    const INITIAL_ICON: &'static str = "audio-input-microphone-symbolic";

    fn spawn_watchers(sender: &ComponentSender<LevelOsdModel<Self>>) {
        spawn_default_input_watcher(sender, None, || LevelOsdCommand::Rebind);
    }

    fn rebind(
        token: &mut WatcherToken,
        sender: &ComponentSender<LevelOsdModel<Self>>,
    ) -> Option<OsdLevel> {
        let device = audio_service().default_input.get()?;
        spawn_input_device_volume_mute_watcher(&device, token.reset(), sender, || {
            LevelOsdCommand::Changed
        });
        Some(level(&device))
    }

    fn read() -> Option<OsdLevel> {
        audio_service()
            .default_input
            .get()
            .map(|device| level(&device))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muted_mic_shows_off_and_an_empty_bar() {
        let level = mic_level("muted", true, 0.8, 80.0);
        assert_eq!(level.fraction, 0.0);
        assert_eq!(level.label, "Off");
        assert_eq!(level.icon, "muted");
    }

    #[test]
    fn live_mic_shows_its_capture_volume() {
        let level = mic_level("live", false, 0.425, 42.5);
        assert_eq!(level.fraction, 0.425);
        assert_eq!(level.label, "43%");
    }
}
