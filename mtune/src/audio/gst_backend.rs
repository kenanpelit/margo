// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::Cell;

use async_channel::Sender;
use glib::clone;
use gst::prelude::*;
use gtk::glib;
use log::{debug, error, warn};

use crate::audio::{PlaybackAction, ReplayGainMode, SeekDirection};

/// Playback-rate bounds (also mirrored to MPRIS `MinimumRate`/`MaximumRate`).
pub const MIN_RATE: f64 = 0.5;
pub const MAX_RATE: f64 = 2.0;

#[derive(Debug)]
pub struct GstBackend {
    sender: Sender<PlaybackAction>,
    gst_player: gst_play::Play,
    replaygain: Option<GstReplayGain>,
    /// Sticky playback rate — `gst_play::Play` resets to 1.0 on `set_uri`,
    /// so we re-apply this after every track change.
    rate: Cell<f64>,
}

/// The `playbin` `audio-filter`. Always routes audio through a
/// `scaletempo` element so a non-1.0 playback rate changes tempo
/// **without shifting pitch** (like mpv's default) — plain `playbin`
/// rate changes are pitch-shifting seeks. ReplayGain's `rgvolume` /
/// `rglimiter` are chained after it when enabled.
#[derive(Debug)]
pub struct GstReplayGain {
    /// audio-filter when ReplayGain is off: `scaletempo` alone.
    plain: gst::Element,
    /// audio-filter when on: `scaletempo ! rgvolume ! rglimiter`.
    rg_bin: gst::Element,
    rg_volume: gst::Element,
}

/// A fresh `scaletempo`, falling back to `identity` where the
/// `audiofx` plugin is missing (rate still works, pitch still shifts —
/// today's behaviour).
fn tempo_element(name: &str) -> Result<gst::Element, Box<dyn std::error::Error>> {
    if let Ok(e) = gst::ElementFactory::make_with_name("scaletempo", Some(name)) {
        return Ok(e);
    }
    warn!("mtune: `scaletempo` element unavailable — speed changes will shift pitch");
    Ok(gst::ElementFactory::make_with_name("identity", Some(name))?)
}

fn send_update_position(sender: &Sender<PlaybackAction>, clock: gst::ClockTime, notify: bool) {
    let pos = clock.seconds();
    if let Err(e) = sender.send_blocking(PlaybackAction::UpdatePosition(pos, notify)) {
        error!("Failed to send UpdatePosition({pos}): {e}");
    }
}

impl GstReplayGain {
    pub fn new() -> Result<GstReplayGain, Box<dyn std::error::Error>> {
        let plain = tempo_element("tempo-plain")?;

        let tempo = tempo_element("tempo-rg")?;
        let rg_volume = gst::ElementFactory::make_with_name("rgvolume", Some("rg volume"))?;
        let rg_limiter = gst::ElementFactory::make_with_name("rglimiter", Some("rg limiter"))?;

        let filter_bin = gst::Bin::builder().name("filter bin").build();
        filter_bin.add(&tempo)?;
        filter_bin.add(&rg_volume)?;
        filter_bin.add(&rg_limiter)?;
        gst::Element::link_many([&tempo, &rg_volume, &rg_limiter])?;

        let pad_src = rg_limiter
            .static_pad("src")
            .ok_or("rglimiter has no src pad")?;
        pad_src.set_active(true)?;
        let ghost_src = gst::GhostPad::with_target(&pad_src)?;
        filter_bin.add_pad(&ghost_src)?;

        let pad_sink = tempo
            .static_pad("sink")
            .ok_or("scaletempo has no sink pad")?;
        pad_sink.set_active(true)?;
        let ghost_sink = gst::GhostPad::with_target(&pad_sink)?;
        filter_bin.add_pad(&ghost_sink)?;

        Ok(Self {
            plain,
            rg_bin: filter_bin.upcast(),
            rg_volume,
        })
    }

    pub fn set_mode(&self, playbin: gst::Element, replaygain: ReplayGainMode) {
        let (filter, album_mode) = match replaygain {
            ReplayGainMode::Album => (&self.rg_bin, true),
            ReplayGainMode::Track => (&self.rg_bin, false),
            ReplayGainMode::Off => (&self.plain, true),
        };

        self.rg_volume.set_property("album-mode", album_mode);
        playbin.set_property("audio-filter", filter);
    }
}

impl GstBackend {
    pub fn new(sender: Sender<PlaybackAction>) -> Self {
        let gst_player = gst_play::Play::default();

        gst_player.set_video_track_enabled(false);

        let mut config = gst_player.config();
        config.set_position_update_interval(250);
        gst_player.set_config(config).unwrap();

        let res = Self {
            sender,
            gst_player,
            replaygain: GstReplayGain::new().ok(),
            rate: Cell::new(1.0),
        };

        res.setup_signals();

        // Put `scaletempo` in the audio path from the start — before the
        // window pushes the persisted ReplayGain mode — so the very
        // first speed change is already pitch-preserving.
        if let Some(ref rg) = res.replaygain {
            rg.set_mode(res.gst_player.pipeline(), ReplayGainMode::Off);
        }

        res
    }

