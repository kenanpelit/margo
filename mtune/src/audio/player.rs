// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    fmt::{self, Display, Formatter},
    rc::Rc,
};

use async_channel::{Receiver, Sender};
use glib::clone;
use gtk::glib;
use log::debug;

use crate::{
    application::ApplicationAction,
    audio::{
        Controller, CoverCache, GstBackend, InhibitController, MprisController, PlayerState, Queue,
        Song, WaveformGenerator,
    },
};

/// Messages the GStreamer backend pushes back to the player on its own
/// thread. User- and MPRIS-driven commands call `AudioPlayer` methods
/// directly, so this channel only carries backend-originated events.
#[derive(Clone, Debug)]
pub enum PlaybackAction {
    UpdatePosition(u64, bool),
    VolumeChanged(f64),
    StreamEnded(String),
    /// The pipeline reached PAUSED/PLAYING — a deferred resume seek can
    /// be applied.
    PipelineReady,
    /// The audio stream's `title` tag (a live station's `StreamTitle`).
    StreamTitle(String),
    /// The pipeline learnt the media's real length (secs).
    DurationChanged(u64),
    /// Playback failed; retain the originating URI to ignore old selections.
    StreamFailed {
        uri: String,
        message: String,
    },
    StreamPlaying(String),
    RetryRadio(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, glib::Enum, PartialEq, Default)]
#[enum_type(name = "TuneRepeatMode")]
pub enum RepeatMode {
    #[default]
    Consecutive,
    RepeatAll,
    RepeatOne,
    RepeatEach,
}

impl Display for RepeatMode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RepeatMode::Consecutive => write!(f, "consecutive"),
            RepeatMode::RepeatAll => write!(f, "repeat-all"),
            RepeatMode::RepeatOne => write!(f, "repeat-one"),
            RepeatMode::RepeatEach => write!(f, "repeat-each"),
        }
    }
}

#[derive(Clone, Copy, Debug, glib::Enum, PartialEq, Default)]
#[enum_type(name = "TuneReplayGainMode")]
pub enum ReplayGainMode {
    #[enum_value(name = "album")]
    Album,
    #[enum_value(name = "track")]
    Track,
    #[enum_value(name = "off")]
    #[default]
    Off,
}

impl From<i32> for ReplayGainMode {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Album,
            1 => Self::Track,
            // 2, and anything a newer or hand-edited dconf key holds.
            _ => Self::Off,
        }
    }
}

impl From<ReplayGainMode> for i32 {
    fn from(value: ReplayGainMode) -> Self {
        match value {
            ReplayGainMode::Album => 0,
            ReplayGainMode::Track => 1,
            ReplayGainMode::Off => 2,
        }
    }
}

#[derive(Debug)]
pub enum SeekDirection {
    Forward,
    Backwards,
}

pub struct AudioPlayer {
    app_sender: Sender<ApplicationAction>,
    receiver: RefCell<Option<Receiver<PlaybackAction>>>,
    backend: GstBackend,
    controllers: Vec<Box<dyn Controller>>,
    mpris: MprisController,
    queue: Queue,
    state: PlayerState,
    waveform_generator: WaveformGenerator,
    /// A resume seek (secs) to apply once the freshly-loaded pipeline is
    /// ready — issuing it straight after `set_song_uri` is too early and
    /// `gst_play` drops it. Consumed on `PlaybackAction::PipelineReady`.
    pending_seek: std::cell::Cell<Option<u64>>,
    retry_sender: Sender<PlaybackAction>,
    retry_generation: std::cell::Cell<u64>,
    retry_count: std::cell::Cell<u32>,
    play_requested: std::cell::Cell<bool>,
}

impl fmt::Debug for AudioPlayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioPlayer").finish()
    }
}

