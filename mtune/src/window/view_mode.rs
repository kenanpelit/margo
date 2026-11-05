// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Playback-speed control and the compact/full view-mode skins.

use super::*;

impl Window {
    /// Wire the header playback-speed control (scale + preset pills) to
    /// the (sticky, global) player rate.
    pub(super) fn setup_speed(&self) {
        let imp = self.imp();
        let Some(player) = self.player() else { return };

        const PRESETS: [f64; 6] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
        for rate in PRESETS {
            let label = if rate.fract() == 0.0 {
                format!("{rate:.0}×")
            } else {
                format!("{rate}×")
            };
            let btn = gtk::Button::builder()
                .label(label)
                .css_classes(["flat"])
                .build();
            btn.connect_clicked(clone!(
                #[weak(rename_to = win)]
                self,
                move |_| {
                    if let Some(p) = win.player() {
                        p.set_playback_rate(rate);
                    }
                }
            ));
            imp.speed_presets.append(&btn);
        }

        // scale -> player
        imp.speed_adjustment.connect_value_changed(clone!(
            #[weak(rename_to = win)]
            self,
            move |adj| {
                if let Some(p) = win.player() {
                    p.set_playback_rate(adj.value());
                }
            }
        ));

        // player -> scale + button label
        let sync = clone!(
            #[weak(rename_to = win)]
            self,
            move |state: &crate::audio::PlayerState| {
                let rate = state.playback_rate();
                let imp = win.imp();
                if (imp.speed_adjustment.value() - rate).abs() > 0.001 {
                    imp.speed_adjustment.set_value(rate);
                }
                let label = if rate.fract() == 0.0 {
                    format!("{rate:.0}×")
                } else {
                    format!("{rate:.2}×")
                };
                imp.speed_button.set_label(&label);
                imp.speed_button
                    .remove_css_class(if (rate - 1.0).abs() < 0.001 {
                        "accent"
                    } else {
                        "flat"
                    });
                imp.speed_button
                    .add_css_class(if (rate - 1.0).abs() < 0.001 {
                        "flat"
                    } else {
                        "accent"
                    });
            }
        );
        sync(player.state());
        player
            .state()
            .connect_notify_local(Some("playback-rate"), move |s, _| sync(s));
    }