    fn setup_signals(&self) {
        let bus = self.gst_player.message_bus();
        bus.set_sync_handler(clone!(
            #[strong(rename_to = sender)]
            self.sender,
            move |_bus, msg| {
                let Ok(play_msg) = gst_play::PlayMessage::parse(msg) else {
                    return gst::BusSyncReply::Drop;
                };

                match play_msg {
                    gst_play::PlayMessage::Error(message) => {
                        error!("GStreamer error: {}", message.error());
                    }
                    gst_play::PlayMessage::Warning(message) => {
                        warn!("GStreamer warning: {}", message.error());
                    }
                    gst_play::PlayMessage::EndOfStream(_) => {
                        if let Err(e) = sender.send_blocking(PlaybackAction::PlayNext) {
                            error!("Failed to send PlayNext: {e}");
                        }
                    }
                    gst_play::PlayMessage::StateChanged(message) => {
                        // Preroll finished — a queued resume seek can land now.
                        if matches!(
                            message.state(),
                            gst_play::PlayState::Paused | gst_play::PlayState::Playing
                        ) {
                            let _ = sender.send_blocking(PlaybackAction::PipelineReady);
                        }
                    }
                    gst_play::PlayMessage::PositionUpdated(message) => {
                        if let Some(position) = message.position() {
                            send_update_position(&sender, position, false);
                        }
                    }
                    gst_play::PlayMessage::SeekDone(message) => {
                        if let Some(position) = message.position() {
                            send_update_position(&sender, position, true);
                        }
                    }
                    gst_play::PlayMessage::VolumeChanged(message) => {
                        let volume = gst_audio::StreamVolume::convert_volume(
                            gst_audio::StreamVolumeFormat::Linear,
                            gst_audio::StreamVolumeFormat::Cubic,
                            message.volume(),
                        );
                        if let Err(e) = sender.send_blocking(PlaybackAction::VolumeChanged(volume))
                        {
                            error!("Failed to send VolumeChanged({volume}): {e}");
                        }
                    }
                    _ => {}
                }

                gst::BusSyncReply::Drop
            }
        ));
    }

    pub fn set_song_uri(&self, uri: Option<&str>) {
        self.gst_player.set_uri(uri);
        // `set_uri` drops back to 1.0 — restore the user's chosen rate.
        if uri.is_some() && (self.rate.get() - 1.0).abs() > f64::EPSILON {
            self.gst_player.set_rate(self.rate.get());
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate.get()
    }

    pub fn set_rate(&self, rate: f64) {
        let rate = rate.clamp(MIN_RATE, MAX_RATE);
        self.rate.set(rate);
        self.gst_player.set_rate(rate);
    }

    pub fn seek(&self, position: u64, duration: u64, offset: u64, direction: SeekDirection) {
        let offset = gst::ClockTime::from_seconds(offset);
        let position = gst::ClockTime::from_seconds(position);
        let duration = gst::ClockTime::from_seconds(duration);

        let destination = match direction {
            SeekDirection::Backwards if position >= offset => position.checked_sub(offset),
            SeekDirection::Backwards if position < offset => Some(gst::ClockTime::from_seconds(0)),
            SeekDirection::Forward if !duration.is_zero() && position + offset <= duration => {
                position.checked_add(offset)
            }
            SeekDirection::Forward if !duration.is_zero() && position + offset > duration => {
                Some(duration)
            }
            _ => None,
        };

        if let Some(destination) = destination {
            self.gst_player.seek(destination);
        }
    }

    pub fn seek_position(&self, position: u64) {
        self.gst_player.seek(gst::ClockTime::from_seconds(position));
    }

    pub fn seek_start(&self) {
        self.gst_player.seek(gst::ClockTime::from_seconds(0));
    }

    pub fn play(&self) {
        self.gst_player.play();
    }

    pub fn pause(&self) {
        self.gst_player.pause();
    }

    pub fn stop(&self) {
        self.gst_player.stop();
    }

    pub fn set_volume(&self, volume: f64) {
        let linear_volume = gst_audio::StreamVolume::convert_volume(
            gst_audio::StreamVolumeFormat::Cubic,
            gst_audio::StreamVolumeFormat::Linear,
            volume,
        );
        debug!("Setting volume to: {}", &linear_volume);
        self.gst_player.set_volume(linear_volume);
    }

    pub fn set_replaygain(&self, replaygain: ReplayGainMode) {
        if let Some(ref r) = self.replaygain {
            r.set_mode(self.gst_player.pipeline(), replaygain);
        }
    }

    pub fn replaygain_available(&self) -> bool {
        self.replaygain.is_some()
    }
}
