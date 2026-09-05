// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::Cell;

use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::audio::{CoverCache, RepeatMode, ShuffleListModel, Song};

/// Which queue index a "next" lands on, or `None` to stop playback.
/// Pure so the repeat-mode matrix is unit-testable. `manual` (the user
/// pressed skip) collapses `RepeatOne` to `RepeatAll`: repeat-one only
/// loops a track when it *ends*, never under the skip button.
fn next_index(current: u32, n_songs: u32, repeat_mode: RepeatMode, manual: bool) -> Option<u32> {
    let effective = if manual && repeat_mode == RepeatMode::RepeatOne {
        RepeatMode::RepeatAll
    } else {
        repeat_mode
    };
    match effective {
        RepeatMode::RepeatOne => Some(current),
        RepeatMode::Consecutive if current + 1 < n_songs => Some(current + 1),
        RepeatMode::Consecutive => None,
        // `next_song()` branches to `next_index_repeat_each` before ever
        // calling this function for `RepeatEach` -- this arm only exists
        // for match-exhaustiveness. Falls back to RepeatAll's semantics
        // (advance/wrap) if it were ever reached some other way.
        RepeatMode::RepeatAll | RepeatMode::RepeatEach if current + 1 < n_songs => {
            Some(current + 1)
        }
        RepeatMode::RepeatAll | RepeatMode::RepeatEach => Some(0),
    }
}

/// `next_index` counterpart for `RepeatMode::RepeatEach`: repeats the
/// current track `repeat_count` times before advancing (wrapping at the
/// end of the queue), tracking replays via `plays_so_far`. Manual skip
/// always advances immediately and resets the counter -- same
/// "an explicit skip always moves to a different track" convention
/// `next_index` already applies to `RepeatOne`. Pure so it's directly
/// unit-testable; the caller stores the returned counter back into
/// `Queue`'s `repeat_plays` cell.
fn next_index_repeat_each(
    current: u32,
    n_songs: u32,
    repeat_count: u32,
    plays_so_far: u32,
    manual: bool,
) -> (Option<u32>, u32) {
    if manual || plays_so_far + 1 >= repeat_count {
        let next = if current + 1 < n_songs {
            current + 1
        } else {
            0
        };
        (Some(next), 0)
    } else {
        (Some(current), plays_so_far + 1)
    }
}

