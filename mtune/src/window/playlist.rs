// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! The playlist sidebar: visibility/shuffle/selection/search state, saved
//! playlists, and its setup and labels.

use super::*;

impl Window {
    pub(super) fn playlist_visible(&self) -> bool {
        self.imp().playlist_visible.get()
    }

    pub(super) fn set_playlist_visible(&self, visible: bool) {
        if visible != self.imp().playlist_visible.replace(visible) {
            self.imp().split_view.set_show_sidebar(visible);
            self.notify("playlist-visible");
        }
    }

    pub(super) fn playlist_shuffled(&self) -> bool {
        self.imp().playlist_shuffled.get()
    }

    pub(super) fn set_playlist_shuffled(&self, shuffled: bool) {
        let imp = self.imp();

        if shuffled != imp.playlist_shuffled.replace(shuffled) {
            if let Some(player) = self.player() {
                let queue = player.queue();
                let state = player.state();
                let reset_song = queue.is_first_song() && !state.playing();

                queue.set_shuffled(shuffled);

                if reset_song {
                    player.skip_to(0);
                }
            }

            self.notify("playlist-shuffled");
        }
    }

    pub(super) fn playlist_selection(&self) -> bool {
        self.imp().playlist_selection.get()
    }

    pub(super) fn set_playlist_selection(&self, selection: bool) {
        let imp = self.imp();

        if selection != imp.playlist_selection.replace(selection) {
            if !selection && let Some(player) = self.player() {
                let queue = player.queue();
                queue.unselect_all_songs();
            }

            self.imp()
                .playlist_view
                .queue_actionbar()
                .set_revealed(selection);

            self.notify("playlist-selection");
        }
    }

    pub(super) fn playlist_search(&self) -> bool {
        self.imp().playlist_search.get()
    }

    pub(super) fn set_playlist_search(&self, search: bool) {
        let imp = self.imp();

        if search != imp.playlist_search.replace(search) {
            if search {
                self.set_playlist_visible(true);
            }
            imp.playlist_view.set_search(search);
            self.notify("playlist-search");
        }
    }

    pub(super) fn restore_playlist(&self) {
        if let Some(songs) = utils::load_cached_songs() {
            // `queue_songs` alone never looks at `pending_start` — that's
            // only consumed once the queue lands, and only `open_files` /
            // `load_library_files` set it first. Calling `queue_songs`
            // directly (as this used to) left `pending_start` at its
            // default `None`, so playback always fell through to the
            // blind `skip_to(0)` in that match's `_` arm: the button
            // restored *which songs* were queued, never *where you were*
            // in them. `[playback] on_start = "resume"` already builds
            // this exact intent from the same `resume-uri` /
            // `resume-position` keys on every normal startup — reuse it
            // here so the button matches that behaviour.
            let settings = utils::settings_manager();
            let uri = settings.string("resume-uri");
            let pos = settings.uint64("resume-position");
            let intent = if uri.is_empty() {
                StartIntent::Top
            } else {
                StartIntent::Resume(uri.to_string(), pos)
            };
            self.load_library_files(songs, intent);
        }
    }

    /// Replace the queue with the tracks referenced by a playlist file
    /// (`.m3u` / `.m3u8` / `.pls`).
    pub fn open_playlist_file(&self, path: &std::path::Path) {
        let files: Vec<gio::File> = crate::playlist::parse(path)
            .into_iter()
            .map(gio::File::for_path)
            .collect();
        if files.is_empty() {
            self.add_toast(i18n("Playlist is empty or unreadable"));
            return;
        }
        if let Some(p) = self.player() {
            p.clear_queue();
        }
        let is_saved = path.starts_with(crate::playlist::library_dir());
        self.imp()
            .active_playlist
            .replace(is_saved.then(|| path.to_path_buf()));
        let intent = is_saved
            .then(|| crate::playlist::resume_index(path))
            .flatten()
            .map(StartIntent::AtIndex)
            .unwrap_or(StartIntent::Top);
        self.imp().pending_start.replace(Some(intent));
        self.queue_songs(files);
    }

