//! Media-player bar pill.
//!
//! Render-only mirror of whichever MPRIS player is *currently
//! playing* — Spotify, mpd, browsers/YouTube, mpv, … — picked
//! from the whole player list rather than just `active_player`
//! (wayle only re-selects that on player add/remove, not when a
//! player starts playing).
//!
//! The pill leads with the album cover (resolved by wayle's art
//! cache) and shows `track — artist`, ellipsized; idle collapses
//! to a single media glyph.
//!
//! Interactions:
//!   * left click  → toggle the layer-shell `MenuType::MediaPlayer`
//!     panel (cover art / seek / controls).
//!   * right click → play / pause the displayed player in place.
//!
//! Every player's title / artist / cover / playback_state is
//! watched under a `WatcherToken` reset whenever the player list
//! changes, so the pill follows playback across players.

use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

pub(crate) struct MediaPlayerModel {
    watcher_token: WatcherToken,
    has_player: bool,
    playing: bool,
    title: String,
    artist: String,
    cover_art: Option<String>,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerInput {
    Clicked,
    PlayPauseClicked,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerOutput {
    Clicked,
}

pub(crate) struct MediaPlayerInit {}

#[derive(Debug)]
pub(crate) enum MediaPlayerCommandOutput {
    /// Player list / active player changed — re-subscribe.
    PlayersChanged,
    /// Some player's metadata or playback state changed.
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
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    #[name = "cover"]
                    gtk::Image {
                        add_css_class: "media-player-bar-cover",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_pixel_size: 20,
                    },

                    #[name = "label"]
                    gtk::Label {
                        add_css_class: "media-player-bar-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_ellipsize: pango::EllipsizeMode::End,
                        set_max_width_chars: 40,
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
            || MediaPlayerCommandOutput::PlayersChanged,
            || MediaPlayerCommandOutput::PlayersChanged,
        );

        let mut model = MediaPlayerModel {
            watcher_token: WatcherToken::new(),
            has_player: false,
            playing: false,
            title: String::new(),
            artist: String::new(),
            cover_art: None,
        };

        subscribe_players(&sender, &mut model.watcher_token);

        let widgets = view_output!();

        // Right click → play/pause the displayed player in place.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            toggle_sender.input(MediaPlayerInput::PlayPauseClicked);
        });
        widgets.root.add_controller(gesture);

        read_display(&mut model);
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
            MediaPlayerInput::PlayPauseClicked => {
                if let Some(player) = display_player() {
                    tokio::spawn(async move {
                        let _ = player.play_pause().await;
                    });
                }
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
            MediaPlayerCommandOutput::PlayersChanged => {
                subscribe_players(&sender, &mut self.watcher_token);
                read_display(self);
            }
            MediaPlayerCommandOutput::TrackChanged => {
                read_display(self);
            }
        }
        apply_visual(widgets, self);
    }
}

/// The player to mirror: the first one actually *playing*, else
/// wayle's `active_player`, else the first in the list.
fn display_player() -> Option<Arc<Player>> {
    let svc = media_service();
    let players = svc.player_list.get();
    players
        .iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .cloned()
        .or_else(|| svc.active_player.get())
        .or_else(|| players.first().cloned())
}

/// Watch *every* player's title / artist / cover / playback
/// state under a fresh `WatcherToken` — so the pill reacts the
/// instant any player starts, stops, or changes track.
fn subscribe_players(
    sender: &ComponentSender<MediaPlayerModel>,
    watcher_token: &mut WatcherToken,
) {
    let token = watcher_token.reset();
    for player in media_service().player_list.get() {
        let title = player.metadata.title.clone();
        let artist = player.metadata.artist.clone();
        let cover = player.metadata.cover_art.clone();
        let playback_state = player.playback_state.clone();
        let t = token.clone();
        watch_cancellable!(
            sender,
            t,
            [
                title.watch(),
                artist.watch(),
                cover.watch(),
                playback_state.watch(),
            ],
            |out| {
                let _ = out.send(MediaPlayerCommandOutput::TrackChanged);
            }
        );
    }
}

/// Refresh the model from whichever player is currently displayed.
fn read_display(model: &mut MediaPlayerModel) {
    match display_player() {
        Some(player) => {
            model.has_player = true;
            model.playing = player.playback_state.get() == PlaybackState::Playing;
            model.title = player.metadata.title.get();
            model.artist = player.metadata.artist.get();
            model.cover_art = player.metadata.cover_art.get();
        }
        None => {
            model.has_player = false;
            model.playing = false;
            model.title.clear();
            model.artist.clear();
            model.cover_art = None;
        }
    }
}

fn apply_visual(widgets: &MediaPlayerModelWidgets, model: &MediaPlayerModel) {
    if !model.has_player {
        widgets.cover.set_icon_name(Some("media-play-symbolic"));
        widgets.label.set_visible(false);
        widgets.root.remove_css_class("paused");
        widgets.root.set_tooltip_text(Some("No media playing"));
        return;
    }

    // Album cover when the art cache resolved one, else a media
    // glyph that doubles as the play/pause state.
    match model.cover_art.as_deref() {
        Some(path) if !path.trim().is_empty() => {
            widgets.cover.set_from_file(Some(path));
        }
        _ => {
            widgets.cover.set_icon_name(Some(if model.playing {
                "media-play-symbolic"
            } else {
                "media-pause-symbolic"
            }));
        }
    }

    // `track — artist`, the song leading. Falls back gracefully
    // when a field is missing.
    let title = model.title.trim();
    let artist = model.artist.trim();
    let text = match (title.is_empty(), artist.is_empty()) {
        (false, false) => format!("{title} — {artist}"),
        (false, true) => title.to_string(),
        (true, false) => artist.to_string(),
        (true, true) => "Media".to_string(),
    };
    widgets.label.set_label(&text);
    widgets.label.set_visible(true);

    // Paused → dim the pill (CSS handles the actual opacity).
    if model.playing {
        widgets.root.remove_css_class("paused");
    } else {
        widgets.root.add_css_class("paused");
    }

    widgets.root.set_tooltip_text(Some(&format!(
        "{}  ·  {}",
        if model.playing { "Playing" } else { "Paused" },
        text
    )));
}
