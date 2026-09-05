// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! MPRIS2 — `org.mpris.MediaPlayer2` + `.Player` + `.TrackList`.
//!
//! Uses `mpris_server::LocalServer` (the `!Send`, glib-main-context variant)
//! so the interface impl can hold the player directly — no channel. The
//! `TrackList` interface exposes mtune's queue to standard MPRIS clients
//! (`playerctl`, KDE Connect, GNOME's media controls): `Tracks`,
//! `GetTracksMetadata`, `GoTo`, `RemoveTrack`, and the `TrackAdded` /
//! `TrackRemoved` / `TrackListReplaced` signals.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{gio, glib, prelude::*};
use log::error;
use mpris_server::{
    LocalServer, LoopStatus, Metadata, PlaybackRate, PlaybackStatus, Property, Time, TrackId,
    TrackListSignal, Volume, zbus::fdo,
};

use crate::application::ApplicationAction;
use crate::audio::{AudioPlayer, Controller, PlaybackState, RepeatMode, Song};
use crate::config::APPLICATION_ID;

const TRACK_PREFIX: &str = "/org/margo/Tune/Track/";

fn track_id(index: usize) -> TrackId {
    TrackId::try_from(format!("{TRACK_PREFIX}{index}")).unwrap_or(TrackId::NO_TRACK)
}

fn track_index(id: &TrackId) -> Option<usize> {
    id.as_str().strip_prefix(TRACK_PREFIX)?.parse().ok()
}

fn status(state: PlaybackState) -> PlaybackStatus {
    match state {
        PlaybackState::Playing => PlaybackStatus::Playing,
        PlaybackState::Paused => PlaybackStatus::Paused,
        PlaybackState::Stopped => PlaybackStatus::Stopped,
    }
}

fn loop_status(repeat: RepeatMode) -> LoopStatus {
    match repeat {
        RepeatMode::Consecutive => LoopStatus::None,
        RepeatMode::RepeatOne => LoopStatus::Track,
        // MPRIS has no fourth LoopStatus value -- RepeatEach reports as
        // Playlist, so an external client at least sees "some loop is
        // on" (it can't specifically request RepeatEach back).
        RepeatMode::RepeatAll | RepeatMode::RepeatEach => LoopStatus::Playlist,
    }
}

/// Build MPRIS `Metadata` for a queue entry.
fn metadata_for(song: &Song, index: usize) -> Metadata {
    let mut m = Metadata::new();
    m.set_trackid(Some(track_id(index)));
    let title = song.title();
    if !title.is_empty() {
        m.set_title(Some(title));
    }
    let artist = song.artist();
    if !artist.is_empty() {
        m.set_artist(Some(vec![artist]));
    }
    let album = song.album();
    if !album.is_empty() {
        m.set_album(Some(album));
    }
    m.set_length(Some(Time::from_secs(song.duration() as i64)));
    if let Some(cache) = song.cover_cache() {
        let file = gio::File::for_path(&cache);
        if file
            .query_info(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .map(|i| i.file_type() == gio::FileType::Regular)
            .unwrap_or(false)
        {
            m.set_art_url(Some(file.uri().to_string()));
        }
    }
    m
}

// ── The interface impl ───────────────────────────────────────────────

struct TuneMpris {
    player: Weak<AudioPlayer>,
    app: async_channel::Sender<ApplicationAction>,
}

impl TuneMpris {
    fn player(&self) -> Option<Rc<AudioPlayer>> {
        self.player.upgrade()
    }
    fn current_metadata(&self) -> Metadata {
        match self.player().and_then(|p| {
            let q = p.queue();
            q.current_song_index()
                .and_then(|i| q.song_at(i).map(|s| (s, i)))
        }) {
            Some((song, i)) => metadata_for(&song, i as usize),
            None => {
                let mut m = Metadata::new();
                m.set_trackid(Some(TrackId::NO_TRACK));
                m
            }
        }
    }
}

impl mpris_server::LocalRootInterface for TuneMpris {
    async fn raise(&self) -> fdo::Result<()> {
        let _ = self.app.send(ApplicationAction::Present).await;
        Ok(())
    }
    async fn quit(&self) -> fdo::Result<()> {
        let _ = self.app.send(ApplicationAction::Quit).await;
        Ok(())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Tune".into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(APPLICATION_ID.trim_end_matches(".Devel").into())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["file".into()])
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![
            "audio/mpeg".into(),
            "audio/flac".into(),
            "audio/ogg".into(),
            "audio/x-vorbis+ogg".into(),
            "audio/x-opus+ogg".into(),
            "audio/mp4".into(),
            "audio/x-wav".into(),
        ])
    }
}

