//! Media-player bar pill.
//!
//! Render-only mirror of the active MPRIS player (Spotify, mpd,
//! browsers/YouTube, …): a play/pause glyph plus the current
//! track title, ellipsized so the pill stays a sane width. When
//! nothing is playing it collapses to a single music-note icon.
//! Click emits `Clicked`; the frame toggles the layer-shell
//! `MenuType::MediaPlayer`, whose panel (the rich
//! cover-art / seek / controls surface) lives in
//! `menu_widgets/media_player/`.
//!
//! The active player can change at runtime, so a `WatcherToken`
//! is reset on every `ActivePlayerChanged` — that cancels the
//! previous player's metadata/state watchers before subscribing
//! to the new one's.

use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_media::types::PlaybackState;

pub(crate) struct MediaPlayerModel {
    watcher_token: WatcherToken,
    has_player: bool,
    playing: bool,
    title: String,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerOutput {
    Clicked,
}

pub(crate) struct MediaPlayerInit {}

#[derive(Debug)]
pub(crate) enum MediaPlayerCommandOutput {
    /// Player list / active player changed — re-subscribe.
    ActivePlayerChanged,
    /// The active player's metadata or playback state changed.
    TrackChanged,
}

#[relm4::component(pub)]
impl Component for MediaPlayerModel {
    type CommandOutput = MediaPlayerCommandOutput;
    type Input = MediaPlayerInput;
    type Output = MediaPlayerOutput;
    type Init = MediaPlayerInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["media-player-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(MediaPlayerInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name = "icon"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },

                    #[name = "label"]
                    gtk::Label {
                        add_css_class: "media-player-bar-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_ellipsize: pango::EllipsizeMode::End,
                        set_max_width_chars: 24,
                    },
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_media_players_watcher(
            &sender,
            || MediaPlayerCommandOutput::ActivePlayerChanged,
            || MediaPlayerCommandOutput::ActivePlayerChanged,
        );

        let mut model = MediaPlayerModel {
            watcher_token: WatcherToken::new(),
            has_player: false,
            playing: false,
            title: String::new(),
        };

        subscribe_active_player(&sender, &mut model.watcher_token);

        let widgets = view_output!();
        read_active(&mut model);
        apply_visual(&widgets, &model);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MediaPlayerInput::Clicked => {
                let _ = sender.output(MediaPlayerOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MediaPlayerCommandOutput::ActivePlayerChanged => {
                subscribe_active_player(&sender, &mut self.watcher_token);
                read_active(self);
            }
            MediaPlayerCommandOutput::TrackChanged => {
                read_active(self);
            }
        }
        apply_visual(widgets, self);
    }
}

/// Re-subscribe the title / playback-state watchers to whichever
/// player is *currently* active: resets `watcher_token` (cancels
/// the previous player's watchers), then — if there is an active
/// player — wires its title + playback-state streams under the
/// fresh token.
fn subscribe_active_player(
    sender: &ComponentSender<MediaPlayerModel>,
    watcher_token: &mut WatcherToken,
) {
    let token = watcher_token.reset();
    let Some(player) = media_service().active_player.get() else {
        return;
    };
    let title = player.metadata.title.clone();
    let playback_state = player.playback_state.clone();
    watch_cancellable!(
        sender,
        token,
        [title.watch(), playback_state.watch()],
        |out| {
            let _ = out.send(MediaPlayerCommandOutput::TrackChanged);
        }
    );
}

/// Refresh the model from the active player's current state.
fn read_active(model: &mut MediaPlayerModel) {
    match media_service().active_player.get() {
        Some(player) => {
            model.has_player = true;
            model.playing = player.playback_state.get() == PlaybackState::Playing;
            model.title = player.metadata.title.get();
        }
        None => {
            model.has_player = false;
            model.playing = false;
            model.title.clear();
        }
    }
}

fn apply_visual(widgets: &MediaPlayerModelWidgets, model: &MediaPlayerModel) {
    if !model.has_player {
        // Idle: a single music-note glyph, no label.
        widgets.icon.set_icon_name(Some("audio-x-generic-symbolic"));
        widgets.label.set_visible(false);
        widgets.root.set_tooltip_text(Some("No media playing"));
        return;
    }

    // State indicator (the pill is render-only — click opens the
    // panel — so the glyph reflects state, it isn't a control).
    widgets.icon.set_icon_name(Some(if model.playing {
        "media-play-symbolic"
    } else {
        "media-pause-symbolic"
    }));

    let title = if model.title.trim().is_empty() {
        "Unknown track"
    } else {
        model.title.trim()
    };
    widgets.label.set_label(title);
    widgets.label.set_visible(true);
    widgets.root.set_tooltip_text(Some(title));
}
