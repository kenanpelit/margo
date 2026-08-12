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

use crate::menus::menu_widgets::media_player::media_player::{
    MediaPlayerInit, MediaPlayerInput, MediaPlayerModel,
};
use crate::menus::menu_widgets::media_player::mpd_player::{MpdPlayerInit, MpdPlayerModel};
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_services::mpd::mpd_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use wayle_media::types::PlaybackState;

/// One switcher slot — either an MPRIS player (identified by wayle's
/// player id) or the single native MPD player. MPD has no equivalent of
/// MPRIS players coming and going with app lifecycle, so unlike
/// `player_controllers` its `Controller` is created once in `init` and
/// never added to / removed from the `Stack` — only whether it's
/// *selected* (via [`visible_slots`]'s `Stopped` filter, same rule
/// `visible_players` already applies to MPRIS) changes.
#[derive(Clone, PartialEq, Eq)]
enum DisplaySlot {
    Mpris(wayle_media::types::PlayerId),
    Mpd,
}

pub(crate) struct MediaPlayersModel {
    player_controllers: Vec<Controller<MediaPlayerModel>>,
    mpd_controller: Controller<MpdPlayerModel>,
    watcher_token: WatcherToken,
    active_player_name: String,
    previous_button_sensitive: bool,
    next_button_sensitive: bool,
    players_visible: bool,
    /// Whether the media menu is currently revealed. New child players
    /// added while open inherit this so their marquees start straight
    /// away instead of waiting for the next reveal toggle.
    revealed: bool,
    /// Explicit prev/next selection, overriding the "first Playing"
    /// default. Mirrors wayle's `active_player`, but spans both
    /// backends — wayle's own `active_player` only knows about MPRIS
    /// players, so a manual selection landing on the MPD slot has
    /// nowhere else to live.
    manual_selection: Option<DisplaySlot>,
}