impl AudioPlayer {
    pub fn new(app_sender: Sender<ApplicationAction>) -> Rc<Self> {
        let (sender, r) = async_channel::unbounded();
        let receiver = RefCell::new(Some(r));

        let mut controllers: Vec<Box<dyn Controller>> = Vec::new();

        // The MPRIS + TrackList server. Held as a field *and* in the
        // controllers list (a cheap `Rc` clone) — the list drives its
        // `set_*` push notifications, the field lets `new` `attach` it to
        // the finished `AudioPlayer` below.
        let mpris = MprisController::new();
        controllers.push(Box::new(mpris.clone()));

        let inhibit_controller = InhibitController::new();
        controllers.push(Box::new(inhibit_controller));

        let waveform_generator = WaveformGenerator::new();
        controllers.push(Box::new(waveform_generator.clone()));

        let backend = GstBackend::new(sender.clone());

        let queue = Queue::default();
        let state = PlayerState::default();

        let res = Rc::new(Self {
            app_sender,
            receiver,
            backend,
            controllers,
            mpris,
            queue,
            state,
            waveform_generator,
            pending_seek: std::cell::Cell::new(None),
            retry_sender: sender,
            retry_generation: std::cell::Cell::new(0),
            retry_count: std::cell::Cell::new(0),
            play_requested: std::cell::Cell::new(false),
        });

        res.clone().setup_channel();
        res.mpris.attach(&res);

        res
    }

    /// A clone of the sender the player uses to talk back to `Application`.
    pub(crate) fn app_sender(&self) -> Sender<ApplicationAction> {
        self.app_sender.clone()
    }

    /// The MPRIS server's zbus connection, once it is up — the
    /// supplementary `org.margo.Tune` interface rides on it (see
    /// [`MprisController::connection`]).
    pub(crate) fn mpris_connection(&self) -> Option<zbus::Connection> {
        self.mpris.connection()
    }

    fn setup_channel(self: Rc<Self>) {
        // Runs once, at construction, while the receiver is still there.
        let receiver = self.receiver.borrow_mut().take().unwrap();

        glib::MainContext::default().spawn_local(clone!(
            #[strong(rename_to = this)]
            self,
            async move {
                use futures::prelude::*;

                let mut receiver = std::pin::pin!(receiver);
                while let Some(action) = receiver.next().await {
                    this.process_action(action);
                }
            }
        ));
    }

    fn process_action(&self, action: PlaybackAction) -> glib::ControlFlow {
        match action {
            PlaybackAction::UpdatePosition(pos, notify) => self.update_position(pos, notify),
            PlaybackAction::VolumeChanged(vol) => self.update_volume(vol),
            PlaybackAction::StreamEnded(uri) => {
                if self.play_requested.get() && self.current_uri_is(&uri) {
                    self.play_next();
                }
            }
            PlaybackAction::PipelineReady => self.apply_pending_seek(),
            PlaybackAction::StreamTitle(title) => self.update_stream_title(&title),
            PlaybackAction::DurationChanged(secs) => self.update_stream_duration(secs),
            PlaybackAction::StreamFailed { uri, message } => {
                if self.play_requested.get() && self.current_uri_is(&uri) {
                    self.report_stream_error(message);
                }
            }
            PlaybackAction::StreamPlaying(uri) => {
                if self.play_requested.get() && self.current_uri_is(&uri) {
                    self.state.set_stream_error("");
                    self.publish_playback_state(PlaybackState::Playing);
                }
            }
            PlaybackAction::RetryRadio(generation) => self.retry_radio(generation),
        }

        glib::ControlFlow::Continue
    }

    fn load_uri(&self, uri: Option<&str>) {
        // A resume target belongs only to the media for which it was queued.
        self.cancel_radio_retry();
        self.retry_count.set(0);
        self.pending_seek.set(None);
        self.backend.set_song_uri(uri);
    }

