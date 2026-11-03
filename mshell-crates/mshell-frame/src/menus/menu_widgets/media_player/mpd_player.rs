//! MPD player card for the media menu.
//!
//! Parallel to [`super::media_player`] (the MPRIS card) rather than
//! forcing both backends through one generic component: MPD's simpler
//! capability set has no shuffle/loop concept that maps cleanly from
//! MPRIS's, and there is always at most one MPD player, so a second small
//! independent component is easier to read and change than threading an
//! `AnyPlayer` abstraction through the 700-line MPRIS card. Reuses the
//! same CSS classes for visual parity in the switcher, and the seek-bar
//! plumbing in [`super::seek`].

use super::seek::{seek_fraction, set_scale_silently, setup_debounced_seek};
use mshell_common::WatcherToken;
use mshell_services::mpd::MpdPlayer;
use mshell_utils::media::format_duration;
use relm4::gtk::glib;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::Duration;
use wayle_media::types::PlaybackState;

pub(crate) struct MpdPlayerModel {
    pub player: Arc<MpdPlayer>,
    /// Keeps the player's watchers alive for the model's lifetime — see
    /// the identical field on `MediaPlayerModel`.
    _watcher_token: WatcherToken,
    title: String,
    artist: String,
    current_track_time: String,
    track_length: String,
    scale_value_changed_signal: Option<glib::SignalHandlerId>,
    playback_state: PlaybackState,
    can_seek: bool,
}

#[derive(Debug)]
pub(crate) enum MpdPlayerInput {
    ScaleChanged(f64),
    ScaleClicked(f64),
    PreviousClicked,
    NextClicked,
    PlayPauseClicked,
}

#[derive(Debug)]
pub(crate) enum MpdPlayerOutput {}

pub(crate) struct MpdPlayerInit {
    pub player: Arc<MpdPlayer>,
}

#[derive(Debug)]
pub(crate) enum MpdPlayerCommandOutput {
    Changed,
}

#[relm4::component(pub)]
impl Component for MpdPlayerModel {
    type CommandOutput = MpdPlayerCommandOutput;
    type Input = MpdPlayerInput;
    type Output = MpdPlayerOutput;
    type Init = MpdPlayerInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "media-player-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,

            gtk::Box {
                add_css_class: "media-player-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_hexpand: true,
                set_spacing: 12,

                #[name = "cover"]
                gtk::Image {
                    add_css_class: "media-player-cover",
                    set_pixel_size: 64,
                    set_valign: gtk::Align::Center,
                },

                gtk::Box {
                    add_css_class: "media-player-info",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                    set_spacing: 2,

                    gtk::Label {
                        add_css_class: "media-player-track",
                        #[watch]
                        set_label: model.title.as_str(),
                        set_xalign: 0.0,
                        set_single_line_mode: true,
                        set_wrap: false,
                        set_max_width_chars: -1,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },

                    gtk::Label {
                        add_css_class: "media-player-artist",
                        #[watch]
                        set_label: model.artist.as_str(),
                        set_xalign: 0.0,
                        set_single_line_mode: true,
                        set_wrap: false,
                        set_max_width_chars: -1,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },

                    gtk::Box {
                        add_css_class: "media-player-progress-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_hexpand: true,

                        gtk::Label {
                            add_css_class: "media-player-time",
                            #[watch]
                            set_label: model.current_track_time.as_str(),
                        },

                        #[name = "scale"]
                        gtk::Scale {
                            add_css_class: "ok-progress-bar",
                            set_hexpand: true,
                            set_can_focus: false,
                            set_focus_on_click: false,
                            set_range: (0.0, 1.0),
                            #[watch]
                            set_sensitive: model.can_seek,
                        },

                        gtk::Label {
                            add_css_class: "media-player-time",
                            #[watch]
                            set_label: model.track_length.as_str(),
                        },
                    },
                },
            },

