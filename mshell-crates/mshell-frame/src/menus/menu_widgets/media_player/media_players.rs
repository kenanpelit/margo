//! Multi-player container for the media menu.
//!
//! Holds one `MediaPlayerModel` per MPRIS player in a `gtk::Stack`
//! and shows the *display player* — the one actually playing
//! (Spotify, mpd, a browser tab, …), falling back to wayle's
//! `active_player`, then the first one. wayle only re-selects
//! `active_player` on player add/remove, so every player's
//! `playback_state` is watched here under a `WatcherToken` and
//! the visible child is recomputed whenever playback moves.
//!
//! Players whose state is `Stopped` are treated as idle and
//! excluded from the prev/next switcher + the default selection
//! — that keeps a browser that merely *registered* an MPRIS
//! interface (but isn't playing anything) out of the menu.

use crate::menus::menu_widgets::media_player::media_player::{MediaPlayerInit, MediaPlayerModel};
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

pub(crate) struct MediaPlayersModel {
    player_controllers: Vec<Controller<MediaPlayerModel>>,
    watcher_token: WatcherToken,
    active_player_name: String,
    previous_button_sensitive: bool,
    next_button_sensitive: bool,
    players_visible: bool,
}

#[derive(Debug)]
pub(crate) enum MediaPlayersInput {
    PreviousClicked,
    NextClicked,
    UpdateState,
}

#[derive(Debug)]
pub(crate) enum MediaPlayersOutput {}

pub(crate) struct MediaPlayersInit {}

#[derive(Debug)]
pub(crate) enum MediaPlayersCommandOutput {
    PlayersChanged,
    ActivePlayerChanged,
    /// Some player's playback state changed — re-pick the
    /// display player.
    PlaybackChanged,
}

#[relm4::component(pub)]
impl Component for MediaPlayersModel {
    type CommandOutput = MediaPlayersCommandOutput;
    type Input = MediaPlayersInput;
    type Output = MediaPlayersOutput;
    type Init = MediaPlayersInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            #[watch]
            set_visible: model.players_visible,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                gtk::Label {
                    add_css_class: "label-small-bold-variant",
                    #[watch]
                    set_label: model.active_player_name.as_str(),
                    set_hexpand: true,
                    set_xalign: 0.0
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.previous_button_sensitive,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayersInput::PreviousClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("menu-left-symbolic"),
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.next_button_sensitive,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayersInput::NextClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("menu-right-symbolic"),
                    },
                },
            },

            #[name = "player_container"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 200,
                set_hexpand: true,
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
            || MediaPlayersCommandOutput::PlayersChanged,
            || MediaPlayersCommandOutput::ActivePlayerChanged,
        );

        let players = media_service().player_list.get();

        let mut model = MediaPlayersModel {
            player_controllers: Vec::new(),
            watcher_token: WatcherToken::new(),
            active_player_name: "".to_string(),
            previous_button_sensitive: false,
            next_button_sensitive: false,
            players_visible: !players.is_empty(),
        };

        subscribe_playback(&sender, &mut model.watcher_token);

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MediaPlayersInput::PreviousClicked => {
                let service = media_service();
                let visible = visible_players();
                if let Some(current) = display_player()
                    && let Some(idx) = visible.iter().position(|p| p.id == current.id)
                    && idx > 0
                {
                    let prev_id = visible[idx - 1].id.clone();
                    tokio::spawn(async move {
                        let _ = service.set_active_player(Some(prev_id)).await;
                    });
                }
            }
            MediaPlayersInput::NextClicked => {
                let service = media_service();
                let visible = visible_players();
                if let Some(current) = display_player()
                    && let Some(idx) = visible.iter().position(|p| p.id == current.id)
                    && idx + 1 < visible.len()
                {
                    let next_id = visible[idx + 1].id.clone();
                    tokio::spawn(async move {
                        let _ = service.set_active_player(Some(next_id)).await;
                    });
                }
            }
            MediaPlayersInput::UpdateState => {
                let visible = visible_players();
                self.players_visible = !visible.is_empty();

                let display = display_player();
                if let Some(display) = &display {
                    self.active_player_name = display.identity.get();
                    if let Some(idx) = visible.iter().position(|p| p.id == display.id) {
                        self.previous_button_sensitive = idx > 0;
                        self.next_button_sensitive = idx + 1 < visible.len();
                    } else {
                        self.previous_button_sensitive = false;
                        self.next_button_sensitive = false;
                    }
                } else {
                    self.active_player_name.clear();
                    self.previous_button_sensitive = false;
                    self.next_button_sensitive = false;
                }

                // Reveal the display player, hide the rest.
                let display_id = display.as_ref().map(|p| &p.id);
                for controller in &self.player_controllers {
                    if Some(&controller.model().player.id) == display_id {
                        widgets
                            .player_container
                            .set_visible_child(controller.widget());
                    }
                }
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MediaPlayersCommandOutput::PlayersChanged => {
                let service = media_service();
                let players = service.player_list.get();

                // Re-arm the per-player playback watchers for the
                // new player set.
                subscribe_playback(&sender, &mut self.watcher_token);

                // Remove controllers for players no longer present.
                self.player_controllers.retain(|controller| {
                    let still_exists = players.iter().any(|p| p.id == controller.model().player.id);
                    if !still_exists {
                        widgets.player_container.remove(controller.widget());
                    }
                    still_exists
                });

                // Add controllers for new players.
                for player in &players {
                    let already_exists = self
                        .player_controllers
                        .iter()
                        .any(|c| c.model().player.id == player.id);

                    if !already_exists {
                        let player_clone = player.clone();
                        let controller = MediaPlayerModel::builder()
                            .launch(MediaPlayerInit {
                                player: player_clone,
                            })
                            .detach();
                        widgets.player_container.add_child(controller.widget());
                        self.player_controllers.push(controller);
                    }
                }

                sender.input(MediaPlayersInput::UpdateState);
            }
            MediaPlayersCommandOutput::ActivePlayerChanged
            | MediaPlayersCommandOutput::PlaybackChanged => {
                sender.input(MediaPlayersInput::UpdateState);
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Players worth showing — anything not `Stopped`. A browser that
/// merely registered an MPRIS interface without playing reports
/// `Stopped`, so this drops it from the switcher.
fn visible_players() -> Vec<Arc<Player>> {
    media_service()
        .player_list
        .get()
        .into_iter()
        .filter(|p| p.playback_state.get() != PlaybackState::Stopped)
        .collect()
}

/// The player to show by default: the first one actually playing,
/// else wayle's `active_player` if it's still a visible player,
/// else the first visible player.
fn display_player() -> Option<Arc<Player>> {
    let visible = visible_players();
    visible
        .iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .cloned()
        .or_else(|| {
            media_service()
                .active_player
                .get()
                .filter(|ap| visible.iter().any(|p| p.id == ap.id))
        })
        .or_else(|| visible.first().cloned())
}

/// Watch every player's `playback_state` under a fresh
/// `WatcherToken` so the display player is recomputed the instant
/// playback starts/stops anywhere.
fn subscribe_playback(
    sender: &ComponentSender<MediaPlayersModel>,
    watcher_token: &mut WatcherToken,
) {
    let token = watcher_token.reset();
    for player in media_service().player_list.get() {
        let playback_state = player.playback_state.clone();
        let t = token.clone();
        watch_cancellable!(sender, t, [playback_state.watch()], |out| {
            let _ = out.send(MediaPlayersCommandOutput::PlaybackChanged);
        });
    }
}
