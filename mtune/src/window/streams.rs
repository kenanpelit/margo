// SPDX-FileCopyrightText: 2026  margo contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Queueing radio stations and podcast episodes (from the Radio &
//! Podcasts browser) and the window's live-stream feedback.

use super::*;

impl Window {
    /// Replace the queue with a station list and play `start`, so
    /// next/previous (window, MPRIS, the shell's Tune pill) step through
    /// the list. The music queue stays restorable: stations are never
    /// written to the saved `current.pls`.
    pub fn play_station_list(&self, stations: &[Song], start: u32) {
        let Some(player) = self.player() else { return };
        if stations.is_empty() {
            return;
        }
        self.imp().active_playlist.replace(None);
        let queue = player.queue();
        if !queue.is_empty() {
            player.clear_queue();
        }
        queue.add_songs(stations);
        self.switch_mode(WindowMode::MainView);
        player.skip_to(start.min(queue.n_songs().saturating_sub(1)));
        player.play();
    }

    /// Add a podcast episode to the queue (once), and with `play_now` jump
    /// to it, resuming at `resume_at` secs. A queue of radio stations is
    /// replaced: episodes and live stations do not mix.
    pub fn queue_episode(&self, episode: &Song, play_now: bool, resume_at: u64) {
        let Some(player) = self.player() else { return };
        let queue = player.queue();
        let has_stations =
            (0..queue.n_songs()).any(|i| queue.song_at(i).is_some_and(|s| s.is_live()));
        if has_stations {
            player.clear_queue();
        }
        let pos = (0..queue.n_songs())
            .find(|&i| queue.song_at(i).is_some_and(|s| s.equals(episode)))
            .unwrap_or_else(|| {
                queue.add_song(episode);
                queue.n_songs().saturating_sub(1)
            });
        self.switch_mode(WindowMode::MainView);
        if play_now {
            let already = queue.current_song_index() == Some(pos);
            player.skip_to(pos);
            if !already && resume_at > 0 {
                player.queue_resume_seek(resume_at);
            }
            player.play();
        } else {
            self.add_skip_to_toast(i18n("Added to the queue"), i18n("Play"), pos);
        }
        self.update_playlist_time();
    }

    /// Toast stream failures, and keep the window title on the current
    /// radio programme.
    pub(super) fn bind_streams(&self) {
        let Some(player) = self.player() else { return };
        let state = player.state();
        let on_error = state.connect_notify_local(
            Some("stream-error"),
            clone!(
                #[weak(rename_to = win)]
                self,
                move |st, _| {
                    let name = st
                        .current_song()
                        .map(|s| s.station_name().unwrap_or_else(|| s.title()));
                    win.add_toast(i18n_k(
                        // Translators: `{name}` is a radio station or podcast
                        // episode, `{error}` GStreamer's reason; leave both
                        // untranslated.
                        "Cannot play “{name}”: {error}",
                        &[
                            ("name", &name.unwrap_or_default()),
                            ("error", &st.stream_error()),
                        ],
                    ));
                }
            ),
        );
        let on_title = state.connect_notify_local(
            Some("title"),
            clone!(
                #[weak(rename_to = win)]
                self,
                move |st, _| win.update_title(st.current_song().as_ref())
            ),
        );
        self.imp().stream_handlers.replace(vec![on_error, on_title]);
    }

    pub(super) fn unbind_streams(&self) {
        if let Some(player) = self.player() {
            for id in self.imp().stream_handlers.take() {
                player.state().disconnect(id);
            }
        }
    }
}
