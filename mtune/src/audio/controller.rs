// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::audio::{PlaybackState, RepeatMode, Song};

pub trait Controller {
    fn set_playback_state(&self, state: &PlaybackState);

    fn set_song(&self, song: &Song);
    fn set_position(&self, position: u64, notify: bool);
    fn set_repeat_mode(&self, repeat: RepeatMode);

    /// Playback rate changed (default: nothing — only MPRIS cares).
    fn set_playback_rate(&self, _rate: f64) {}
}
