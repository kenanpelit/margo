//! Lyrics bar pill.
//!
//! Shows the current *synced* line of the now-playing track, scrolling in the
//! bar as the song plays — a lightweight indicator that doubles as the opener
//! for the full lyrics menu (left click → `MenuType::Lyrics`).
//!
//! It mirrors the media pill's player tracking (follow whichever MPRIS player
//! is actually playing, across players), then layers on two things the media
//! pill doesn't need: it fetches lyrics for the current track off-thread
//! ([`crate::lyrics::fetch`] via `spawn_blocking`, disk-cached) and watches the
//! display player's playback *position* to light up the active line.
//!
//! Fetching only happens when this pill (or the lyrics menu) is actually in
//! use, so a user who never adds the widget never hits lrclib.

use crate::lyrics::{self, Lyrics, TrackKey};
use futures::StreamExt;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

pub(crate) struct LyricsModel {
    /// Watches every player's metadata + playback state — re-pick the display
    /// player / refresh the track key whenever playback moves.
    players_token: WatcherToken,
    /// Follows the *display* player's position only; reset when it changes.
    position_token: WatcherToken,
    /// `PlayerId` (as string) the position watcher currently follows.
    position_player_id: Option<String>,
    has_player: bool,
    key: Option<TrackKey>,
    lyrics: Lyrics,
    loading: bool,
    position_ms: u64,
    active_idx: Option<usize>,
}

#[derive(Debug)]
pub(crate) enum LyricsInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum LyricsOutput {
    Clicked,
}

pub(crate) struct LyricsInit {}

#[derive(Debug)]
pub(crate) enum LyricsCommandOutput {
    /// Player list / active player changed — re-subscribe + re-pick.
    PlayersChanged,
    /// Some player's metadata or playback state changed.
    TrackChanged,
    /// Display player's position advanced.
    Position(Duration),
    /// Background lyrics fetch finished (for the given track key).
    Fetched(TrackKey, Lyrics),
}

#[relm4::component(pub)]
impl Component for LyricsModel {
    type CommandOutput = LyricsCommandOutput;
    type Input = LyricsInput;
    type Output = LyricsOutput;
    type Init = LyricsInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["lyrics-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(LyricsInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        add_css_class: "lyrics-bar-icon",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("media-view-subtitles-symbolic"),
                    },

                    #[name = "label"]
                    gtk::Label {
                        add_css_class: "lyrics-bar-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_ellipsize: pango::EllipsizeMode::End,
                        set_max_width_chars: 32,
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
            || LyricsCommandOutput::PlayersChanged,
            || LyricsCommandOutput::PlayersChanged,
        );

        let mut model = LyricsModel {
            players_token: WatcherToken::new(),
            position_token: WatcherToken::new(),
            position_player_id: None,
            has_player: false,
            key: None,
            lyrics: Lyrics::None,
            loading: false,
            position_ms: 0,
            active_idx: None,
        };

        subscribe_players(&sender, &mut model.players_token);

        let widgets = view_output!();

        model.refresh_track(&sender);
        apply_visual(&widgets, &model);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LyricsInput::Clicked => {
                let _ = sender.output(LyricsOutput::Clicked);
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
            LyricsCommandOutput::PlayersChanged => {
                subscribe_players(&sender, &mut self.players_token);
                self.refresh_track(&sender);
            }
            LyricsCommandOutput::TrackChanged => {
                self.refresh_track(&sender);
            }
            LyricsCommandOutput::Position(pos) => {
                self.position_ms = pos.as_millis() as u64;
                let new_idx = self.compute_active();
                // Only repaint when the active line actually moves — position
                // ticks land roughly once a second.
                if new_idx == self.active_idx {
                    return;
                }
                self.active_idx = new_idx;
            }
            LyricsCommandOutput::Fetched(key, lyrics) => {
                if self.key.as_ref() != Some(&key) {
                    // The track moved on while we were fetching — stale result.
                    return;
                }
                self.lyrics = lyrics;
                self.loading = false;
                self.active_idx = self.compute_active();
            }
        }
        apply_visual(widgets, self);
    }
}

impl LyricsModel {
    /// Active synced-line index for the current position, or `None`.
    fn compute_active(&self) -> Option<usize> {
        match &self.lyrics {
            Lyrics::Synced(lines) => lyrics::index_for_time(lines, self.position_ms),
            _ => None,
        }
    }