    fn set_playback_state(&self, state: PlaybackState) {
        self.cancel_radio_retry();
        self.play_requested.set(state == PlaybackState::Playing);
        if state == PlaybackState::Playing {
            self.retry_count.set(0);
        }
        if let Some(current_song) = self.state.current_song() {
            debug!("Current song: {}", current_song.uri());

            self.state.set_playback_state(&state);

            for c in &self.controllers {
                c.set_playback_state(&state);
            }

            match state {
                PlaybackState::Playing => self.backend.play(),
                PlaybackState::Paused => self.backend.pause(),
                PlaybackState::Stopped => self.backend.stop(),
            }
        } else {
            debug!("Getting the next song");
            if let Some(next_song) = self.queue.next_song(false) {
                debug!("Next song: {}", next_song.uri());

                for c in &self.controllers {
                    c.set_song(&next_song);
                }

                next_song.set_playing(true);

                self.load_uri(Some(&next_song.uri()));
                self.state.set_current_song(Some(next_song));
                self.state.set_playback_state(&state);

                for c in &self.controllers {
                    c.set_playback_state(&state);
                }

                match state {
                    PlaybackState::Playing => self.backend.play(),
                    PlaybackState::Paused => self.backend.pause(),
                    PlaybackState::Stopped => self.backend.stop(),
                }
            } else {
                debug!("No songs left");
                self.load_uri(None);
                self.state.set_current_song(None);
                self.state.set_playback_state(&PlaybackState::Stopped);

                for c in &self.controllers {
                    c.set_playback_state(&PlaybackState::Stopped);
                }
            }
        }

        // Keep the app alive with no window while something is loaded and not
        // fully stopped (see `Application::set_background_hold`).
        let active =
            self.state.current_song().is_some() && !matches!(state, PlaybackState::Stopped);
        let _ = self
            .app_sender
            .send_blocking(ApplicationAction::BackgroundHold(active));
    }

    pub fn toggle_play(&self) {
        if self.queue.is_empty() {
            return;
        }

        if self.state.playing() {
            self.set_playback_state(PlaybackState::Paused);
        } else {
            self.set_playback_state(PlaybackState::Playing);
        }
    }

    pub fn play(&self) {
        if !self.state.playing() {
            self.set_playback_state(PlaybackState::Playing);
        }
    }

    pub fn pause(&self) {
        self.cancel_radio_retry();
        self.play_requested.set(false);
        if self.state.playing() {
            self.set_playback_state(PlaybackState::Paused);
        } else {
            // A reconnect can already be loading while advertised Stopped.
            self.backend.stop();
            self.publish_playback_state(PlaybackState::Stopped);
        }
    }

    pub fn stop(&self) {
        self.set_playback_state(PlaybackState::Stopped);
    }

    pub fn skip_previous(&self) {
        if self.queue.is_empty() {
            return;
        }

        if let Some(current_song) = self.state.current_song() {
            // We only skip to the previous song if we are
            // within a seek backward step, otherwise we just
            // restart the song (a live stream has no start to go back to)
            if self.state.position() >= 10 && !current_song.is_live() {
                self.backend.seek_start();
                return;
            }

            if self.queue.is_first_song() {
                return;
            }

            debug!("Marking '{}' as not playing", current_song.uri());
            current_song.set_playing(false);
        }

        if let Some(prev_song) = self.queue.previous_song() {
            debug!("Playing previous: {}", prev_song.uri());

            let was_playing = self.state.playing();
            if was_playing {
                self.set_playback_state(PlaybackState::Paused);
            }

            for c in &self.controllers {
                c.set_song(&prev_song);
            }

            self.load_uri(Some(&prev_song.uri()));

            debug!("Marking '{}' as playing", prev_song.uri());
            prev_song.set_playing(true);

            self.state.set_current_song(Some(prev_song));

            if was_playing {
                self.set_playback_state(PlaybackState::Playing);
            }
        }
    }

    /// Explicit "next" — user pressed the button / keybind / sent it
    /// over MPRIS or the shell IPC. Always advances to a different
    /// track (see [`Queue::next_song`]'s `manual` flag).
    pub fn skip_next(&self) {
        self.advance(true);
    }

    /// A track ended on its own — honour `RepeatOne` (replay it).
    fn play_next(&self) {
        if self.state.current_song().is_some_and(|song| song.is_live()) {
            self.report_stream_error("Live radio stream ended.".into());
        } else {
            self.advance(false);
        }
    }

