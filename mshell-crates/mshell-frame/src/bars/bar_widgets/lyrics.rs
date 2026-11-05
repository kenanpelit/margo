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
//! use, so a user who never adds the widget never hits lrclib. A transient
//! network failure is retried with back-off ([`lyrics::retry_delay`]) while
//! the same track plays. Word-timed lyrics (enhanced LRC / NetEase YRC) fill
//! the bar line word by word: a [`PlayClock`] smooths wayle's once-a-second
//! position samples and a short repaint tick runs only while such a line is
//! showing and the player is playing.

use crate::lyrics::{self, Lyrics, LyricsSource, PlayClock, Resolution, TrackKey, ui};
use futures::StreamExt;
use mshell_common::scoped_effects::EffectScope;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, LyricsBarWidgetStoreFields,
};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use reactive_graph::traits::{Get, GetUntracked, Track};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::gtk::{glib, pango};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    source: LyricsSource,
    loading: bool,
    /// Smoothed playback position of the display player.
    clock: PlayClock,
    /// Global timing offset (ms), live from `bars.widgets.lyrics.offset_ms`.
    offset_ms: i32,
    active_idx: Option<usize>,
    /// Karaoke split (byte offset) last painted into the bar line.
    painted_split: usize,
    /// Back-off retries already spent on the current track.
    retry_attempt: u32,
    /// The fast karaoke repaint tick, when running.
    tick: Option<glib::SourceId>,
    /// Last seen source settings (`netease_fallback`, `local_dir`), to tell a
    /// real change from the effect's first run.
    sources: (bool, String),
    /// Whether to show the current line in the bar (else icon-only; words live
    /// in the menu). Live from `bars.widgets.lyrics.show_line_in_bar`.
    show_line: bool,
    /// Max width of the bar line in characters. Live from
    /// `bars.widgets.lyrics.max_width_chars`, clamped 8..=120.
    max_width_chars: i32,
    _effects: EffectScope,
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
    Fetched(TrackKey, Resolution),
    /// A back-off retry for this track came due.
    Retry(TrackKey),
    /// Karaoke repaint tick.
    Tick,
    /// `offset_ms` config changed.
    OffsetChanged(i32),
    /// `netease_fallback` config changed — the source chain differs, so
    /// resolve the current track again.
    SourcesChanged(bool, String),
    /// `show_line_in_bar` config changed.
    ShowLineChanged(bool),
    /// `max_width_chars` config changed.
    MaxWidthChanged(i32),
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
                        set_icon_name: Some("lyrics-symbolic"),
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

        // Track the "show line in bar" config live so toggling it in Settings
        // updates the pill without a restart.
        let mut effects = EffectScope::new();
        let cs = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .show_line_in_bar()
                .get();
            let _ = cs
                .command_sender()
                .send(LyricsCommandOutput::ShowLineChanged(v));
        });
        let cs = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .max_width_chars()
                .get();
            let _ = cs
                .command_sender()
                .send(LyricsCommandOutput::MaxWidthChanged(clamp_width(v)));
        });

        let cs = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .offset_ms()
                .get();
            let _ = cs
                .command_sender()
                .send(LyricsCommandOutput::OffsetChanged(v));
        });
        let cs = sender.clone();
        effects.push(move |_| {
            let lyr = || config_manager().config().bars().widgets().lyrics();
            let _ = cs
                .command_sender()
                .send(LyricsCommandOutput::SourcesChanged(
                    lyr().netease_fallback().get(),
                    lyr().local_dir().get(),
                ));
        });
        // The player allow/block lists pick which player drives the pill.
        let cs = sender.clone();
        effects.push(move |_| {
            let lyr = || config_manager().config().bars().widgets().lyrics();
            lyr().player_allowlist().track();
            lyr().player_blocklist().track();
            let _ = cs
                .command_sender()
                .send(LyricsCommandOutput::PlayersChanged);
        });

        let mut model = LyricsModel {
            players_token: WatcherToken::new(),
            position_token: WatcherToken::new(),
            position_player_id: None,
            has_player: false,
            key: None,
            lyrics: Lyrics::None,
            source: LyricsSource::Lrclib,
            loading: false,
            clock: PlayClock::new(Instant::now()),
            offset_ms: config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .offset_ms()
                .get_untracked(),
            active_idx: None,
            painted_split: 0,
            retry_attempt: 0,
            tick: None,
            sources: current_sources(),
            show_line: config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .show_line_in_bar()
                .get_untracked(),
            max_width_chars: clamp_width(
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .lyrics()
                    .max_width_chars()
                    .get_untracked(),
            ),
            _effects: effects,
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
                self.clock.sample(pos.as_millis() as u64, Instant::now());
                // Only repaint when the active line (or its karaoke fill)
                // actually moves — position ticks land roughly once a second.
                if !self.advance() {
                    return;
                }
            }
            LyricsCommandOutput::Tick => {
                if !self.advance() {
                    return;
                }
            }
            LyricsCommandOutput::Fetched(key, res) => {
                if self.key.as_ref() != Some(&key) {
                    // The track moved on while we were fetching — stale result.
                    return;
                }
                if res.transient
                    && let Some(delay) = lyrics::retry_delay(self.retry_attempt)
                {
                    self.retry_attempt += 1;
                    schedule_retry(&sender, key, delay);
                }
                self.lyrics = res.lyrics;
                self.source = res.source;
                self.loading = false;
                self.active_idx = self.compute_active();
            }
            LyricsCommandOutput::Retry(key) => {
                if self.key.as_ref() != Some(&key) || self.loading {
                    return;
                }
                self.loading = true;
                kick_fetch(&sender, key);
            }
            LyricsCommandOutput::OffsetChanged(v) => {
                self.offset_ms = v;
                self.active_idx = self.compute_active();
            }
            LyricsCommandOutput::SourcesChanged(netease, local_dir) => {
                // Forget the key so `refresh_track` resolves it again under
                // the new chain — only on a real change (the effect also
                // fires once at start-up).
                let next = (netease, local_dir);
                if std::mem::replace(&mut self.sources, next.clone()) != next
                    && self.key.take().is_some()
                {
                    self.refresh_track(&sender);
                }
            }
            LyricsCommandOutput::ShowLineChanged(v) => {
                self.show_line = v;
            }
            LyricsCommandOutput::MaxWidthChanged(v) => {
                self.max_width_chars = v;
            }
        }
        self.painted_split = apply_visual(widgets, self);
        self.sync_tick(&sender);
    }
}