    /// Wire the compact (mini / strip) skin: bind the now-playing widgets,
    /// the seek + volume scales, and the queue peek list to the shared
    /// player, then restore the saved view mode.
    pub(super) fn setup_compact(&self) {
        let imp = self.imp();
        let Some(player) = self.player() else { return };
        let state = player.state();

        // ── now-playing labels + cover ──
        let refresh_meta = clone!(
            #[weak(rename_to = win)]
            self,
            move |st: &crate::audio::PlayerState| {
                let imp = win.imp();
                let song = st.current_song();
                let title = song
                    .as_ref()
                    .map(|s| s.title())
                    .unwrap_or_else(|| i18n("Nothing playing"));
                imp.mini_title.set_label(&title);
                imp.mini_artist.set_label(&st.artist().unwrap_or_default());
                imp.mini_album.set_label(&st.album().unwrap_or_default());
                match st.cover() {
                    Some(tex) => imp.mini_cover.set_paintable(Some(&tex)),
                    None => imp
                        .mini_cover
                        .set_icon_name(Some("org.margo.Tune-symbolic")),
                }
            }
        );
        refresh_meta(state);
        state.connect_notify_local(
            Some("song"),
            clone!(
                #[strong]
                refresh_meta,
                move |st, _| refresh_meta(st)
            ),
        );

        // ── play/pause icon ──
        let sync_play = clone!(
            #[weak(rename_to = win)]
            self,
            move |st: &crate::audio::PlayerState| {
                win.imp().mini_play_button.set_icon_name(if st.playing() {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
            }
        );
        sync_play(state);
        state.connect_notify_local(
            Some("playing"),
            clone!(
                #[strong]
                sync_play,
                move |st, _| sync_play(st)
            ),
        );

        // ── seek scale ↔ position/duration ──
        imp.mini_seek.connect_change_value(clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, value| {
                if !win.imp().mini_seek_guard.get()
                    && let Some(p) = win.player()
                {
                    p.seek_position_rel(value.clamp(0.0, 1.0));
                }
                glib::Propagation::Proceed
            }
        ));
        let sync_pos = clone!(
            #[weak(rename_to = win)]
            self,
            move |st: &crate::audio::PlayerState| {
                let imp = win.imp();
                let pos = st.position();
                let dur = st.duration().max(1);
                imp.mini_seek_guard.set(true);
                imp.mini_seek_adjustment
                    .set_value((pos as f64 / dur as f64).clamp(0.0, 1.0));
                imp.mini_seek_guard.set(false);
                imp.mini_time.set_label(&utils::format_time(pos as i64));
            }
        );
        sync_pos(state);
        state.connect_notify_local(
            Some("position"),
            clone!(
                #[strong]
                sync_pos,
                move |st, _| sync_pos(st)
            ),
        );

        // ── volume ↔ ScaleButton ──
        imp.mini_volume_adjustment.set_value(state.volume());
        imp.mini_volume.connect_value_changed(clone!(
            #[weak(rename_to = win)]
            self,
            move |_, v| {
                if let Some(p) = win.player() {
                    p.set_volume(v);
                }
            }
        ));
        state.connect_notify_local(
            Some("volume"),
            clone!(
                #[weak(rename_to = win)]
                self,
                move |st, _| {
                    let adj = &win.imp().mini_volume_adjustment;
                    if (adj.value() - st.volume()).abs() > 0.01 {
                        adj.set_value(st.volume());
                    }
                }
            ),
        );

        // ── queue peek list ──
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let label = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["mini-queue-row"])
                .build();
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(label) = item.child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(song) = item.item().and_downcast::<Song>() else {
                return;
            };
            label.set_label(&format!("{}.  {}", item.position() + 1, song.title()));
            if song.playing() {
                label.add_css_class("current");
            } else {
                label.remove_css_class("current");
            }
        });
        imp.mini_queue.set_factory(Some(&factory));
        let selection = gtk::NoSelection::new(Some(player.queue().model().clone()));
        imp.mini_queue.set_model(Some(&selection));
        imp.mini_queue.connect_activate(clone!(
            #[weak(rename_to = win)]
            self,
            move |_, pos| {
                if let Some(p) = win.player() {
                    p.skip_to(pos);
                    p.play();
                }
            }
        ));
    }

    pub(super) fn set_view_mode(&self, mode: ViewMode) {
        let _ = self.imp().settings.set_string("view-mode", mode.as_str());
        self.apply_view_mode(mode);
    }

    pub(super) fn apply_view_mode(&self, mode: ViewMode) {
        let imp = self.imp();
        imp.view_mode.set(mode);

        // The empty-library screen owns the window until a queue exists.
        if imp.main_stack.visible_child_name().as_deref() == Some("initial-view") {
            return;
        }

        let strip = mode == ViewMode::Strip;
        imp.compact_box.set_css_classes(if strip {
            &["compact-view", "strip"]
        } else {
            &["compact-view"]
        });
        imp.mini_cover.set_pixel_size(if strip { 40 } else { 56 });
        imp.mini_album.set_visible(!strip);
        imp.mini_queue_scroll.set_visible(!strip);
        imp.mini_queue_sep.set_visible(!strip);
        // Strip has no queue peek, so pin a sensible width for the
        // control row; mini sizes to its content (cover + meta + queue).
        imp.compact_box
            .set_width_request(if strip { 380 } else { -1 });

        match mode {
            ViewMode::Full => {
                imp.main_stack.set_visible_child_name("main-view");
                self.set_size_request(360, 480);
                self.set_resizable(true);
                self.set_default_size(
                    imp.settings.int("window-width").max(360),
                    imp.settings.int("window-height").max(480),
                );
            }
            // Compact skins are fixed to their natural content size —
            // `resizable(false)` makes GTK track the content exactly, so
            // there are no magic dimensions to get wrong.
            ViewMode::Mini | ViewMode::Strip => {
                imp.main_stack.set_visible_child_name("compact-view");
                self.set_size_request(-1, -1);
                self.set_default_size(-1, -1);
                self.set_resizable(false);
            }
        }
    }
}
