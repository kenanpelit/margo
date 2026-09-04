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
    PlayNext,
    /// The pipeline reached PAUSED/PLAYING — a deferred resume seek can
    /// be applied.
    PipelineReady,
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
}

impl Display for RepeatMode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RepeatMode::Consecutive => write!(f, "consecutive"),
            RepeatMode::RepeatAll => write!(f, "repeat-all"),
            RepeatMode::RepeatOne => write!(f, "repeat-one"),
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
            2 => Self::Off,
            _ => panic!("invalid ReplayGainMode enum key"),
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

        let backend = GstBackend::new(sender);

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
            PlaybackAction::PlayNext => self.play_next(),
            PlaybackAction::PipelineReady => self.apply_pending_seek(),
        }

        glib::ControlFlow::Continue
    }

    fn set_playback_state(&self, state: PlaybackState) {
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

                self.backend.set_song_uri(Some(&next_song.uri()));
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
                self.backend.set_song_uri(None);
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
        if self.state.playing() {
            self.set_playback_state(PlaybackState::Paused);
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
            // restart the song
            if self.state.position() >= 10 {
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

            self.backend.set_song_uri(Some(&prev_song.uri()));
            self.backend.seek_start();

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
        self.advance(false);
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

            self.backend.set_song_uri(Some(&next_song.uri()));
            self.backend.seek_start();

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

            self.backend.set_song_uri(Some(&song.uri()));
            self.backend.seek_start();

            song.set_playing(true);

            self.state.set_current_song(Some(song));

            if was_playing {
                self.set_playback_state(PlaybackState::Playing);
            }
        } else {
            self.backend.set_song_uri(None);
            self.state.set_current_song(None);
            self.set_playback_state(PlaybackState::Stopped);
        }
    }

    fn seek(&self, offset: u64, direction: SeekDirection) {
        self.backend.seek(
            self.state.position(),
            self.state.duration(),
            offset,
            direction,
        );
    }

    pub fn seek_start(&self) {
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
        let duration = self.state.duration() as f64;
        let pos = (duration * position).clamp(0.0, duration);
        self.backend.seek_position(pos as u64);
    }

    pub fn seek_position_abs(&self, position: u64) {
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
        self.pending_seek.set(Some(secs));
    }

    fn apply_pending_seek(&self) {
        let Some(target) = self.pending_seek.take() else {
            return;
        };
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

    pub fn toggle_repeat_mode(&self) {
        let cur_mode = self.queue.repeat_mode();
        let new_mode = match cur_mode {
            RepeatMode::Consecutive => RepeatMode::RepeatAll,
            RepeatMode::RepeatAll => RepeatMode::RepeatOne,
            RepeatMode::RepeatOne => RepeatMode::Consecutive,
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

        let mut cover_cache = CoverCache::global().lock().unwrap();
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
