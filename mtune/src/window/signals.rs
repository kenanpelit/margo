// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Binding the window to the player state and wiring its widget signals.

use super::*;

impl Window {
    // Bind the PlayerState to the UI
    pub(super) fn bind_state(&self) {
        let imp = self.imp();
        if let Some(player) = self.player() {
            let state = player.state();

            // Use the PlayerState:playing property to control the play/pause button
            self.update_play_button();
            let notify_playing_id = state.connect_notify_local(
                Some("playing"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |_, _| {
                        win.set_playlist_selection(false);
                        win.update_play_button();
                    }
                ),
            );
            imp.notify_playing_id.replace(Some(notify_playing_id));

            // Update the position labels
            self.update_position_labels();
            let notify_position_id = state.connect_notify_local(
                Some("position"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |_, _| {
                        win.update_position_labels();
                    }
                ),
            );
            imp.notify_position_id.replace(Some(notify_position_id));

            // Update the UI
            self.update_song();
            let notify_song_id = state.connect_notify_local(
                Some("song"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |_, _| {
                        win.update_song();
                    }
                ),
            );
            imp.notify_song_id.replace(Some(notify_song_id));

            // Update the cover, if any is available
            self.update_cover();
            let notify_cover_id = state.connect_notify_local(
                Some("cover"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |_, _| {
                        win.update_cover();
                    }
                ),
            );
            imp.notify_cover_id.replace(Some(notify_cover_id));

            // Bind the song properties to the UI
            state
                .bind_property("title", &imp.song_details.get().title_label(), "label")
                .sync_create()
                .build();
            state
                .bind_property("artist", &imp.song_details.get().artist_label(), "label")
                .sync_create()
                .build();
            state
                .bind_property("album", &imp.song_details.get().album_label(), "label")
                .sync_create()
                .build();
            state
                .bind_property(
                    "volume",
                    &imp.playback_control.get().volume_control(),
                    "volume",
                )
                .sync_create()
                .build();
        }
    }

    pub(super) fn unbind_state(&self) {
        if let Some(player) = self.player() {
            let state = player.state();

            if let Some(id) = self.imp().notify_playing_id.take() {
                state.disconnect(id);
            }
            if let Some(id) = self.imp().notify_position_id.take() {
                state.disconnect(id);
            }
            if let Some(id) = self.imp().notify_song_id.take() {
                state.disconnect(id);
            }
            if let Some(id) = self.imp().notify_cover_id.take() {
                state.disconnect(id);
            }
        }
    }

