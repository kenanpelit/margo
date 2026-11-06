// SPDX-License-Identifier: GPL-3.0-or-later

//! Follows playback for network songs, whether or not the browser is open:
//! saves podcast episodes' resume point / played state and fetches the
//! artwork of the stream that starts playing (so the window and MPRIS show
//! the station logo or podcast cover).

use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use super::{
    art,
    shared::{self, Change},
};
use crate::audio::{AudioPlayer, CoverCache, Song, StreamKind};

/// How often a resume point is written while an episode plays.
const SAVE_EVERY: Duration = Duration::from_secs(20);

struct Tracker {
    player: Weak<AudioPlayer>,
    last_save: Cell<Instant>,
    dirty: Cell<bool>,
}

impl Tracker {
    fn flush(&self) {
        if self.dirty.replace(false) {
            self.last_save.set(Instant::now());
            shared::podcasts_changed(Change::Progress);
        }
    }

    fn on_position(&self, song: &Song, position: u64, duration: u64) {
        if song.kind() != StreamKind::Episode {
            return;
        }
        let Some(key) = song.stream_key() else {
            return;
        };
        let lib = shared::podcasts();
        let (changed, flipped) = {
            let mut l = lib.data.borrow_mut();
            let before = l.progress(&key).played;
            let changed = l.record_position(&key, position, duration);
            (changed, l.progress(&key).played != before)
        };
        if changed {
            self.dirty.set(true);
            if flipped || self.last_save.get().elapsed() >= SAVE_EVERY {
                self.flush();
            }
        }
    }

    fn fetch_cover(&self, song: &Song) {
        if !song.is_stream() || song.cover_texture().is_some() {
            return;
        }
        let Some(url) = song.art_url() else {
            return;
        };
        let song = song.clone();
        let player = self.player.clone();
        glib::spawn_future_local(async move {
            let Some(pixbuf) = art::pixbuf(&url, crate::utils::COVER_SIZE).await else {
                return;
            };
            let uuid = art::url_hash(&url);
            let cover = crate::audio::cover_cache::lock(CoverCache::global())
                .cover_from_pixbuf(&uuid, &pixbuf);
            if let Some(cover) = cover {
                song.set_stream_cover(cover, uuid);
                if let Some(p) = player.upgrade() {
                    p.announce_metadata(&song);
                }
            }
        });
    }
}

/// Start following `player` (once, at application startup).
pub fn attach(player: &Rc<AudioPlayer>) {
    let t = Rc::new(Tracker {
        player: Rc::downgrade(player),
        last_save: Cell::new(Instant::now()),
        dirty: Cell::new(false),
    });
    let state = player.state();

    let tr = t.clone();
    state.connect_notify_local(Some("position"), move |st, _| {
        if let Some(song) = st.current_song() {
            tr.on_position(&song, st.position(), st.duration());
        }
    });

    let tr = t.clone();
    state.connect_notify_local(Some("song"), move |st, _| {
        tr.flush();
        if let Some(song) = st.current_song() {
            tr.fetch_cover(&song);
        }
    });

    state.connect_notify_local(Some("playing"), move |st, _| {
        if !st.playing() {
            t.flush();
        }
    });
}