impl LyricsModel {
    /// Lyric-clock position now: the smoothed player position plus the
    /// user's offset.
    fn lyric_position(&self) -> u64 {
        let track = self.key.as_ref().map(lyrics::offsets::get).unwrap_or(0);
        lyrics::effective_position(
            self.clock.at(Instant::now()),
            lyrics::total_offset(self.offset_ms, track),
        )
    }

    /// Active synced-line index for the current position, or `None`.
    fn compute_active(&self) -> Option<usize> {
        match &self.lyrics {
            Lyrics::Synced(lines) => lyrics::index_for_time(lines, self.lyric_position()),
            _ => None,
        }
    }

    /// The active line, when it is word-timed.
    fn active_word_line(&self) -> Option<&lyrics::LyricLine> {
        match &self.lyrics {
            Lyrics::Synced(lines) => self
                .active_idx
                .and_then(|i| lines.get(i))
                .filter(|l| !l.words.is_empty()),
            _ => None,
        }
    }

    /// Re-evaluate the active line + karaoke split; `true` when either moved
    /// (a repaint is due).
    fn advance(&mut self) -> bool {
        let new_idx = self.compute_active();
        let split = self
            .active_word_line()
            .filter(|_| new_idx == self.active_idx)
            .map(|l| lyrics::lrc::karaoke_split(l, self.lyric_position()))
            .unwrap_or(0);
        if new_idx == self.active_idx && split == self.painted_split {
            return false;
        }
        self.active_idx = new_idx;
        true
    }

