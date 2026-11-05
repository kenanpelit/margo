// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Filling the play queue from files and keeping the window bound to it.

use super::*;

impl Window {
    pub(super) fn queue_songs(&self, queue: Vec<gio::File>) {
        if queue.is_empty() {
            self.add_toast(i18n("No available song found"));
            return;
        }

        self.switch_mode(WindowMode::MainView);

        // Disable actions on the queue; loading is "atomic"
        self.action_set_enabled("queue.add-song", false);
        self.action_set_enabled("queue.add-folder", false);
        self.action_set_enabled("queue.clear", false);

        self.set_playlist_visible(true);
        self.imp().playlist_view.begin_loading();

        // Begin the trace
        let now = Instant::now();

        // Turn the list of files into songs one at a time into the main loop
        let n_files = queue.len() as u32;

        let mut files = queue.into_iter();
        let mut songs = Vec::new();
        let mut cur_file: u32 = 0;
        let mut duplicates: u32 = 0;

        glib::idle_add_local(clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move || {
                files
                    .next()
                    .map(|f| {
                        win.imp().playlist_view.update_loading(cur_file, n_files);
                        if let Ok(s) = Song::from_uri(f.uri().as_str())
                            && let Some(player) = win.player()
                        {
                            let queue = player.queue();
                            if queue.contains(&s) {
                                duplicates += 1;
                            } else {
                                songs.push(s);
                                cur_file += 1;
                            }
                        }
                    })
                    .map(|_| glib::ControlFlow::Continue)
                    .unwrap_or_else(|| {
                        debug!(
                            "Total loading time for {} files: {} ms",
                            n_files,
                            now.elapsed().as_millis()
                        );

                        // Re-enable the actions
                        win.action_set_enabled("queue.add-song", true);
                        win.action_set_enabled("queue.add-folder", true);
                        win.action_set_enabled("queue.clear", true);

                        if songs.is_empty() {
                            if duplicates == 0 {
                                win.add_toast(i18n("No songs found"));
                            }
                        } else if let Some(player) = win.player() {
                            let queue = player.queue();
                            let was_empty = queue.is_empty();

                            win.imp().playlist_view.end_loading();

                            // Bulk add to avoid hammering the UI with list model updates
                            queue.add_songs(&songs);

                            // Store the current state of the playlist
                            utils::store_playlist(queue);

                            debug!(
                                "Queue was empty: {}, new size: {}",
                                was_empty,
                                queue.n_songs()
                            );
                            if was_empty {
                                match win.imp().pending_start.borrow_mut().take() {
                                    Some(StartIntent::Resume(uri, pos)) => {
                                        match queue.position_of_uri(&uri) {
                                            Some(ix) => {
                                                player.skip_to(ix);
                                                // Defer the seek — the pipeline
                                                // isn't ready this instant.
                                                player.queue_resume_seek(pos);
                                            }
                                            None => player.skip_to(0),
                                        }
                                    }
                                    Some(StartIntent::AtIndex(ix)) => {
                                        let clamped = ix.min(queue.n_songs().saturating_sub(1));
                                        player.skip_to(clamped);
                                    }
                                    Some(StartIntent::Nothing) => {}
                                    _ => player.skip_to(0),
                                }
                            }

                            // Allow jumping to the song we just added
                            if songs.len() == 1 {
                                // If we added a single song, and the queue was empty, we
                                // dispense with the pleasantries and we start playing
                                // immediately; otherwise, we let the user choose whether
                                // to jump to the newly added song
                                if was_empty {
                                    player.play();
                                } else {
                                    win.add_skip_to_toast(
                                        i18n("Added a new song"),
                                        i18n("Play"),
                                        queue.n_songs() - 1,
                                    );
                                }
                            } else {
                                let msg = ni18n_f(
                                    // Translators: the `{}` must be left unmodified;
                                    // it will be expanded to the number of songs added
                                    // to the playlist
                                    "Added one song",
                                    "Added {} songs",
                                    songs.len() as u32,
                                    &[&songs.len().to_string()],
                                );

                                win.add_toast(msg);
                            }
                        }

                        glib::ControlFlow::Break
                    })
            }
        ));
    }

    pub(super) fn add_files_to_queue(&self, model: &gio::ListModel) {
        let mut queue: Vec<gio::File> = vec![];

        for pos in 0..model.n_items() {
            let file = model.item(pos).unwrap().downcast::<gio::File>().unwrap();

            if let Ok(info) = file.query_info(
                "standard::name,standard::display-name,standard::type,standard::content-type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            ) {
                match info.file_type() {
                    gio::FileType::Regular => {
                        if let Some(content_type) = info.content_type()
                            && gio::content_type_is_mime_type(&content_type, "audio/*")
                        {
                            debug!("Adding file '{}' to the queue", file.uri());
                            queue.push(file);
                        }
                    }
                    gio::FileType::Directory => {
                        debug!("Adding folder '{}' to the queue", file.uri());
                        let files = utils::load_files_from_folder(&file, true);
                        queue.extend(files);
                    }
                    _ => (),
                }
            }
        }

        self.queue_songs(queue);
    }

    // Bind the Queue to the UI
    pub(super) fn bind_queue(&self) {
        if let Some(player) = self.player() {
            let queue = player.queue();

            let notify_nsongs_id = queue.connect_notify_local(
                Some("n-songs"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |queue, _| {
                        debug!("queue.n_songs() = {}", queue.n_songs());
                        if queue.is_empty() {
                            win.set_initial_state();
                            win.reset_queue();
                        } else {
                            win.action_set_enabled("queue.toggle", true);
                            win.action_set_enabled("queue.shuffle", queue.n_songs() > 1);

                            win.action_set_enabled("win.play", true);
                            win.action_set_enabled("win.previous", true);
                            win.action_set_enabled("win.next", queue.n_songs() > 1);
                        }

                        if queue.n_songs() == 1
                            && let Some(p) = win.player()
                        {
                            p.skip_next();
                        }

                        win.update_playlist_time();
                    }
                ),
            );
            self.imp().notify_nsongs_id.replace(Some(notify_nsongs_id));

            queue.connect_notify_local(
                Some("repeat-mode"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |queue, _| {
                        win.imp().playback_control.set_repeat_mode(
                            queue.repeat_mode(),
                            queue.repeat_count(),
                            queue.repeat_plays(),
                        );
                    }
                ),
            );
            // A live `repeat-count` change (e.g. `mshellctl mtune
            // repeat-count`) should refresh the tooltip immediately even
            // when the mode itself doesn't change.
            queue.connect_notify_local(
                Some("repeat-count"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |queue, _| {
                        win.imp().playback_control.set_repeat_mode(
                            queue.repeat_mode(),
                            queue.repeat_count(),
                            queue.repeat_plays(),
                        );
                    }
                ),
            );
            // Advance the "2/3"-style readout live as the current track
            // replays under `RepeatEach`.
            queue.connect_notify_local(
                Some("repeat-plays"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |queue, _| {
                        win.imp().playback_control.set_repeat_mode(
                            queue.repeat_mode(),
                            queue.repeat_count(),
                            queue.repeat_plays(),
                        );
                    }
                ),
            );

            let notify_current_id = queue.connect_notify_local(
                Some("current"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |queue, _| {
                        if queue.is_last_song() {
                            match queue.repeat_mode() {
                                RepeatMode::Consecutive => {
                                    win.action_set_enabled("win.next", false);
                                }
                                _ => {
                                    win.action_set_enabled("win.next", true);
                                }
                            }
                        } else {
                            win.action_set_enabled("win.next", true);
                        }
                    }
                ),
            );
            self.imp()
                .notify_current_id
                .replace(Some(notify_current_id));
        }
    }

    pub(super) fn unbind_queue(&self) {
        if let Some(player) = self.player() {
            let queue = player.queue();
            if let Some(id) = self.imp().notify_nsongs_id.take() {
                queue.disconnect(id);
            }
            if let Some(id) = self.imp().notify_current_id.take() {
                queue.disconnect(id);
            }
        }
    }
}