    pub(super) fn connect_signals(&self) {
        self.imp().split_view.connect_notify_local(
            Some("collapsed"),
            clone!(
                #[weak(rename_to = win)]
                self,
                move |split_view, _| {
                    win.set_playlist_visible(split_view.shows_sidebar());
                    win.imp()
                        .playlist_view
                        .back_button()
                        .set_visible(split_view.is_collapsed());
                }
            ),
        );

        self.imp().split_view.connect_notify_local(
            Some("show-sidebar"),
            clone!(
                #[weak(rename_to = win)]
                self,
                move |split_view, _| {
                    win.set_playlist_visible(split_view.shows_sidebar());
                }
            ),
        );

        self.imp().waveform_view.connect_closure(
            "position-changed",
            false,
            closure_local!(
                #[watch(rename_to = win)]
                self,
                move |_wv: WaveformView, position: f64| {
                    debug!("New position: {}", position);
                    if let Some(player) = win.player() {
                        let state = player.state();
                        if state.current_song().is_some() {
                            player.seek_position_rel(position);
                            player.play();
                        }
                    }
                }
            ),
        );

        self.imp()
            .playback_control
            .volume_control()
            .connect_closure(
                "volume-changed",
                false,
                closure_local!(
                    #[watch(rename_to = win)]
                    self,
                    move |_vc: VolumeControl, volume: f64| {
                        debug!("Volume changed: {}", volume);
                        if let Some(p) = win.player() {
                            p.set_volume(volume);
                        }
                    }
                ),
            );

        self.imp()
            .playlist_view
            .queue_select_all_button()
            .connect_clicked(clone!(
                #[weak(rename_to = win)]
                self,
                move |_| {
                    if win.playlist_search() {
                        if let Some(ref model) = *win.imp().playlist_filtermodel.borrow() {
                            for idx in 0..model.n_items() {
                                let item = model.item(idx).unwrap();
                                let song = item.downcast_ref::<Song>().unwrap();
                                song.set_selected(true);
                            }
                        }
                    } else if let Some(player) = win.player() {
                        let queue = player.queue();
                        for idx in 0..queue.n_songs() {
                            let song = queue.song_at(idx).unwrap();
                            song.set_selected(true);
                        }
                    }
                }
            ));

        self.imp()
            .playlist_view
            .queue_remove_button()
            .connect_clicked(clone!(
                #[weak(rename_to = win)]
                self,
                move |_| {
                    if let Some(player) = win.player() {
                        let queue = player.queue();
                        let mut remove_songs: Vec<Song> = Vec::new();
                        // Collect all songs to be removed first, since we can't
                        // remove objects from the model while we're iterating it
                        for idx in 0..queue.n_songs() {
                            let song = queue.song_at(idx).unwrap();
                            if song.selected() {
                                remove_songs.push(song);
                            }
                        }

                        for song in remove_songs {
                            win.remove_song(&song);
                        }

                        // Store the current state of the playlist
                        utils::store_playlist(queue);
                    }
                }
            ));

        self.imp()
            .playlist_view
            .playlist_searchbar()
            .connect_notify_local(
                Some("search-mode-enabled"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |searchbar, _| {
                        win.set_playlist_search(searchbar.is_search_mode());
                    }
                ),
            );

        self.imp().settings.connect_changed(
            Some("enable-recoloring"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |settings, _| {
                    debug!(
                        "GSettings:enable-recoloring: {}",
                        settings.boolean("enable-recoloring")
                    );
                    if let Some(player) = this.player() {
                        let state = player.state();
                        this.update_style(state.current_song().as_ref());
                    }
                }
            ),
        );
        let _dummy = self.imp().settings.boolean("enable-recoloring");

        self.connect_close_request(move |window| {
            debug!("Saving window state");
            let settings = utils::settings_manager();
            // Only the full skin's size is worth remembering; the mini /
            // strip sizes are fixed.
            if window.imp().view_mode.get() == ViewMode::Full {
                let (width, height) = window.default_size();
                settings
                    .set_int("window-width", width)
                    .and_then(|_| settings.set_int("window-height", height))
                    .unwrap_or_else(|e| log::warn!("Unable to store the window size: {e}"));
            }

            // Remember where we were so `[playback] on_start = "resume"`
            // can pick the queue back up next launch.
            if let Some(player) = window.player() {
                let (uri, pos) = player
                    .state()
                    .current_song()
                    .map(|s| (s.uri(), player.state().position()))
                    .unwrap_or_default();
                let _ = settings.set_string("resume-uri", &uri);
                let _ = settings.set_uint64("resume-position", pos);

                if let Some(playlist_path) = window.imp().active_playlist.borrow().as_deref() {
                    let ix = player.queue().current_song_index().unwrap_or(0);
                    let _ = crate::playlist::update_resume_index(playlist_path, ix);
                }
            }

            window.unbind_queue();
            window.unbind_state();
            window.unbind_waveform();

            glib::Propagation::Proceed
        });

        self.imp()
            .playlist_view
            .playlist_searchbar()
            .set_key_capture_widget(Some(self.upcast_ref::<gtk::Widget>()));
    }

    // The initial state of the playback actions
    pub(super) fn set_initial_state(&self) {
        if let Some(player) = self.player() {
            let queue = player.queue();
            self.action_set_enabled("win.play", !queue.is_empty());
            self.action_set_enabled("win.previous", !queue.is_empty());
            self.action_set_enabled("win.next", !queue.is_last_song());

            self.action_set_enabled("queue.toggle", !queue.is_empty());
            self.action_set_enabled("queue.shuffle", queue.n_songs() > 1);
            self.action_set_enabled("win.replaygain", player.replaygain_available());

            let replaygain = self.imp().settings.enum_("replay-gain").into();
            self.set_replaygain(replaygain);

            // Manually set player state, because set_replaygain
            // only updates player state when the value changes.
            player.set_replaygain(replaygain);

            self.imp().playback_control.set_repeat_mode(
                queue.repeat_mode(),
                queue.repeat_count(),
                queue.repeat_plays(),
            );
            self.set_playlist_shuffled(queue.is_shuffled());

            // Manually update the icon on the initial empty state
            // to avoid generating the UI definition file at build
            // time
            self.imp().status_page.set_icon_name(Some(APPLICATION_ID));

            if utils::has_cached_playlist() {
                self.imp().restore_playlist_button.set_visible(true);
                self.imp()
                    .restore_playlist_button
                    .add_css_class("suggested-action");
                self.imp()
                    .add_folder_button
                    .remove_css_class("suggested-action");
            } else {
                self.imp().restore_playlist_button.set_visible(false);
                self.imp()
                    .restore_playlist_button
                    .remove_css_class("suggested-action");
                self.imp()
                    .add_folder_button
                    .add_css_class("suggested-action");
            }

            let state = player.state();
            if state.playing() || !queue.is_empty() {
                self.switch_mode(WindowMode::MainView);
            } else {
                self.switch_mode(WindowMode::InitialView);
            }
        }
    }
}