impl mpris_server::LocalPlayerInterface for TuneMpris {
    async fn next(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.skip_next();
        }
        Ok(())
    }
    async fn previous(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.skip_previous();
        }
        Ok(())
    }
    async fn pause(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.pause();
        }
        Ok(())
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.toggle_play();
        }
        Ok(())
    }
    async fn stop(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.stop();
        }
        Ok(())
    }
    async fn play(&self) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.play();
        }
        Ok(())
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        if let Some(p) = self.player() {
            p.seek_offset(offset.as_secs());
        }
        Ok(())
    }
    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        if let Some(p) = self.player()
            && track_index(&track_id) == p.queue().current_song_index().map(|i| i as usize)
        {
            p.seek_position_abs(position.as_secs().max(0) as u64);
        }
        Ok(())
    }
    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("open_uri".into()))
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self
            .player()
            .map(|p| {
                if p.state().current_song().is_none() {
                    PlaybackStatus::Stopped
                } else if p.state().playing() {
                    PlaybackStatus::Playing
                } else {
                    PlaybackStatus::Paused
                }
            })
            .unwrap_or(PlaybackStatus::Stopped))
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(self
            .player()
            .map(|p| loop_status(p.queue().repeat_mode()))
            .unwrap_or(LoopStatus::None))
    }
    async fn set_loop_status(&self, loop_status: LoopStatus) -> mpris_server::zbus::Result<()> {
        if let Some(p) = self.player() {
            let mode = match loop_status {
                LoopStatus::None => RepeatMode::Consecutive,
                LoopStatus::Track => RepeatMode::RepeatOne,
                LoopStatus::Playlist => RepeatMode::RepeatAll,
            };
            p.update_repeat_mode(mode);
        }
        Ok(())
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(self.player().map(|p| p.playback_rate()).unwrap_or(1.0))
    }
    async fn set_rate(&self, rate: PlaybackRate) -> mpris_server::zbus::Result<()> {
        if let Some(p) = self.player() {
            p.set_playback_rate(rate);
        }
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self
            .player()
            .map(|p| p.queue().is_shuffled())
            .unwrap_or(false))
    }
    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        if let Some(p) = self.player() {
            p.queue().set_shuffled(shuffle);
        }
        Ok(())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.current_metadata())
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.player().map(|p| p.state().volume()).unwrap_or(1.0))
    }
    async fn set_volume(&self, volume: Volume) -> mpris_server::zbus::Result<()> {
        if let Some(p) = self.player() {
            p.set_volume(volume.clamp(0.0, 1.0));
        }
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_secs(
            self.player()
                .map(|p| p.state().position() as i64)
                .unwrap_or(0),
        ))
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(crate::audio::MIN_RATE)
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(crate::audio::MAX_RATE)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self
            .player()
            .map(|p| p.queue().n_songs() > 1)
            .unwrap_or(false))
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self
            .player()
            .map(|p| p.queue().n_songs() > 1)
            .unwrap_or(false))
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self
            .player()
            .map(|p| !p.queue().is_empty())
            .unwrap_or(false))
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

impl mpris_server::LocalTrackListInterface for TuneMpris {
    async fn get_tracks_metadata(&self, track_ids: Vec<TrackId>) -> fdo::Result<Vec<Metadata>> {
        let Some(p) = self.player() else {
            return Ok(Vec::new());
        };
        let q = p.queue();
        Ok(track_ids
            .iter()
            .filter_map(|id| {
                let i = track_index(id)?;
                q.song_at(i as u32).map(|s| metadata_for(&s, i))
            })
            .collect())
    }
    async fn add_track(
        &self,
        _uri: String,
        _after_track: TrackId,
        _set_as_current: bool,
    ) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "add tracks from the Tune window or by pointing it at a folder".into(),
        ))
    }
    async fn remove_track(&self, track_id: TrackId) -> fdo::Result<()> {
        if let Some(p) = self.player()
            && let Some(i) = track_index(&track_id)
            && let Some(song) = p.queue().song_at(i as u32)
        {
            p.remove_song(&song);
        }
        Ok(())
    }
    async fn go_to(&self, track_id: TrackId) -> fdo::Result<()> {
        if let Some(p) = self.player()
            && let Some(i) = track_index(&track_id)
        {
            p.skip_to(i as u32);
        }
        Ok(())
    }
    async fn tracks(&self) -> fdo::Result<Vec<TrackId>> {
        Ok(self
            .player()
            .map(|p| {
                (0..p.queue().n_songs())
                    .map(|i| track_id(i as usize))
                    .collect()
            })
            .unwrap_or_default())
    }
    async fn can_edit_tracks(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

// ── The controller wrapper ───────────────────────────────────────────

struct MprisInner {
    server: RefCell<Option<Rc<LocalServer<TuneMpris>>>>,
}

#[derive(Clone)]
pub struct MprisController(Rc<MprisInner>);

impl std::fmt::Debug for MprisController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MprisController").finish_non_exhaustive()
    }
}