    fn advance(&self, manual: bool) {
        if self.queue.is_empty() {
            return;
        }

        if let Some(current_song) = self.state.current_song() {
            current_song.set_playing(false);
        }

        if let Some(next_song) = self.queue.next_song(manual) {
            debug!("Playing next (skip-next): {}", next_song.uri());

            let was_playing = self.state.playing();
            if was_playing {
                self.set_playback_state(PlaybackState::Paused);
            }

            for c in &self.controllers {
                c.set_song(&next_song);
            }

            self.load_uri(Some(&next_song.uri()));

            next_song.set_playing(true);

            self.state.set_current_song(Some(next_song));

            if was_playing {
                self.set_playback_state(PlaybackState::Playing);
            }
        } else {
            self.skip_to(0);
            self.set_playback_state(PlaybackState::Stopped);
        }
    }

    pub fn skip_to(&self, pos: u32) {
        if self.queue.is_empty() {
            return;
        }

        if Some(pos) == self.queue.current_song_index() {
            return;
        }

        if let Some(current_song) = self.state.current_song() {
            current_song.set_playing(false);
        }

        if let Some(song) = self.queue.skip_song(pos) {
            debug!("Playing next (skip-to): {}", song.uri());
            let was_playing = self.state.playing();
            if was_playing {
                self.set_playback_state(PlaybackState::Paused);
            }

            for c in &self.controllers {
                c.set_song(&song);
            }

            self.load_uri(Some(&song.uri()));

            song.set_playing(true);

            self.state.set_current_song(Some(song));

            if was_playing {
                self.set_playback_state(PlaybackState::Playing);
            }
        } else {
            self.load_uri(None);
            self.state.set_current_song(None);
            self.set_playback_state(PlaybackState::Stopped);
        }
    }

    /// Whether the current entry can seek at all (a live radio stream
    /// cannot; asking GStreamer to just stalls it).
    fn seekable(&self) -> bool {
        !self.state.current_song().is_some_and(|s| s.is_live())
    }

    fn seek(&self, offset: u64, direction: SeekDirection) {
        if !self.seekable() {
            return;
        }
        self.backend.seek(
            self.state.position(),
            self.state.duration(),
            offset,
            direction,
        );
    }

    pub fn seek_start(&self) {
        if !self.seekable() {
            return;
        }
        let position = self.state.position() + 1;
        self.backend.seek(
            position,
            self.state.duration(),
            position,
            SeekDirection::Backwards,
        );
    }

    pub fn seek_backwards(&self) {
        self.seek(10, SeekDirection::Backwards);
    }

    pub fn seek_forward(&self) {
        self.seek(10, SeekDirection::Forward);
    }

    pub fn seek_offset(&self, offset: i64) {
        let direction = if offset < 0 {
            SeekDirection::Backwards
        } else {
            SeekDirection::Forward
        };
        self.seek(offset.unsigned_abs(), direction);
    }

    pub fn seek_position_rel(&self, position: f64) {
        if !self.seekable() {
            return;
        }
        let duration = self.state.duration() as f64;
        let pos = (duration * position).clamp(0.0, duration);
        self.backend.seek_position(pos as u64);
    }

