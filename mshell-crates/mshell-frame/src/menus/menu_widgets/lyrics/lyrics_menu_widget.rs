//! Lyrics menu — the full synced-lyrics panel the lyrics pill (and
//! `mshellctl menu lyrics`) opens.
//!
//! Renders the now-playing track's lyrics as a scrolling column, the active
//! line lit (`--primary`) and auto-scrolled to the centre as the song plays.
//! Falls back to a status line for the empty states (nothing playing, loading,
//! no lyrics) and to a plain non-highlighted column for unsynced lyrics.
//!
//! Like every per-output menu it's built eagerly, so the network fetch +
//! position tracking start **lazily** on first reveal (`ParentRevealChanged`)
//! and stop on hide — only a song you actually open the panel for is fetched.

use crate::lyrics::{self, Lyrics, TrackKey};
use futures::StreamExt;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use relm4::gtk::prelude::*;
use relm4::gtk::{glib, pango};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

pub(crate) struct LyricsMenuWidgetModel {
    /// Watches every player's metadata + playback while the panel is open.
    players_token: WatcherToken,
    /// Follows the display player's position; reset on player switch / hide.
    position_token: WatcherToken,
    position_player_id: Option<String>,
    revealed: bool,
    has_player: bool,
    title: String,
    artist: String,
    key: Option<TrackKey>,
    lyrics: Lyrics,
    loading: bool,
    position_ms: u64,
    active_idx: Option<usize>,
    /// One label per *synced* line, in order — rebuilt when lyrics change,
    /// re-styled (active tint) as the position advances.
    line_labels: Vec<gtk::Label>,
}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetInput {
    /// Menu reveal toggled — start the fetch + position watch on show, stop on
    /// hide.
    ParentRevealChanged(bool),
    /// Refresh button — re-fetch the current track's lyrics, bypassing cache.
    Refresh,
}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetOutput {}

pub(crate) struct LyricsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetCommandOutput {
    PlayersChanged,
    TrackChanged,
    Position(Duration),
    Fetched(TrackKey, Lyrics),
}

#[relm4::component(pub)]
impl Component for LyricsMenuWidgetModel {
    type CommandOutput = LyricsMenuWidgetCommandOutput;
    type Input = LyricsMenuWidgetInput;
    type Output = LyricsMenuWidgetOutput;
    type Init = LyricsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "lyrics-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            set_hexpand: true,
            set_vexpand: true,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("lyrics-symbolic"),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        add_css_class: "panel-title",
                        #[watch]
                        set_label: if model.title.is_empty() { "Lyrics" } else { &model.title },
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },

                    gtk::Label {
                        add_css_class: "lyrics-menu-artist",
                        #[watch]
                        set_label: &model.artist,
                        #[watch]
                        set_visible: !model.artist.trim().is_empty(),
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },
                },

                // Manual re-fetch — bypasses the cache and re-hits lrclib.
                gtk::Button {
                    add_css_class: "lyrics-refresh",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Re-fetch lyrics"),
                    #[watch]
                    set_visible: model.has_player,
                    set_icon_name: "view-refresh-symbolic",
                    connect_clicked => LyricsMenuWidgetInput::Refresh,
                },
            },

            // Terse source/state badge — visible whenever a player is present,
            // mirrors the musiclyrics panel ("Synced · lrclib.net", …).
            gtk::Label {
                #[watch]
                set_css_classes: &model.badge_classes(),
                set_halign: gtk::Align::Center,
                #[watch]
                set_label: model.badge_text(),
                #[watch]
                set_visible: !model.badge_text().is_empty(),
            },

            gtk::Separator {
                set_orientation: gtk::Orientation::Horizontal,
                #[watch]
                set_visible: model.has_player,
            },

            #[name = "status"]
            gtk::Label {
                add_css_class: "lyrics-status",
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_vexpand: true,
                #[watch]
                set_label: model.status_message().unwrap_or(""),
                #[watch]
                set_visible: model.status_message().is_some(),
            },

            #[name = "scroller"]
            gtk::ScrolledWindow {
                add_css_class: "lyrics-scroller",
                set_hexpand: true,
                set_vexpand: true,
                set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),
                #[watch]
                set_visible: model.status_message().is_none(),

                #[name = "lines_box"]
                gtk::Box {
                    add_css_class: "lyrics-lines",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Permanent, lightweight: react to players being added / removed. The
        // heavy work (metadata watch, position stream, fetch) is gated behind
        // `revealed`, so it only runs while the panel is open.
        spawn_media_players_watcher(
            &sender,
            || LyricsMenuWidgetCommandOutput::PlayersChanged,
            || LyricsMenuWidgetCommandOutput::PlayersChanged,
        );

        let model = LyricsMenuWidgetModel {
            players_token: WatcherToken::new(),
            position_token: WatcherToken::new(),
            position_player_id: None,
            revealed: false,
            has_player: false,
            title: String::new(),
            artist: String::new(),
            key: None,
            lyrics: Lyrics::None,
            loading: false,
            position_ms: 0,
            active_idx: None,
            line_labels: Vec::new(),
        };

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
            LyricsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.revealed = visible;
                if visible {
                    subscribe_players(&sender, &mut self.players_token);
                    self.pick_track(&sender);
                    self.rebuild_lines(widgets);
                } else {
                    // Closed: stop the watchers so a hidden panel costs nothing.
                    self.players_token.reset();
                    self.position_token.reset();
                    self.position_player_id = None;
                }
            }
            LyricsMenuWidgetInput::Refresh => {
                if self.revealed
                    && self.has_player
                    && let Some(key) = self.key.clone().filter(TrackKey::is_valid)
                {
                    self.lyrics = Lyrics::None;
                    self.active_idx = None;
                    self.loading = true;
                    kick_refetch(&sender, key);
                    self.rebuild_lines(widgets);
                }
            }
        }
        self.apply_active(widgets);
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
            LyricsMenuWidgetCommandOutput::PlayersChanged => {
                if self.revealed {
                    subscribe_players(&sender, &mut self.players_token);
                    self.pick_track(&sender);
                    self.rebuild_lines(widgets);
                }
            }
            LyricsMenuWidgetCommandOutput::TrackChanged => {
                if self.revealed {
                    self.pick_track(&sender);
                    self.rebuild_lines(widgets);
                }
            }
            LyricsMenuWidgetCommandOutput::Position(pos) => {
                self.position_ms = pos.as_millis() as u64;
                let new_idx = self.compute_active();
                if new_idx == self.active_idx {
                    return;
                }
                self.active_idx = new_idx;
            }
            LyricsMenuWidgetCommandOutput::Fetched(key, lyrics) => {
                if self.key.as_ref() != Some(&key) {
                    return;
                }
                self.lyrics = lyrics;
                self.loading = false;
                self.active_idx = self.compute_active();
                self.rebuild_lines(widgets);
            }
        }
        self.apply_active(widgets);
        self.update_view(widgets, sender);
    }
}