impl MprisController {
    pub fn new() -> Self {
        Self(Rc::new(MprisInner {
            server: RefCell::new(None),
        }))
    }

    /// Bring the MPRIS + TrackList server up, bound to `player`.
    pub fn attach(&self, player: &Rc<AudioPlayer>) {
        let imp = TuneMpris {
            player: Rc::downgrade(player),
            app: player.app_sender(),
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            match LocalServer::new_with_track_list(APPLICATION_ID, imp).await {
                Ok(server) => {
                    let server = Rc::new(server);
                    this.0.server.replace(Some(server.clone()));
                    server.run().await;
                }
                Err(e) => error!("mtune: MPRIS server: {e}"),
            }
        });
    }

    /// The MPRIS server's D-Bus connection, once it is up. mtune's
    /// supplementary `org.margo.Tune` interface is served on this same
    /// connection: the GApplication owns the bare `org.margo.Tune` bus
    /// name outright, so a second zbus connection could never claim it.
    pub fn connection(&self) -> Option<zbus::Connection> {
        self.0
            .server
            .borrow()
            .as_ref()
            .map(|s| s.connection().clone())
    }

    fn with_server(&self, f: impl FnOnce(Rc<LocalServer<TuneMpris>>) + 'static) {
        if let Some(server) = self.0.server.borrow().clone() {
            f(server);
        }
    }

    fn emit_props(&self, props: Vec<Property>) {
        self.with_server(move |server| {
            glib::spawn_future_local(async move {
                let _ = server.properties_changed(props).await;
            });
        });
    }
}

impl Default for MprisController {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for MprisController {
    fn set_playback_state(&self, state: &PlaybackState) {
        self.emit_props(vec![
            Property::PlaybackStatus(status(*state)),
            Property::CanPlay(true),
        ]);
    }

    fn set_song(&self, song: &Song) {
        // Metadata of the now-playing track + a full tracklist re-sync
        // (the queue index -> track-id mapping shifts on add / remove).
        let uri = song.uri();
        self.with_server(move |server| {
            let imp = server.imp();
            let Some(player) = imp.player.upgrade() else {
                return;
            };
            let q = player.queue();
            let index = q
                .position_of_uri(&uri)
                .or_else(|| q.current_song_index())
                .unwrap_or(0) as usize;
            let meta = q
                .song_at(index as u32)
                .map(|s| metadata_for(&s, index))
                .unwrap_or_else(Metadata::new);
            let tracks: Vec<TrackId> = (0..q.n_songs()).map(|i| track_id(i as usize)).collect();
            let current = track_id(index);
            glib::spawn_future_local(async move {
                let _ = server
                    .properties_changed(vec![Property::Metadata(meta)])
                    .await;
                let _ = server
                    .track_list_emit(TrackListSignal::TrackListReplaced {
                        tracks,
                        current_track: current,
                    })
                    .await;
            });
        });
    }

    fn set_position(&self, position: u64, notify: bool) {
        if notify {
            self.with_server(move |server| {
                glib::spawn_future_local(async move {
                    let _ = server
                        .emit(mpris_server::Signal::Seeked {
                            position: Time::from_secs(position as i64),
                        })
                        .await;
                });
            });
        }
    }

    fn set_repeat_mode(&self, repeat: RepeatMode) {
        self.emit_props(vec![Property::LoopStatus(loop_status(repeat))]);
    }

    fn set_playback_rate(&self, rate: f64) {
        self.emit_props(vec![Property::Rate(rate)]);
    }
}