    pub fn seek_position_abs(&self, position: u64) {
        if !self.seekable() {
            return;
        }
        let duration = self.state.duration();
        // Clamp *below* the duration (seeking exactly to the end just
        // fires EOS); `max` here was a long-standing bug — it sent every
        // seek to the end of the track.
        let pos = if duration > 0 {
            position.min(duration.saturating_sub(1))
        } else {
            position
        };
        self.backend.seek_position(pos);
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    pub fn waveform_generator(&self) -> &WaveformGenerator {
        &self.waveform_generator
    }

    pub fn set_current_song(&self, song: Option<Song>) {
        self.cancel_radio_retry();
        self.state.set_current_song(song);
    }

    fn update_position(&self, position: u64, notify: bool) {
        // Backstop: if the pipeline never sent a StateChanged we could act
        // on, the first real position tick still lets the seek land.
        if self.pending_seek.get().is_some() {
            self.apply_pending_seek();
            return;
        }

        self.state.set_position(position);

        for c in &self.controllers {
            c.set_position(position, notify);
        }
    }

    /// Arm a resume seek to `secs`, applied once the current track's
    /// pipeline reaches PAUSED/PLAYING (see [`PlaybackAction::PipelineReady`]).
    pub fn queue_resume_seek(&self, secs: u64) {
        self.pending_seek.set(self.seekable().then_some(secs));
    }

    fn apply_pending_seek(&self) {
        let Some(target) = self.pending_seek.take() else {
            return;
        };
        if !self.seekable() {
            return;
        }
        let dur = self.state.duration();
        let target = if dur > 0 {
            target.min(dur.saturating_sub(1))
        } else {
            target
        };
        if target > 0 {
            self.backend.seek_position(target);
        }
    }

    fn update_stream_title(&self, raw: &str) {
        if let Some(song) = self.state.current_song()
            && song.is_live()
            && song.set_now_playing(raw)
        {
            self.announce_metadata(&song);
        }
    }

    fn update_stream_duration(&self, secs: u64) {
        if let Some(song) = self.state.current_song()
            && song.set_stream_duration(secs)
        {
            self.announce_metadata(&song);
        }
    }

    /// Re-publish `song`'s metadata after it changed in place (a new
    /// programme title, the real length, fetched artwork): the window, the
    /// `org.margo.Tune` bridge and MPRIS — so the shell's media widget
    /// follows the station.
    pub fn announce_metadata(&self, song: &Song) {
        if self.state.current_song().as_ref() == Some(song) {
            self.state.notify_song_metadata();
        }
        for c in &self.controllers {
            c.song_metadata_changed(song);
        }
    }

    fn current_uri_is(&self, uri: &str) -> bool {
        self.state
            .current_song()
            .is_some_and(|song| song.uri() == uri)
    }

    fn cancel_radio_retry(&self) {
        self.retry_generation
            .set(self.retry_generation.get().wrapping_add(1));
    }

    // Publish an observed state without issuing a new command to GstPlay.
    fn publish_playback_state(&self, state: PlaybackState) {
        self.state.set_playback_state(&state);
        for controller in &self.controllers {
            controller.set_playback_state(&state);
        }
        let _ = self
            .app_sender
            .send_blocking(ApplicationAction::BackgroundHold(
                self.play_requested.get() || state != PlaybackState::Stopped,
            ));
    }

    fn report_stream_error(&self, msg: String) {
        self.pending_seek.set(None);
        self.cancel_radio_retry();
        self.backend.stop();
        let retry = self.play_requested.get()
            && self.state.current_song().is_some_and(|song| song.is_live())
            && self.retry_count.get() < 3;
        if !retry {
            self.play_requested.set(false);
        }
        self.publish_playback_state(PlaybackState::Stopped);
        if self
            .state
            .current_song()
            .is_some_and(|song| song.is_stream())
        {
            self.state.set_stream_error(&msg);
        }
        if retry {
            // Bounded per explicit playback request, not per successful packet:
            // a station that repeatedly drops must not reconnect forever.
            let delay = 1 << self.retry_count.get();
            self.retry_count.set(self.retry_count.get() + 1);
            let generation = self.retry_generation.get();
            let sender = self.retry_sender.clone();
            log::warn!(
                "Radio stream failed; reconnect {}/3 in {delay}s",
                self.retry_count.get()
            );
            glib::timeout_add_once(std::time::Duration::from_secs(delay), move || {
                let _ = sender.send_blocking(PlaybackAction::RetryRadio(generation));
            });
        }
    }

    fn retry_radio(&self, generation: u64) {
        if generation != self.retry_generation.get() || !self.play_requested.get() {
            return;
        }
        if let Some(song) = self.state.current_song().filter(|song| song.is_live()) {
            self.backend.set_song_uri(Some(&song.uri()));
            self.backend.play();
            // Keep Stopped/error visible until GstPlay really reaches Playing.
        }
    }

    pub fn playback_rate(&self) -> f64 {
        self.backend.rate()
    }

    /// Set the (sticky, global) playback rate — clamped to 0.5..=2.0,
    /// propagated to the pipeline, `PlayerState` and every controller
    /// (so MPRIS emits `Rate`).
    pub fn set_playback_rate(&self, rate: f64) {
        self.backend.set_rate(rate);
        let applied = self.backend.rate();
        self.state.set_playback_rate(applied);
        for c in &self.controllers {
            c.set_playback_rate(applied);
        }
    }

    fn update_volume(&self, volume: f64) {
        debug!("Updating volume to: {}", &volume);
        self.state.set_volume(volume);
    }

    pub fn set_volume(&self, volume: f64) {
        self.backend.set_volume(volume);
    }

    /// Apply a remembered level at startup: the pipeline gets it before
    /// the first track loads, and `PlayerState` reflects it immediately so
    /// the window's sliders and the MPRIS/shell snapshot start correct
    /// (GstPlay's own `VolumeChanged` echo arrives asynchronously).
    pub fn restore_volume(&self, volume: f64) {
        let volume = crate::volume_memory::sanitize(volume);
        self.backend.set_volume(volume);
        self.state.set_volume(volume);
    }

    pub fn toggle_repeat_mode(&self) {
        let cur_mode = self.queue.repeat_mode();
        let new_mode = match cur_mode {
            RepeatMode::Consecutive => RepeatMode::RepeatAll,
            RepeatMode::RepeatAll => RepeatMode::RepeatOne,
            RepeatMode::RepeatOne => RepeatMode::RepeatEach,
            RepeatMode::RepeatEach => RepeatMode::Consecutive,
        };
        self.queue.set_repeat_mode(new_mode);

        for c in &self.controllers {
            c.set_repeat_mode(new_mode);
        }
    }

    /// Set an explicit repeat mode (MPRIS `LoopStatus`, `AppCommand`)
    /// and propagate it to every controller + the UI.
    pub fn update_repeat_mode(&self, repeat: RepeatMode) {
        if repeat != self.queue.repeat_mode() {
            self.queue.set_repeat_mode(repeat);

            for c in &self.controllers {
                c.set_repeat_mode(repeat);
            }
        }
    }

    pub fn clear_queue(&self) {
        self.stop();
        self.state.set_current_song(None);
        self.queue.clear();

        let mut cover_cache = crate::audio::cover_cache::lock(CoverCache::global());
        cover_cache.clear();
    }

    pub fn remove_song(&self, song: &Song) {
        if song.playing() {
            self.skip_next();
        }

        self.queue.remove_song(song);

        if self.queue.is_empty() {
            self.state.set_current_song(None);
        }
    }

    pub fn set_replaygain(&self, replaygain: ReplayGainMode) {
        self.backend.set_replaygain(replaygain);
    }

    pub fn replaygain_available(&self) -> bool {
        self.backend.replaygain_available()
    }
}

#[cfg(test)]
#[path = "radio_recovery_tests.rs"]
mod radio_recovery_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::stream::{StreamInfo, StreamKind};

