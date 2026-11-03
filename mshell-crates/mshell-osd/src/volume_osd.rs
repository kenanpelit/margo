//! Volume OSD — the default audio sink's level and mute state.

use crate::level_osd::{LevelOsdCommand, LevelOsdInit, LevelOsdModel, LevelSource, OsdLevel};
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_out_icon, spawn_default_output_watcher, spawn_output_device_volume_mute_watcher,
};
use relm4::ComponentSender;
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;

/// Level source for the default output device.
pub struct VolumeSource;

pub type VolumeOsdModel = LevelOsdModel<VolumeSource>;
pub type VolumeOsdInit = LevelOsdInit;

fn level(device: &Arc<OutputDevice>) -> OsdLevel {
    let volume = device.volume.get();
    crate::level_osd::audio_level(
        get_audio_out_icon(device),
        volume.average(),
        volume.average_percentage(),
    )
}

impl LevelSource for VolumeSource {
    const INITIAL_ICON: &'static str = "audio-volume-medium-symbolic";

    fn spawn_watchers(sender: &ComponentSender<LevelOsdModel<Self>>) {
        spawn_default_output_watcher(sender, None, || LevelOsdCommand::Rebind);
    }

    fn rebind(
        token: &mut WatcherToken,
        sender: &ComponentSender<LevelOsdModel<Self>>,
    ) -> Option<OsdLevel> {
        let device = audio_service().default_output.get()?;
        spawn_output_device_volume_mute_watcher(&device, token.reset(), sender, || {
            LevelOsdCommand::Changed
        });
        Some(level(&device))
    }

    fn read() -> Option<OsdLevel> {
        audio_service()
            .default_output
            .get()
            .map(|device| level(&device))
    }
}