            gtk::Box {
                add_css_class: "media-player-controls",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Center,
                set_spacing: 8,

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    connect_clicked[sender] => move |_| {
                        sender.input(MpdPlayerInput::PreviousClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("media-skip-previous-symbolic"),
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    add_css_class: "media-player-ctl-primary",
                    set_hexpand: false,
                    set_vexpand: false,
                    connect_clicked[sender] => move |_| {
                        sender.input(MpdPlayerInput::PlayPauseClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_icon_name: match model.playback_state {
                            PlaybackState::Playing => Some("media-pause-symbolic"),
                            PlaybackState::Paused | PlaybackState::Stopped => Some("media-play-symbolic"),
                        },
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    connect_clicked[sender] => move |_| {
                        sender.input(MpdPlayerInput::NextClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("media-skip-next-symbolic"),
                    },
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = MpdPlayerModel {
            player: init.player,
            _watcher_token: WatcherToken::new(),
            title: String::new(),
            artist: String::new(),
            current_track_time: format_duration(Duration::ZERO),
            track_length: format_duration(Duration::ZERO),
            scale_value_changed_signal: None,
            playback_state: PlaybackState::Stopped,
            can_seek: true,
        };

        subscribe(&sender, &mut model);

        let widgets = view_output!();

        model.scale_value_changed_signal = Some(setup_debounced_seek(
            &widgets.scale,
            &sender,
            MpdPlayerInput::ScaleChanged,
            MpdPlayerInput::ScaleClicked,
        ));

        read_display(&mut model);
        apply_scale(&widgets, &model);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MpdPlayerInput::ScaleChanged(value) => {
                let position =
                    Duration::from_secs_f64(value * self.player.duration.get().as_secs_f64());
                self.current_track_time = format_duration(position);
            }
            MpdPlayerInput::ScaleClicked(value) => {
                let duration = self.player.duration.get();
                let position = Duration::from_secs_f64(value * duration.as_secs_f64());
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.seek(position).await;
                });
            }
            MpdPlayerInput::PreviousClicked => {
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.previous().await;
                });
            }
            MpdPlayerInput::NextClicked => {
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.next().await;
                });
            }
            MpdPlayerInput::PlayPauseClicked => {
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.play_pause().await;
                });
            }
        }
        self.update_view(widgets, _sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MpdPlayerCommandOutput::Changed => {
                read_display(self);
                apply_scale(widgets, self);
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Watch every field this card renders — called once from `init` only.
/// `wayle_core::Property::watch()` replays the current value immediately
/// on subscribe, so calling this again from the `Changed` handler (as a
/// naive "re-arm on change" pattern would) re-triggers `Changed`
/// synchronously on every one of the 6 streams and spins the executor at
/// 100% CPU — this bit us once already, see
/// `reference_watch_resubscribe_loop` in project memory.
fn subscribe(sender: &ComponentSender<MpdPlayerModel>, model: &mut MpdPlayerModel) {
    let token = model._watcher_token.reset();
    let player = &model.player;
    mshell_common::watch_cancellable!(
        sender,
        token,
        [
            player.title.watch(),
            player.artist.watch(),
            player.cover_art.watch(),
            player.playback_state.watch(),
            player.position.watch(),
            player.duration.watch(),
        ],
        |out| {
            let _ = out.send(MpdPlayerCommandOutput::Changed);
        }
    );
}

fn read_display(model: &mut MpdPlayerModel) {
    model.title = model.player.title.get();
    model.artist = model.player.artist.get();
    model.playback_state = model.player.playback_state.get();
    model.current_track_time = format_duration(model.player.position.get());
    model.track_length = format_duration(model.player.duration.get());
}

fn apply_scale(widgets: &MpdPlayerModelWidgets, model: &MpdPlayerModel) {
    let fraction = seek_fraction(model.player.position.get(), model.player.duration.get());
    set_scale_silently(
        &widgets.scale,
        model.scale_value_changed_signal.as_ref(),
        fraction,
    );
    widgets
        .cover
        .set_from_file(model.player.cover_art.get().as_deref());
    if model.player.cover_art.get().is_none() {
        widgets.cover.set_icon_name(Some("media-play-symbolic"));
    }
}
