//! Brightness OSD — the primary backlight's level.

use crate::level_osd::{LevelOsdCommand, LevelOsdInit, LevelOsdModel, LevelSource, OsdLevel};
use mshell_services::brightness_service;
use mshell_utils::brightness::{get_brightness_icon, spawn_brightness_watcher};
use relm4::ComponentSender;

/// Level source for the primary backlight device.
pub struct BrightnessSource;

pub type BrightnessOsdModel = LevelOsdModel<BrightnessSource>;
pub type BrightnessOsdInit = LevelOsdInit;

impl LevelSource for BrightnessSource {
    const INITIAL_ICON: &'static str = "brightness-medium-symbolic";

    fn spawn_watchers(sender: &ComponentSender<LevelOsdModel<Self>>) {
        spawn_brightness_watcher(sender, || LevelOsdCommand::Changed);
    }

    fn read() -> Option<OsdLevel> {
        let brightness = brightness_service()?.primary.get()?.percentage();
        Some(OsdLevel {
            icon: get_brightness_icon(brightness.value()),
            fraction: brightness.fraction(),
            label: crate::percent_label(brightness.value()),
        })
    }
}