    /// Re-pick the display player, follow its position, and kick a lyrics fetch
    /// when the track key changed.
    fn refresh_track(&mut self, sender: &ComponentSender<Self>) {
        let Some(player) = display_player() else {
            self.has_player = false;
            self.key = None;
            self.lyrics = Lyrics::None;
            self.loading = false;
            self.active_idx = None;
            self.position_player_id = None;
            self.position_token.reset();
            return;
        };

        self.has_player = true;

        // Follow this player's position if we switched players.
        let id = player.id.to_string();
        if self.position_player_id.as_deref() != Some(id.as_str()) {
            let token = self.position_token.reset();
            subscribe_position(player.clone(), sender, token);
            self.position_player_id = Some(id);
            self.position_ms = 0;
        }

        let key = TrackKey {
            artist: player.metadata.artist.get(),
            title: player.metadata.title.get(),
            album: player.metadata.album.get(),
            duration_secs: player
                .metadata
                .length
                .get()
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        if self.key.as_ref() == Some(&key) {
            return; // Same track — keep the lyrics we already have.
        }

        self.key = Some(key.clone());
        self.lyrics = Lyrics::None;
        self.active_idx = None;
        if key.is_valid() {
            self.loading = true;
            kick_fetch(sender, key);
        } else {
            self.loading = false;
        }
    }
}

/// The player to mirror: the first one actually *playing*, else wayle's
/// `active_player`, else the first in the list. (Matches the media pill.)
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

/// Watch every player's metadata + playback state under a fresh token so the
/// display player + track key follow playback across players.
fn subscribe_players(sender: &ComponentSender<LyricsModel>, token: &mut WatcherToken) {
    let token = token.reset();
    for player in media_service().player_list.get() {
        let metadata = player.metadata.clone();
        let playback = player.playback_state.clone();
        let t = token.clone();
        watch_cancellable!(sender, t, [metadata.watch(), playback.watch()], |out| {
            let _ = out.send(LyricsCommandOutput::TrackChanged);
        });
    }
}

/// Stream the display player's position into [`LyricsCommandOutput::Position`]
/// until the token is cancelled (player switched) or the component shuts down.
fn subscribe_position(
    player: Arc<Player>,
    sender: &ComponentSender<LyricsModel>,
    token: CancellationToken,
) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut stream = Box::pin(player.position.watch());
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = token.cancelled() => break,
                Some(pos) = stream.next() => {
                    let _ = out.send(LyricsCommandOutput::Position(pos));
                }
                else => break,
            }
        }
    });
}

/// Resolve lyrics for `key` off the main thread, then deliver them tagged with
/// the key so a stale (track-changed) result can be discarded.
fn kick_fetch(sender: &ComponentSender<LyricsModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let lyrics = tokio::task::spawn_blocking(move || lyrics::fetch(&for_fetch))
            .await
            .unwrap_or(Lyrics::None);
        let _ = out.send(LyricsCommandOutput::Fetched(key, lyrics));
    });
}

fn apply_visual(widgets: &LyricsModelWidgets, model: &LyricsModel) {
    let root = &widgets.root;
    root.remove_css_class("has-lyrics");
    root.remove_css_class("dim");

    if !model.has_player {
        widgets.label.set_visible(false);
        root.add_css_class("dim");
        root.set_tooltip_text(Some("No media playing"));
        return;
    }

    if model.loading {
        widgets.label.set_label("…");
        widgets.label.set_visible(true);
        root.set_tooltip_text(Some("Loading lyrics…"));
        return;
    }

    match &model.lyrics {
        Lyrics::Synced(lines) => {
            let text = model
                .active_idx
                .and_then(|i| lines.get(i))
                .map(|l| l.text.as_str())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or("♪");
            widgets.label.set_label(text);
            widgets.label.set_visible(true);
            root.add_css_class("has-lyrics");
            root.set_tooltip_text(Some("Synced lyrics — click for the full panel"));
        }
        Lyrics::Plain(_) => {
            widgets.label.set_visible(false);
            root.add_css_class("has-lyrics");
            root.set_tooltip_text(Some("Lyrics available (unsynced) — click to view"));
        }
        Lyrics::None => {
            widgets.label.set_visible(false);
            root.add_css_class("dim");
            root.set_tooltip_text(Some("No lyrics found"));
        }
    }
}