    // No window, session bus, inhibitors or real audio device in these tests.
    pub(super) fn test_player() -> AudioPlayer {
        gst::init().unwrap();
        let (app_sender, _) = async_channel::unbounded();
        let (sender, receiver) = async_channel::unbounded();
        AudioPlayer {
            app_sender,
            receiver: RefCell::new(Some(receiver)),
            backend: GstBackend::new(sender.clone()),
            controllers: Vec::new(),
            mpris: MprisController::new(),
            queue: Queue::default(),
            state: PlayerState::default(),
            waveform_generator: WaveformGenerator::new(),
            pending_seek: std::cell::Cell::new(None),
            retry_sender: sender,
            retry_generation: std::cell::Cell::new(0),
            retry_count: std::cell::Cell::new(0),
            play_requested: std::cell::Cell::new(false),
        }
    }

    fn radio(key: &str) -> Song {
        Song::from_stream(StreamInfo {
            kind: StreamKind::Radio,
            uri: format!("http://127.0.0.1:9/{key}"),
            key: key.to_string(),
            ..StreamInfo::default()
        })
    }

    #[test]
    fn restored_volume_reaches_the_pipeline_and_state_clamped() {
        let _guard = crate::audio::gst_backend::tests::GST_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let player = test_player();
        player.restore_volume(0.5);
        assert_eq!(player.state.volume(), 0.5);
        // The pipeline takes linear volume: cubic 0.5 -> 0.125.
        let linear = player.backend.test_player().volume();
        assert!((linear - 0.125).abs() < 1e-6, "pipeline volume {linear}");

        player.restore_volume(7.0);
        assert_eq!(player.state.volume(), 1.0);
        player.restore_volume(f64::NAN);
        assert_eq!(player.state.volume(), 1.0);
        player.restore_volume(-2.0);
        assert_eq!(player.state.volume(), 0.0);
        assert_eq!(player.backend.test_player().volume(), 0.0);
    }