    /// The saved playlist backing the current queue, if any -- see
    /// `active_playlist`'s field doc.
    pub fn active_playlist(&self) -> Option<PathBuf> {
        self.imp().active_playlist.borrow().clone()
    }

    /// Load a playlist from the library by display name.
    pub(super) fn load_saved_playlist(&self, name: &str) {
        self.open_playlist_file(&crate::playlist::saved_path(name));
    }

    /// Write the current queue to the library under `name`.
    pub(super) fn save_current_playlist(&self, name: &str) {
        let Some(player) = self.player() else { return };
        match crate::playlist::save(name, player.queue()) {
            Ok(_) => self.add_toast(i18n_f("Saved playlist “{}”", &[name])),
            Err(e) => {
                debug!("save playlist: {e}");
                self.add_toast(i18n("Could not save the playlist"));
            }
        }
    }

    pub(super) fn setup_playlist(&self) {
        let imp = self.imp();

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(clone!(
            #[weak(rename_to = win)]
            self,
            move |_, item| {
                let row = QueueRow::default();
                let list_item = item.downcast_ref::<gtk::ListItem>().unwrap();
                list_item.set_child(Some(&row));

                row.connect_notify_local(
                    Some("selected"),
                    clone!(
                        #[weak]
                        win,
                        move |_, _| {
                            win.update_selected_count();
                        }
                    ),
                );

                win.bind_property("playlist-selection", &row, "selection-mode")
                    .sync_create()
                    .build();

                list_item
                    .bind_property("item", &row, "song")
                    .sync_create()
                    .build();

                list_item
                    .property_expression("item")
                    .chain_property::<Song>("artist")
                    .bind(&row, "song-artist", gtk::Widget::NONE);
                list_item
                    .property_expression("item")
                    .chain_property::<Song>("title")
                    .bind(&row, "song-title", gtk::Widget::NONE);
                list_item
                    .property_expression("item")
                    .chain_property::<Song>("cover")
                    .bind(&row, "song-cover", gtk::Widget::NONE);
                list_item
                    .property_expression("item")
                    .chain_property::<Song>("playing")
                    .bind(&row, "playing", gtk::Widget::NONE);
                list_item
                    .property_expression("item")
                    .chain_property::<Song>("selected")
                    .bind(&row, "selected", gtk::Widget::NONE);
                list_item.property_expression("position").bind(
                    &row,
                    "track-number",
                    gtk::Widget::NONE,
                );
            }
        ));
        imp.playlist_view
            .queue_view()
            .set_factory(Some(&factory.upcast::<gtk::ListItemFactory>()));

        if let Some(player) = self.player() {
            let queue = player.queue();

            let filter = FuzzyFilter::new();
            let filter_model =
                gtk::FilterListModel::new(Some(queue.model().clone()), Some(filter.clone()));
            let sorter = FuzzySorter::new();
            let sorter_model = gtk::SortListModel::new(Some(filter_model), Some(sorter.clone()));
            let selection = gtk::NoSelection::new(Some(sorter_model.clone()));
            imp.playlist_view
                .queue_view()
                .set_model(Some(selection.upcast_ref::<gtk::SelectionModel>()));
            imp.playlist_view.queue_view().connect_activate(clone!(
                #[weak(rename_to = win)]
                self,
                #[weak]
                selection,
                move |_, pos| {
                    let song = selection
                        .upcast::<gio::ListModel>()
                        .item(pos)
                        .unwrap()
                        .downcast::<Song>()
                        .unwrap();

                    if let Some(player) = win.player() {
                        let queue = player.queue();

                        let mut real_pos = None;
                        for i in 0..queue.model().n_items() {
                            if let Some(item) = queue.model().item(i) {
                                let s = item.downcast::<Song>().unwrap();
                                if s.equals(&song) {
                                    real_pos = Some(i);
                                    break;
                                }
                            }
                        }

                        if let Some(real_pos) = real_pos {
                            if win.playlist_selection() {
                                queue.select_song_at(real_pos);
                            } else if queue.current_song_index() != Some(real_pos) {
                                player.skip_to(real_pos);
                                player.play();
                            } else if !player.state().playing() {
                                player.play();
                            }
                        }
                    }
                }
            ));

            imp.playlist_filtermodel
                .replace(Some(sorter_model.upcast::<gio::ListModel>()));

            imp.playlist_view
                .playlist_searchentry()
                .bind_property("text", &filter, "search")
                .sync_create()
                .build();
            imp.playlist_view
                .playlist_searchentry()
                .bind_property("text", &sorter, "search")
                .sync_create()
                .build();
            imp.playlist_view
                .playlist_searchentry()
                .connect_search_changed(clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |_| {
                        if let Some(adjustment) = win.imp().playlist_view.queue_view().vadjustment()
                        {
                            adjustment.set_value(0.0);
                        }
                    }
                ));
        }
    }

    pub(super) fn update_playlist_time(&self) {
        if let Some(player) = self.player() {
            let queue = player.queue();
            let n_songs = queue.n_songs();
            if let Some(current) = queue.current_song_index() {
                let mut remaining_time = 0;
                for pos in 0..n_songs {
                    let song = queue.song_at(pos).unwrap();
                    if pos >= current {
                        remaining_time += song.duration();
                    }
                }

                let remaining_min = ((remaining_time - (remaining_time % 60)) / 60) as u32;
                let remaining_hrs = (remaining_min - (remaining_min % 60)) / 60;

                let remaining_str = if remaining_hrs > 0 {
                    // Translators: the `{}` must be left unmodified, and
                    // it will be replaced by the number of minutes remaining
                    // in the string "N hours M minutes remaining"
                    let minutes = ni18n_f(
                        "{} minute",
                        "{} minutes",
                        remaining_min % 60,
                        &[&(remaining_min % 60).to_string()],
                    );

                    // Translators: `{hours}` and `{minutes}` must be left
                    // unmodified, and they will be replaced by number of
                    // hours and by the translated number of minutes
                    // remaining, respectively
                    ni18n_k(
                        "{hours} hour {minutes} remaining",
                        "{hours} hours {minutes} remaining",
                        remaining_hrs,
                        &[("hours", &remaining_hrs.to_string()), ("minutes", &minutes)],
                    )
                } else {
                    ni18n_f(
                        // Translators: the '{}' must be left unmodified, and
                        // it will be replaced by the number of minutes remaining
                        // in the playlist
                        "{} minute remaining",
                        "{} minutes remaining",
                        remaining_min,
                        &[&remaining_min.to_string()],
                    )
                };

                self.imp()
                    .playlist_view
                    .queue_length_label()
                    .set_label(&remaining_str);
                self.imp()
                    .playlist_view
                    .queue_length_label()
                    .set_visible(true);
            } else {
                self.imp()
                    .playlist_view
                    .queue_length_label()
                    .set_visible(false);
            }
        }
    }

    pub(super) fn scroll_playlist_to_song(&self) {
        let queue_view = self.imp().playlist_view.queue_view();
        if let Some(player) = self.player()
            && let Some(current_idx) = player.queue().current_song_index()
        {
            debug!("Scrolling playlist to {}", current_idx);
            queue_view
                .upcast_ref::<gtk::Widget>()
                .activate_action("list.scroll-to-item", Some(&current_idx.to_variant()))
                .expect("Failed to activate action");
        }
    }

    pub(super) fn update_selected_count(&self) {
        if let Some(player) = self.player() {
            let queue = player.queue();
            let n_selected = queue.n_selected_songs();

            let selected_str = if n_selected == 0 {
                i18n("No song selected")
            } else {
                ni18n_f(
                    // Translators: The '{}' must be left unmodified, and
                    // it is expanded to the number of songs selected
                    "{} song selected",
                    "{} songs selected",
                    n_selected,
                    &[&n_selected.to_string()],
                )
            };

            self.imp()
                .playlist_view
                .queue_selected_label()
                .set_label(&selected_str);
        }
    }
}