#[derive(Debug)]
pub(crate) enum MediaPlayersInput {
    PreviousClicked,
    NextClicked,
    UpdateState,
    /// Menu reveal state changed — forwarded to every child player so
    /// their marquee timers stop while the media menu is closed.
    ParentRevealChanged(bool),
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

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("media-play-symbolic"),
                },

                gtk::Label {
                    add_css_class: "panel-title",
                    #[watch]
                    set_label: model.active_player_name.as_str(),
                    set_hexpand: true,
                    set_xalign: 0.0,
                    // Truncate long player names so the title doesn't
                    // demand width and push the dashboard's right column
                    // past its homogeneous slot (was breaking the
                    // equal-column rule, §7).
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
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
                // Fill the media root's height. The dashboard's right column
                // gives this widget vexpand+Fill (last child, `fill: true`);
                // without the Stack also expanding, the player card sat at
                // natural height with empty space below it — the card read as
                // "too high" and the right column ended above the left.
                set_vexpand: true,
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
        let mpd_playing = mpd_service().player.playback_state.get() != PlaybackState::Stopped;

        // Created once and never removed from the Stack — see the
        // `DisplaySlot` doc comment above for why MPD doesn't need the
        // MPRIS players' dynamic add/remove dance.
        let mpd_controller = MpdPlayerModel::builder()
            .launch(MpdPlayerInit {
                player: mpd_service().player.clone(),
            })
            .detach();

        let mut model = MediaPlayersModel {
            player_controllers: Vec::new(),
            mpd_controller,
            watcher_token: WatcherToken::new(),
            active_player_name: "".to_string(),
            previous_button_sensitive: false,
            next_button_sensitive: false,
            players_visible: !players.is_empty() || mpd_playing,
            revealed: false,
            manual_selection: None,
        };

        subscribe_playback(&sender, &mut model.watcher_token);

        let widgets = view_output!();
        widgets
            .player_container
            .add_child(model.mpd_controller.widget());

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
                let visible = visible_slots();
                if let Some(current) = display_slot(self)
                    && let Some(idx) = visible.iter().position(|s| *s == current)
                    && idx > 0
                {
                    select_slot(self, visible[idx - 1].clone());
                }
            }
            MediaPlayersInput::NextClicked => {
                let visible = visible_slots();
                if let Some(current) = display_slot(self)
                    && let Some(idx) = visible.iter().position(|s| *s == current)
                    && idx + 1 < visible.len()
                {
                    select_slot(self, visible[idx + 1].clone());
                }
            }
            MediaPlayersInput::UpdateState => {
                let visible = visible_slots();
                self.players_visible = !visible.is_empty();

                let display = display_slot(self);
                if let Some(display) = &display {
                    self.active_player_name = match display {
                        DisplaySlot::Mpris(id) => media_service()
                            .player_list
                            .get()
                            .iter()
                            .find(|p| p.id == *id)
                            .map(|p| p.identity.get())
                            .unwrap_or_default(),
                        DisplaySlot::Mpd => "MPD".to_string(),
                    };
                    if let Some(idx) = visible.iter().position(|s| s == display) {
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

                // Reveal the display slot, hide the rest.
                match &display {
                    Some(DisplaySlot::Mpris(id)) => {
                        for controller in &self.player_controllers {
                            if controller.model().player.id == *id {
                                widgets
                                    .player_container
                                    .set_visible_child(controller.widget());
                            }
                        }
                    }
                    Some(DisplaySlot::Mpd) => {
                        widgets
                            .player_container
                            .set_visible_child(self.mpd_controller.widget());
                    }
                    None => {}
                }
            }
            MediaPlayersInput::ParentRevealChanged(visible) => {
                self.revealed = visible;
                for controller in &self.player_controllers {
                    controller
                        .sender()
                        .send(MediaPlayerInput::ParentRevealChanged(visible))
                        .ok();
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
                        // A player added while the menu is already open must
                        // start its marquee now — the reveal broadcast won't
                        // fire again until the next open/close.
                        if self.revealed {
                            controller
                                .sender()
                                .send(MediaPlayerInput::ParentRevealChanged(true))
                                .ok();
                        }
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

/// Slots worth showing — anything not `Stopped`. A browser that merely
/// registered an MPRIS interface without playing reports `Stopped`, so
/// this drops it from the switcher; the same rule applies to the MPD
/// slot so an idle/disconnected MPD doesn't clutter the switcher either.
fn visible_slots() -> Vec<DisplaySlot> {
    let mut slots: Vec<DisplaySlot> = media_service()
        .player_list
        .get()
        .into_iter()
        .filter(|p| p.playback_state.get() != PlaybackState::Stopped)
        .map(|p| DisplaySlot::Mpris(p.id.clone()))
        .collect();
    if mpd_service().player.playback_state.get() != PlaybackState::Stopped {
        slots.push(DisplaySlot::Mpd);
    }
    slots
}

/// The slot to show by default: the first one actually playing (MPRIS
/// checked before MPD — arbitrary but stable tie-break), else the
/// model's own `manual_selection` if it's still visible, else the first
/// visible slot.
fn display_slot(model: &MediaPlayersModel) -> Option<DisplaySlot> {
    let visible = visible_slots();
    media_service()
        .player_list
        .get()
        .into_iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .map(|p| DisplaySlot::Mpris(p.id.clone()))
        .or_else(|| {
            (mpd_service().player.playback_state.get() == PlaybackState::Playing)
                .then_some(DisplaySlot::Mpd)
        })
        .or_else(|| {
            model
                .manual_selection
                .clone()
                .filter(|sel| visible.contains(sel))
        })
        .or_else(|| visible.first().cloned())
}

/// Record an explicit prev/next selection (see `manual_selection`'s doc
/// comment) and, for an MPRIS slot, also tell wayle — the bar pill's own
/// `display_player` falls back to wayle's `active_player`, so without
/// this a menu prev/next click would stop influencing what the pill
/// shows once nothing is actively `Playing`.
fn select_slot(model: &mut MediaPlayersModel, slot: DisplaySlot) {
    if let DisplaySlot::Mpris(id) = &slot {
        let service = media_service();
        let id = id.clone();
        tokio::spawn(async move {
            let _ = service.set_active_player(Some(id)).await;
        });
    }
    model.manual_selection = Some(slot);
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
    let mpd_playback_state = mpd_service().player.playback_state.clone();
    watch_cancellable!(sender, token, [mpd_playback_state.watch()], |out| {
        let _ = out.send(MediaPlayersCommandOutput::PlaybackChanged);
    });
}