mod imp {
    use glib::{ParamSpec, ParamSpecBoolean, ParamSpecEnum, ParamSpecObject, ParamSpecUInt, Value};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug)]
    pub struct Queue {
        pub model: ShuffleListModel,
        pub store: gio::ListStore,
        pub repeat_mode: Cell<RepeatMode>,
        /// Configurable N for `RepeatMode::RepeatEach` (Settings/CLI via
        /// `mshellctl mtune repeat-count`; persisted in `mtune.toml`).
        pub repeat_count: Cell<u32>,
        /// How many times the *current* track has played consecutively
        /// under `RepeatEach`. Internal only -- not a GObject property.
        pub repeat_plays: Cell<u32>,
        pub current_pos: Cell<Option<u32>>,
        pub shuffled: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Queue {
        const NAME: &'static str = "TuneQueue";
        type Type = super::Queue;

        fn new() -> Self {
            let store = gio::ListStore::new::<Song>();
            let model = ShuffleListModel::new(Some(&store));

            Self {
                store,
                model,
                repeat_mode: Cell::new(RepeatMode::default()),
                repeat_count: Cell::new(3),
                repeat_plays: Cell::new(0),
                current_pos: Cell::new(None),
                shuffled: Cell::new(false),
            }
        }
    }

    impl ObjectImpl for Queue {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![
                    ParamSpecObject::builder::<Song>("current")
                        .read_only()
                        .build(),
                    ParamSpecEnum::builder::<RepeatMode>("repeat-mode")
                        .read_only()
                        .build(),
                    ParamSpecUInt::builder("repeat-count").read_only().build(),
                    ParamSpecUInt::builder("n-songs").read_only().build(),
                    ParamSpecBoolean::builder("shuffled").read_only().build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "current" => self.obj().current_song().to_value(),
                "repeat-mode" => self.repeat_mode.get().to_value(),
                "repeat-count" => self.repeat_count.get().to_value(),
                "n-songs" => self.store.n_items().to_value(),
                "shuffled" => self.shuffled.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct Queue(ObjectSubclass<imp::Queue>);
}

impl Default for Queue {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Queue {
    pub fn n_songs(&self) -> u32 {
        self.imp().model.n_items()
    }

    pub fn is_empty(&self) -> bool {
        self.imp().model.n_items() == 0
    }

    pub fn model(&self) -> &gio::ListModel {
        self.imp().model.as_ref()
    }

    pub fn song_at(&self, pos: u32) -> Option<Song> {
        if let Some(song) = self.imp().model.item(pos) {
            return Some(song.downcast::<Song>().unwrap());
        }

        None
    }

    pub fn current_song(&self) -> Option<Song> {
        if let Some(pos) = self.imp().current_pos.get() {
            return self.song_at(pos);
        }

        None
    }

    /// Queue position of the first song whose URI matches, if any.
    pub fn position_of_uri(&self, uri: &str) -> Option<u32> {
        (0..self.n_songs()).find(|&i| self.song_at(i).map(|s| s.uri()) == Some(uri.to_string()))
    }

    pub fn set_current_song(&self, song: Option<Song>) {
        if let Some(song) = song {
            for i in 0..self.n_songs() {
                let s = self.song_at(i).unwrap();
                if song.equals(&s) {
                    self.imp().current_pos.replace(Some(i));
                    self.imp().repeat_plays.set(0);
                    self.notify("current");
                    return;
                }
            }
        } else {
            self.imp().current_pos.replace(None);
            self.imp().repeat_plays.set(0);
            self.notify("current");
        }
    }

    pub fn current_song_index(&self) -> Option<u32> {
        self.imp().current_pos.get()
    }

    pub fn add_song(&self, song: &Song) -> bool {
        if !song.equals(&Song::default()) {
            // Add song to the backing store
            self.imp().store.append(song);
            self.notify("n-songs");
            true
        } else {
            false
        }
    }

    pub fn add_songs(&self, songs: &[impl IsA<glib::Object>]) {
        self.imp()
            .store
            .splice(self.imp().model.n_items(), 0, songs);
        self.notify("n-songs");
    }

    pub fn remove_song(&self, song: &Song) {
        let was_shuffled = self.imp().model.shuffled();
        let n_songs = self.n_songs();
        for pos in 0..n_songs {
            let s = self
                .imp()
                .store
                .item(pos)
                .unwrap()
                .downcast::<Song>()
                .unwrap();
            if s.equals(song) {
                self.imp().store.remove(pos);
                break;
            }
        }

        if n_songs != self.n_songs() {
            if was_shuffled {
                self.imp().model.reshuffle(0);
            }
            self.notify("n-songs");
        }

        if self.is_empty() {
            self.imp().current_pos.replace(None);
            self.imp().repeat_plays.set(0);
        }
    }

    pub fn clear(&self) {
        let mut cover_cache = CoverCache::global().lock().unwrap();
        cover_cache.clear();

        self.imp().current_pos.replace(None);
        self.imp().repeat_plays.set(0);
        self.imp().store.remove_all();
        self.notify("n-songs");
    }

    pub fn skip_song(&self, pos: u32) -> Option<Song> {
        self.imp().current_pos.replace(Some(pos));
        self.imp().repeat_plays.set(0);
        self.notify("current");
        self.song_at(pos)
    }

    pub fn previous_song(&self) -> Option<Song> {
        if let Some(current_pos) = self.imp().current_pos.get()
            && current_pos > 0
        {
            let prev = current_pos - 1;
            self.imp().current_pos.replace(Some(prev));
            self.imp().repeat_plays.set(0);
            self.notify("current");
            return self.song_at(current_pos - 1);
        }

        None
    }

    /// The next song to play. `manual` = the user pressed "next" (vs. a
    /// track ending on its own): an explicit skip always moves to a
    /// *different* track — `RepeatOne` only loops the current track when
    /// it finishes by itself, never under the skip button.
    pub fn next_song(&self, manual: bool) -> Option<Song> {
        let n_songs = self.imp().model.n_items();
        if n_songs == 0 {
            return None;
        }

        let Some(current) = self.current_song_index() else {
            // Nothing playing yet — start at the top.
            self.imp().current_pos.replace(Some(0));
            self.notify("current");
            return self.song_at(0);
        };

        let repeat_mode = self.imp().repeat_mode.get();
        let next = if repeat_mode == RepeatMode::RepeatEach {
            let (next, plays) = next_index_repeat_each(
                current,
                n_songs,
                self.imp().repeat_count.get(),
                self.imp().repeat_plays.get(),
                manual,
            );
            self.imp().repeat_plays.set(plays);
            next
        } else {
            next_index(current, n_songs, repeat_mode, manual)
        };
        self.imp().current_pos.replace(next);
        self.notify("current");
        next.and_then(|n| self.song_at(n))
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.imp().repeat_mode.get()
    }

    pub fn set_repeat_mode(&self, repeat_mode: RepeatMode) {
        let old_mode = self.imp().repeat_mode.replace(repeat_mode);
        if old_mode != repeat_mode {
            self.notify("repeat-mode");
        }
    }

    pub fn repeat_count(&self) -> u32 {
        self.imp().repeat_count.get()
    }

    pub fn set_repeat_count(&self, count: u32) {
        let old = self.imp().repeat_count.replace(count);
        if old != count {
            self.notify("repeat-count");
        }
    }

    pub fn is_first_song(&self) -> bool {
        if let Some(current_pos) = self.imp().current_pos.get() {
            return current_pos == 0;
        }

        false
    }

    pub fn is_last_song(&self) -> bool {
        let n_items = self.imp().model.n_items();

        if let Some(current_pos) = self.imp().current_pos.get()
            && n_items > 0
        {
            return current_pos == n_items - 1;
        }

        false
    }

    pub fn is_shuffled(&self) -> bool {
        self.imp().shuffled.get()
    }

    pub fn set_shuffled(&self, shuffled: bool) {
        if shuffled != self.imp().shuffled.replace(shuffled) {
            if shuffled {
                let current_pos = self.imp().current_pos.get().unwrap_or(0);
                self.imp().model.reshuffle(current_pos);
            } else {
                let current_pos = self.current_song_index().unwrap_or(0);
                let current_song = self.song_at(current_pos);
                self.imp().model.unshuffle();
                self.set_current_song(current_song);
            }
            self.notify("shuffled");
        }
    }

    pub fn select_song_at(&self, index: u32) {
        if let Some(song) = self.imp().model.item(index) {
            let song = song.downcast_ref::<Song>().unwrap();
            let is_selected = !song.selected();
            song.set_selected(is_selected);
        }
    }

    pub fn unselect_all_songs(&self) {
        for i in 0..self.imp().store.n_items() {
            let song = self.imp().store.item(i).unwrap();
            song.downcast_ref::<Song>().unwrap().set_selected(false);
        }
    }

    pub fn n_selected_songs(&self) -> u32 {
        let mut count = 0;
        for i in 0..self.imp().store.n_items() {
            let song = self.imp().store.item(i).unwrap();
            if song.downcast_ref::<Song>().unwrap().selected() {
                count += 1;
            }
        }

        count
    }

    pub fn contains(&self, s: &Song) -> bool {
        for i in 0..self.imp().store.n_items() {
            let song = self.imp().store.item(i).unwrap();
            if song.downcast_ref::<Song>().unwrap().equals(s) {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_next_skips_past_repeat_one() {
        // Regression: with LoopStatus=Track (RepeatOne) the "next"
        // button / MPRIS Next / shell IPC did nothing — next_index
        // returned the *current* track. An explicit skip must always
        // move on; repeat-one only loops on a natural track end.
        // 3-song queue, currently on song 0.
        assert_eq!(next_index(0, 3, RepeatMode::RepeatOne, false), Some(0)); // auto: replay
        assert_eq!(next_index(0, 3, RepeatMode::RepeatOne, true), Some(1)); // manual: advance
        assert_eq!(next_index(2, 3, RepeatMode::RepeatOne, true), Some(0)); // manual at end: wrap
    }

    #[test]
    fn consecutive_stops_at_the_end_regardless_of_manual() {
        assert_eq!(next_index(1, 3, RepeatMode::Consecutive, false), Some(2));
        assert_eq!(next_index(1, 3, RepeatMode::Consecutive, true), Some(2));
        assert_eq!(next_index(2, 3, RepeatMode::Consecutive, false), None);
        assert_eq!(next_index(2, 3, RepeatMode::Consecutive, true), None);
    }

    #[test]
    fn repeat_all_wraps_regardless_of_manual() {
        assert_eq!(next_index(0, 3, RepeatMode::RepeatAll, false), Some(1));
        assert_eq!(next_index(2, 3, RepeatMode::RepeatAll, false), Some(0));
        assert_eq!(next_index(2, 3, RepeatMode::RepeatAll, true), Some(0));
    }

    #[test]
    fn single_song_queue() {
        assert_eq!(next_index(0, 1, RepeatMode::Consecutive, true), None);
        assert_eq!(next_index(0, 1, RepeatMode::RepeatAll, true), Some(0));
        assert_eq!(next_index(0, 1, RepeatMode::RepeatOne, true), Some(0));
    }

    #[test]
    fn repeat_each_replays_then_advances() {
        // 3-song queue, count=3, currently on song 0, auto (track ended).
        assert_eq!(next_index_repeat_each(0, 3, 3, 0, false), (Some(0), 1)); // 1st replay
        assert_eq!(next_index_repeat_each(0, 3, 3, 1, false), (Some(0), 2)); // 2nd replay
        assert_eq!(next_index_repeat_each(0, 3, 3, 2, false), (Some(1), 0)); // 3rd play done, advance
    }

    #[test]
    fn repeat_each_wraps_at_the_end() {
        assert_eq!(next_index_repeat_each(2, 3, 3, 2, false), (Some(0), 0));
    }

    #[test]
    fn repeat_each_manual_skip_advances_immediately() {
        // Manual skip always moves on and resets the counter, regardless
        // of how many replays have happened so far -- same convention as
        // RepeatOne's manual-skip-breaks-the-loop rule.
        assert_eq!(next_index_repeat_each(0, 3, 3, 1, true), (Some(1), 0));
        assert_eq!(next_index_repeat_each(2, 3, 3, 0, true), (Some(0), 0));
    }
}
