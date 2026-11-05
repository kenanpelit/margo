// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! The now-playing view: waveform, play button, position labels, song
//! details, cover and the cover-tinted style.

use super::*;

impl Window {
    pub(super) fn setup_waveform(&self) {
        if let Some(player) = self.player() {
            let peaks = player.waveform_generator().peaks();
            self.imp().waveform_view.set_peaks(peaks);

            let notify_peaks_id = player.waveform_generator().connect_notify_local(
                Some("has-peaks"),
                clone!(
                    #[weak(rename_to = win)]
                    self,
                    move |wf, _| {
                        let peaks = wf.peaks();
                        win.imp().waveform_view.set_peaks(peaks);
                    }
                ),
            );
            self.imp().notify_peaks_id.replace(Some(notify_peaks_id));
        }
    }

    pub(super) fn unbind_waveform(&self) {
        if let Some(player) = self.player()
            && let Some(id) = self.imp().notify_peaks_id.take()
        {
            player.waveform_generator().disconnect(id);
        }
    }

    pub(super) fn update_play_button(&self) {
        if let Some(player) = self.player() {
            let state = player.state();
            let play_button = self.imp().playback_control.play_button();
            if state.playing() {
                play_button.set_icon_name("media-playback-pause-symbolic");
            } else {
                play_button.set_icon_name("media-playback-start-symbolic");
            }
        }
    }

    pub(super) fn update_position_labels(&self) {
        if let Some(player) = self.player() {
            let state = player.state();
            if state.current_song().is_some() {
                let elapsed = state.position();
                let duration = state.duration();
                let remaining = duration.saturating_sub(elapsed);
                self.set_song_time(Some(elapsed), Some(remaining));

                let position = state.position() as f64 / state.duration() as f64;
                self.set_song_position(position);
            } else {
                self.set_song_time(None, None);
                self.set_song_position(0.0);
            }
        }
    }

    pub(super) fn update_song(&self) {
        if let Some(player) = self.player() {
            let state = player.state();
            self.scroll_playlist_to_song();
            self.update_playlist_time();
            self.update_title(state.current_song().as_ref());
            self.update_style(state.current_song().as_ref());
        }
    }

    pub(super) fn update_cover(&self) {
        if let Some(player) = self.player() {
            let state = player.state();
            let song_cover = self.imp().song_cover.get();
            if let Some(cover) = state.cover() {
                song_cover.album_image().set_cover(Some(&cover));
                song_cover.show_cover_image(true);
            } else {
                song_cover.album_image().set_cover(None);
                song_cover.show_cover_image(false);
            }
        }
    }

    pub(super) fn setup_provider(&self) {
        let imp = self.imp();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &imp.provider, 400);
        }
    }

    pub(super) fn update_style(&self, song: Option<&Song>) {
        let imp = self.imp();

        if !imp.settings.boolean("enable-recoloring") {
            imp.provider.load_from_string("");
            imp.main_stack.remove_css_class("main-window");
            return;
        }

        if let Some(song) = song
            && let Some(bg_colors) = song.cover_palette()
        {
            let mut css = String::new();

            css.push_str(":root {");

            let n_colors = bg_colors.len();
            for (i, color) in bg_colors.iter().enumerate().take(n_colors) {
                let s = format!("--background-color-{}: {};", i, color);
                css.push_str(&s);
            }

            for i in n_colors + 1 - 1..5 {
                css.push_str(&format!(
                    "--background-color-{}: var(--window-bg-color);",
                    i
                ));
            }

            css.push('}');

            imp.provider.load_from_string(&css);
            if !imp.main_stack.has_css_class("main-window") {
                imp.main_stack.add_css_class("main-window");
            }

            self.action_set_enabled("win.enable-recoloring", true);

            return;
        }

        imp.provider.load_from_string("");
        imp.main_stack.remove_css_class("main-window");
        self.action_set_enabled("win.enable-recoloring", false);
    }

    pub(super) fn update_title(&self, song: Option<&Song>) {
        if let Some(song) = song {
            self.set_title(Some(&format!("{} - {}", song.artist(), song.title())));
        } else {
            self.set_title(Some("Tune"));
        }
    }
}