    #[test]
    fn a_failed_station_stops_playback_and_can_be_retried() {
        let _guard = crate::audio::gst_backend::tests::GST_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let player = test_player();
        let song = radio("failed");
        player.queue.add_song(&song);
        player.state.set_current_song(Some(song.clone()));
        player.state.set_playback_state(&PlaybackState::Playing);
        player.queue_resume_seek(42);
        player.report_stream_error("not-negotiated".into());
        assert!(
            !player.state.playing(),
            "failed station still advertises Playing"
        );
        assert_eq!(player.state.stream_error(), "not-negotiated");
        assert_eq!(player.state.current_song(), Some(song));
        assert_eq!(player.pending_seek.get(), None);
    }

    #[test]
    fn changing_station_discards_the_previous_tracks_resume_seek() {
        let _guard = crate::audio::gst_backend::tests::GST_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let player = test_player();
        for key in ["one", "two", "three"] {
            player.queue.add_song(&radio(key));
        }
        player.skip_to(0);
        for target in [1, 2, 0] {
            player.pending_seek.set(Some(42));
            player.skip_to(target);
            assert_eq!(
                player.pending_seek.get(),
                None,
                "skip_to retained a stale seek"
            );
        }
        player.queue_resume_seek(42);
        player.skip_next();
        assert_eq!(
            player.pending_seek.get(),
            None,
            "next retained a stale seek"
        );
        player.queue_resume_seek(42);
        player.skip_previous();
        assert_eq!(
            player.pending_seek.get(),
            None,
            "previous retained a stale seek"
        );
    }