impl LyricsMenuWidgetModel {
    /// Verbose body message for the non-list states; `None` once there are
    /// lines to show. Mirrors the musiclyrics panel's centred message.
    fn status_message(&self) -> Option<&'static str> {
        if !self.has_player {
            return Some("Nothing playing right now.");
        }
        if self.loading {
            return Some("Searching lyrics…");
        }
        match &self.lyrics {
            Lyrics::Instrumental => Some("This track is instrumental."),
            Lyrics::None => Some("No lyrics found for this track."),
            l if l.is_empty() => Some("No lyrics found for this track."),
            _ => None,
        }
    }

    /// Terse source/state badge text, or `""` when no badge should show (no
    /// player). Matches the musiclyrics panel's status pill.
    fn badge_text(&self) -> &'static str {
        if !self.has_player {
            return "";
        }
        if self.loading {
            return "Searching lyrics…";
        }
        match &self.lyrics {
            Lyrics::Synced(_) => "Synced · lrclib.net",
            Lyrics::Plain(_) => "Unsynced · lrclib.net",
            Lyrics::Instrumental => "Instrumental",
            Lyrics::None => "No lyrics found",
        }
    }

    /// CSS classes for the badge — the `-ok` accent variant when we actually
    /// have lyrics to show.
    fn badge_classes(&self) -> Vec<&'static str> {
        let mut classes = vec!["lyrics-status-badge"];
        if !self.loading && matches!(self.lyrics, Lyrics::Synced(_) | Lyrics::Plain(_)) {
            classes.push("lyrics-status-badge-ok");
        }
        classes
    }

    fn compute_active(&self) -> Option<usize> {
        match &self.lyrics {
            Lyrics::Synced(lines) => lyrics::index_for_time(lines, self.position_ms),
            _ => None,
        }
    }

    /// Re-pick the display player, follow its position, refresh title/artist,
    /// and kick a fetch when the track key changed.
    fn pick_track(&mut self, sender: &ComponentSender<Self>) {
        let Some(player) = display_player() else {
            self.has_player = false;
            self.title.clear();
            self.artist.clear();
            self.key = None;
            self.lyrics = Lyrics::None;
            self.loading = false;
            self.active_idx = None;
            self.position_player_id = None;
            self.position_token.reset();
            return;
        };

        self.has_player = true;
        self.title = player.metadata.title.get();
        self.artist = player.metadata.artist.get();

        let id = player.id.to_string();
        if self.position_player_id.as_deref() != Some(id.as_str()) {
            let token = self.position_token.reset();
            subscribe_position(player.clone(), sender, token);
            self.position_player_id = Some(id);
            self.position_ms = 0;
        }

        let key = track_key(&player);
        if self.key.as_ref() == Some(&key) {
            return;
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

    /// Rebuild the scrolling column from the current lyrics. Synced lines are
    /// tracked in `line_labels` for active-tinting; plain lines are static.
    fn rebuild_lines(&mut self, widgets: &LyricsMenuWidgetModelWidgets) {
        while let Some(child) = widgets.lines_box.first_child() {
            widgets.lines_box.remove(&child);
        }
        self.line_labels.clear();

        match &self.lyrics {
            Lyrics::Synced(lines) => {
                for line in lines {
                    let label = make_line_label(&line.text, false);
                    widgets.lines_box.append(&label);
                    self.line_labels.push(label);
                }
            }
            Lyrics::Plain(lines) => {
                for text in lines {
                    let label = make_line_label(text, true);
                    widgets.lines_box.append(&label);
                }
            }
            Lyrics::Instrumental | Lyrics::None => {}
        }
    }

    /// Light the active synced line and scroll it to the centre.
    fn apply_active(&self, widgets: &LyricsMenuWidgetModelWidgets) {
        for (i, label) in self.line_labels.iter().enumerate() {
            if Some(i) == self.active_idx {
                label.add_css_class("lyrics-line-active");
            } else {
                label.remove_css_class("lyrics-line-active");
            }
        }
        if let Some(label) = self.active_idx.and_then(|i| self.line_labels.get(i)) {
            scroll_center(&widgets.scroller, label);
        }
    }
}

fn make_line_label(text: &str, plain: bool) -> gtk::Label {
    let shown = if text.trim().is_empty() { "♪" } else { text };
    let mut classes = vec!["lyrics-line"];
    if plain {
        classes.push("lyrics-line-plain");
    }
    gtk::Label::builder()
        .label(shown)
        .css_classes(classes)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .build()
}

/// Smoothly centre `label` in `scroller`. Deferred to idle so the label's
/// geometry is laid out (it's freshly appended on a rebuild).
fn scroll_center(scroller: &gtk::ScrolledWindow, label: &gtk::Label) {
    let scroller = scroller.clone();
    let label = label.clone();
    glib::idle_add_local_once(move || {
        // Position of the label within the scrolled content (its parent box) —
        // the same coordinate space as the vertical adjustment.
        let Some(parent) = label.parent() else { return };
        let Some(bounds) = label.compute_bounds(&parent) else {
            return;
        };
        if bounds.height() == 0.0 {
            return;
        }
        let vadj = scroller.vadjustment();
        let center = bounds.y() as f64 + bounds.height() as f64 / 2.0;
        let target = center - vadj.page_size() / 2.0;
        let max = (vadj.upper() - vadj.page_size()).max(0.0);
        vadj.set_value(target.clamp(0.0, max));
    });
}

fn track_key(player: &Player) -> TrackKey {
    lyrics::key_for(
        &player.metadata.title.get(),
        &player.metadata.artist.get(),
        &player.metadata.album.get(),
        player
            .metadata
            .length
            .get()
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// The player to mirror: first actually playing, else active, else first.
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

fn subscribe_players(sender: &ComponentSender<LyricsMenuWidgetModel>, token: &mut WatcherToken) {
    let token = token.reset();
    for player in media_service().player_list.get() {
        let metadata = player.metadata.clone();
        let playback = player.playback_state.clone();
        let t = token.clone();
        watch_cancellable!(sender, t, [metadata.watch(), playback.watch()], |out| {
            let _ = out.send(LyricsMenuWidgetCommandOutput::TrackChanged);
        });
    }
}

fn subscribe_position(
    player: Arc<Player>,
    sender: &ComponentSender<LyricsMenuWidgetModel>,
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
                    let _ = out.send(LyricsMenuWidgetCommandOutput::Position(pos));
                }
                else => break,
            }
        }
    });
}

fn kick_fetch(sender: &ComponentSender<LyricsMenuWidgetModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let lyrics = tokio::task::spawn_blocking(move || lyrics::fetch(&for_fetch))
            .await
            .unwrap_or(Lyrics::None);
        let _ = out.send(LyricsMenuWidgetCommandOutput::Fetched(key, lyrics));
    });
}

/// Like [`kick_fetch`] but bypasses the disk cache (manual refresh).
fn kick_refetch(sender: &ComponentSender<LyricsMenuWidgetModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let lyrics = tokio::task::spawn_blocking(move || lyrics::refetch(&for_fetch))
            .await
            .unwrap_or(Lyrics::None);
        let _ = out.send(LyricsMenuWidgetCommandOutput::Fetched(key, lyrics));
    });
}