    /// Run the fast karaoke tick only while it shows something: the line is
    /// in the bar, the lyrics are word-timed and the player is playing.
    fn sync_tick(&mut self, sender: &ComponentSender<Self>) {
        let want = self.show_line && self.clock.playing() && lyrics::has_word_timing(&self.lyrics);
        match (want, self.tick.is_some()) {
            (true, false) => {
                let cs = sender.command_sender().clone();
                self.tick = Some(glib::timeout_add_local(ui::KARAOKE_TICK, move || {
                    if cs.send(LyricsCommandOutput::Tick).is_ok() {
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                }));
            }
            (false, true) => {
                if let Some(id) = self.tick.take() {
                    id.remove();
                }
            }
            _ => {}
        }
    }

    /// Re-pick the display player, follow its position, and kick a lyrics fetch
    /// when the track key changed.
    fn refresh_track(&mut self, sender: &ComponentSender<Self>) {
        let Some(player) = ui::display_player() else {
            self.has_player = false;
            self.key = None;
            self.lyrics = Lyrics::None;
            self.source = LyricsSource::Lrclib;
            self.loading = false;
            self.active_idx = None;
            self.position_player_id = None;
            self.position_token.reset();
            self.clock.set_playing(false, Instant::now());
            return;
        };

        self.has_player = true;
        self.clock.set_playing(
            player.playback_state.get() == PlaybackState::Playing,
            Instant::now(),
        );

        // Follow this player's position if we switched players.
        let id = player.id.to_string();
        if self.position_player_id.as_deref() != Some(id.as_str()) {
            let token = self.position_token.reset();
            subscribe_position(player.clone(), sender, token);
            self.position_player_id = Some(id);
            self.clock.sample(0, Instant::now());
        }

        let key = lyrics::key_for(
            &player.metadata.title.get(),
            &player.metadata.artist.get(),
            &player.metadata.album.get(),
            player
                .metadata
                .length
                .get()
                .map(|d| d.as_secs())
                .unwrap_or(0),
        );

        if self.key.as_ref() == Some(&key) {
            return; // Same track — keep the lyrics we already have.
        }

        self.key = Some(key.clone());
        self.lyrics = Lyrics::None;
        self.source = LyricsSource::Lrclib;
        self.active_idx = None;
        self.retry_attempt = 0;
        if key.is_valid() {
            self.loading = true;
            kick_fetch(sender, key);
        } else {
            self.loading = false;
        }
    }
}

/// Watch every player's metadata + playback state under a fresh token so the
/// display player + track key follow playback across players.
fn subscribe_players(sender: &ComponentSender<LyricsModel>, token: &mut WatcherToken) {
    let token = token.reset();
    for player in media_service()
        .map(|m| m.player_list.get())
        .unwrap_or_default()
    {
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

/// Non-empty embedded lyrics text when the display player is mtune and its
/// current track's tags carry any — `None` for every other player.
fn embedded_hint() -> Option<String> {
    ui::display_player()
        .filter(|p| p.id.bus_name() == mshell_services::mtune::BUS_NAME)
        .map(|_| {
            mshell_services::mtune::mtune_service()
                .player
                .lyrics_embedded
                .get()
        })
        .filter(|s| !s.is_empty())
}

/// Resolve lyrics for `key` off the main thread, then deliver them tagged with
/// the key so a stale (track-changed) result can be discarded. mtune's own
/// embedded lyrics (when present) win over a lrclib lookup.
fn kick_fetch(sender: &ComponentSender<LyricsModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let embedded = embedded_hint();
        let opts = ui::fetch_options();
        let res = tokio::task::spawn_blocking(move || {
            lyrics::fetch(&for_fetch, embedded.as_deref(), &opts)
        })
        .await
        .unwrap_or(Resolution {
            lyrics: Lyrics::None,
            source: LyricsSource::Lrclib,
            transient: true,
        });
        let _ = out.send(LyricsCommandOutput::Fetched(key, res));
    });
}

/// Deliver [`LyricsCommandOutput::Retry`] for `key` after `delay`; the
/// handler drops it if the track moved on meanwhile.
fn schedule_retry(sender: &ComponentSender<LyricsModel>, key: TrackKey, delay: Duration) {
    sender.command(move |out, shutdown| async move {
        tokio::select! {
            () = shutdown.wait() => {}
            () = tokio::time::sleep(delay) => {
                let _ = out.send(LyricsCommandOutput::Retry(key));
            }
        }
    });
}

/// The source settings a resolved key depends on.
fn current_sources() -> (bool, String) {
    let lyr = || config_manager().config().bars().widgets().lyrics();
    (
        lyr().netease_fallback().get_untracked(),
        lyr().local_dir().get_untracked(),
    )
}

/// Clamp a configured bar-line width (chars) to a sane range so a stray value
/// can't collapse the pill or let it eat the whole bar.
fn clamp_width(chars: i32) -> i32 {
    chars.clamp(8, 120)
}

/// Paint the pill; returns the karaoke split it painted (0 when none).
fn apply_visual(widgets: &LyricsModelWidgets, model: &LyricsModel) -> usize {
    let root = &widgets.root;
    widgets.label.set_attributes(None);
    widgets.label.set_max_width_chars(model.max_width_chars);
    root.remove_css_class("has-lyrics");
    root.remove_css_class("dim");

    if !model.has_player {
        widgets.label.set_visible(false);
        root.add_css_class("dim");
        root.set_tooltip_text(Some("No media playing"));
        return 0;
    }

    if model.loading {
        widgets.label.set_label("…");
        widgets.label.set_visible(model.show_line);
        root.set_tooltip_text(Some("Loading lyrics…"));
        return 0;
    }

    match &model.lyrics {
        Lyrics::Synced(lines) => {
            let line = model
                .active_idx
                .and_then(|i| lines.get(i))
                .filter(|l| !l.text.trim().is_empty());
            let mut split = 0;
            match line {
                Some(l) => {
                    widgets.label.set_label(&l.text);
                    if !l.words.is_empty() {
                        let pos = model.lyric_position();
                        split = lyrics::lrc::karaoke_split(l, pos);
                        widgets
                            .label
                            .set_attributes(Some(&ui::karaoke_attrs(l, pos)));
                    }
                }
                None => widgets.label.set_label("♪"),
            }
            widgets.label.set_visible(model.show_line);
            root.add_css_class("has-lyrics");
            root.set_tooltip_text(Some(&format!(
                "Synced lyrics · {} — click for the full panel",
                model.source.label()
            )));
            return split;
        }
        Lyrics::Plain(_) => {
            widgets.label.set_visible(false);
            root.add_css_class("has-lyrics");
            let tip = match model.source {
                LyricsSource::Embedded => "Embedded lyrics — click to view".to_string(),
                src => format!(
                    "Lyrics available (unsynced) · {} — click to view",
                    src.label()
                ),
            };
            root.set_tooltip_text(Some(&tip));
        }
        Lyrics::Instrumental => {
            widgets.label.set_visible(false);
            root.add_css_class("dim");
            root.set_tooltip_text(Some("Instrumental — no lyrics"));
        }
        Lyrics::None => {
            widgets.label.set_visible(false);
            root.add_css_class("dim");
            root.set_tooltip_text(Some("No lyrics found"));
        }
    }
    0
}