    #[test]
    fn station_navigation_never_seeks_the_live_pipeline() {
        let _guard = crate::audio::gst_backend::tests::GST_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use gst::prelude::*;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::time::{Duration, Instant};
        let fixture = crate::audio::gst_backend::tests::RadioFixture::new();
        let player = test_player();
        let sink = gst::ElementFactory::make("fakesink")
            .property("sync", true)
            .property("signal-handoffs", true)
            .build()
            .unwrap();
        let buffers = Arc::new(AtomicUsize::new(0));
        let count = buffers.clone();
        sink.connect("handoff", false, move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            None
        });
        let seeks = Arc::new(AtomicUsize::new(0));
        let count = seeks.clone();
        sink.static_pad("sink").unwrap().add_probe(
            gst::PadProbeType::EVENT_UPSTREAM,
            move |_, info| {
                if info
                    .event()
                    .is_some_and(|e| e.type_() == gst::EventType::Seek)
                {
                    count.fetch_add(1, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            },
        );
        player
            .backend
            .test_player()
            .pipeline()
            .set_property("audio-sink", &sink);
        let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = warnings.clone();
        player
            .backend
            .test_player()
            .message_bus()
            .set_sync_handler(move |_, msg| {
                if let Ok(message) = gst_play::PlayMessage::parse(msg) {
                    match message {
                        gst_play::PlayMessage::Warning(w) => {
                            observed.lock().unwrap().push(w.error().to_string())
                        }
                        gst_play::PlayMessage::Error(e) => {
                            observed.lock().unwrap().push(e.error().to_string())
                        }
                        _ => {}
                    }
                }
                gst::BusSyncReply::Drop
            });
        for name in ["paradise", "lowmp3", "mp3"] {
            let song = Song::from_stream(StreamInfo {
                kind: StreamKind::Radio,
                key: name.into(),
                uri: fixture.uri(name),
                ..StreamInfo::default()
            });
            player.queue.add_song(&song);
        }
        // Adding a station selects it before Play, as window/streams does.
        player.skip_to(0);
        player.play();
        // Public-station formats: Paradise AAC, Virgin 22.05kHz MP3,
        // Dance 44.1kHz MP3; repeat the exact failing native E2E order.
        for (navigation, target) in [0, 1, 2, 0, 2, 1].into_iter().enumerate() {
            let before = buffers.load(Ordering::Relaxed);
            if navigation == 1 {
                player.skip_next();
            } else if navigation == 5 {
                player.skip_previous();
            } else {
                player.skip_to(target);
            }
            let deadline = Instant::now() + Duration::from_secs(8);
            while buffers.load(Ordering::Relaxed) < before + 10 {
                while let Ok(action) = player.receiver.borrow().as_ref().unwrap().try_recv() {
                    assert!(
                        !matches!(action, PlaybackAction::StreamFailed { .. }),
                        "{action:?}"
                    );
                    player.process_action(action);
                }
                assert!(
                    Instant::now() < deadline,
                    "station navigation {navigation} stalled"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            // A saved listening position must never turn a live station into
            // a seek, even if a late PipelineReady arrives after selection.
            player.queue_resume_seek(42);
            assert_eq!(
                player.pending_seek.get(),
                None,
                "a live station must not arm a deferred resume seek"
            );
            player.process_action(PlaybackAction::PipelineReady);
            let deadline = Instant::now() + Duration::from_secs(8);
            while buffers.load(Ordering::Relaxed) < before + 20 {
                assert!(
                    Instant::now() < deadline,
                    "radio resume seek stalled playback"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                seeks.load(Ordering::Relaxed),
                0,
                "station navigation {navigation} issued a seek"
            );
            assert!(
                warnings.lock().unwrap().is_empty(),
                "station navigation {navigation}: {:?}",
                warnings.lock().unwrap()
            );
        }
        crate::audio::gst_backend::tests::stop_player(player.backend.test_player());
    }

    #[test]
    fn a_broken_http_station_can_be_replaced_without_restarting_the_player() {
        let _guard = crate::audio::gst_backend::tests::GST_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use gst::prelude::*;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::time::{Duration, Instant};
        let fixture = crate::audio::gst_backend::tests::RadioFixture::new();
        let player = test_player();
        let sink = gst::ElementFactory::make("fakesink")
            .property("sync", true)
            .property("signal-handoffs", true)
            .build()
            .unwrap();
        let buffers = Arc::new(AtomicUsize::new(0));
        let count = buffers.clone();
        sink.connect("handoff", false, move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            None
        });
        player
            .backend
            .test_player()
            .pipeline()
            .set_property("audio-sink", &sink);
        for name in ["broken", "aac"] {
            player.queue.add_song(&Song::from_stream(StreamInfo {
                kind: StreamKind::Radio,
                key: name.into(),
                uri: fixture.uri(name),
                ..StreamInfo::default()
            }));
        }
        player.skip_to(0);
        player.play();
        let deadline = Instant::now() + Duration::from_secs(8);
        while player.state.stream_error().is_empty() {
            while let Ok(action) = player.receiver.borrow().as_ref().unwrap().try_recv() {
                player.process_action(action);
            }
            assert!(Instant::now() < deadline, "HTTP failure was not reported");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!player.state.playing());
        player.skip_to(1);
        player.play();
        let deadline = Instant::now() + Duration::from_secs(8);
        while buffers.load(Ordering::Relaxed) < 10 {
            while let Ok(action) = player.receiver.borrow().as_ref().unwrap().try_recv() {
                assert!(
                    !matches!(action, PlaybackAction::StreamFailed { .. }),
                    "{action:?}"
                );
                player.process_action(action);
            }
            assert!(Instant::now() < deadline, "replacement station stalled");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(player.state.playing());
        crate::audio::gst_backend::tests::stop_player(player.backend.test_player());
    }

    #[test]
    fn an_unknown_replaygain_key_means_off() {
        assert_eq!(ReplayGainMode::from(0), ReplayGainMode::Album);
        assert_eq!(ReplayGainMode::from(1), ReplayGainMode::Track);
        assert_eq!(ReplayGainMode::from(2), ReplayGainMode::Off);
        // An out-of-range value (a hand-edited or newer dconf key).
        assert_eq!(ReplayGainMode::from(7), ReplayGainMode::Off);
        assert_eq!(ReplayGainMode::from(-1), ReplayGainMode::Off);
    }
}
