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
//!
//! Word-timed lyrics fill the active line word by word (the unsung rest
//! dimmed, [`ui::karaoke_attrs`]) on a short repaint tick that only runs
//! while the panel is open, the player plays and the lyrics carry word
//! timings. With `show_translation` on, a source's translation /
//! romanization sits under each line as a dim second line.
//!
//! Below the lines sits a player footer ([`super::transport`]): seek bar,
//! prev / play / next, and a per-track timing stepper for an LRC that is
//! out of sync for this one recording. Clicking a synced line seeks there.

use super::transport::{Transport, TransportState};
use crate::lyrics::{self, Lyrics, LyricsSource, PlayClock, Resolution, TrackKey, offsets, ui};
use futures::StreamExt;
use mshell_common::scoped_effects::EffectScope;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, LyricsBarWidgetStoreFields,
    MenuStoreFields, MenusStoreFields,
};
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use reactive_graph::traits::{Get, GetUntracked, Track};
use relm4::gtk::prelude::*;
use relm4::gtk::{glib, pango};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    source: LyricsSource,
    loading: bool,
    /// Smoothed playback position of the display player.
    clock: PlayClock,
    /// Global timing offset (ms), live from `bars.widgets.lyrics.offset_ms`.
    offset_ms: i32,
    /// Live `bars.widgets.lyrics.show_translation`.
    show_translation: bool,
    /// Last seen source settings (`netease_fallback`, `local_dir`) — tells
    /// a real change from the effect's first run.
    sources: (bool, String),
    /// The display player (transport target).
    player: Option<Arc<Player>>,
    /// The player footer (built after the view, see `init`).
    transport: Option<Transport>,
    /// A seek just sent: hold the shown position there until the player's
    /// own position catches up (≤ 1.5 s), so the bar doesn't jump back.
    pending_seek: Option<(Duration, Instant)>,
    /// Click-to-seek handlers on the line labels send through this.
    seek_sender: relm4::Sender<LyricsMenuWidgetInput>,
    active_idx: Option<usize>,
    /// Karaoke split (byte offset) last painted on the active line.
    painted_split: usize,
    /// Back-off retries already spent on the current track.
    retry_attempt: u32,
    /// The fast karaoke repaint tick, when running.
    tick: Option<glib::SourceId>,
    /// One label per *synced* line, in order — rebuilt when lyrics change,
    /// re-styled (active tint) as the position advances.
    line_labels: Vec<gtk::Label>,
    /// Configured cap (px) for the *inner* lines scroller, live from
    /// Settings → Widgets → Lyrics ("Maximum Height",
    /// `menus.lyrics_menu.maximum_height`). 0 = no cap.
    max_height: i32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetInput {
    /// Menu reveal toggled — start the fetch + position watch on show, stop on
    /// hide.
    ParentRevealChanged(bool),
    /// Refresh button — re-fetch the current track's lyrics, bypassing cache.
    Refresh,
    /// `menus.lyrics_menu.maximum_height` changed (Settings → Widgets →
    /// Lyrics) — applied live, no reopen needed.
    SetMaxHeight(i32),
    /// `bars.widgets.lyrics.offset_ms` changed.
    SetOffset(i32),
    /// `bars.widgets.lyrics.show_translation` changed.
    SetShowTranslation(bool),
    /// `bars.widgets.lyrics.netease_fallback` / `local_dir` changed.
    SetSources(bool, String),
    /// The player allow / block lists changed — re-pick the player.
    PlayerListsChanged,
    Previous,
    PlayPause,
    Next,
    /// Seek bar dragged (display only).
    SeekPreview(f64),
    /// Seek bar settled — seek there.
    SeekCommit(f64),
    /// A synced line was clicked — seek to where it starts.
    SeekToLine(usize),
    /// Per-track timing stepper: add `delta` ms (0 = reset).
    NudgeOffset(i32),
}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetOutput {}

pub(crate) struct LyricsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum LyricsMenuWidgetCommandOutput {
    PlayersChanged,
    TrackChanged,
    Position(Duration),
    Fetched(TrackKey, Resolution),
    /// A back-off retry for this track came due.
    Retry(TrackKey),
    /// Karaoke repaint tick.
    Tick,
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
                set_label: &model.badge_text(),
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
                // A GtkScrolledWindow reports ~0 natural height by default, so
                // in a menu that sizes to its content the lyrics viewport
                // would collapse to nothing and no lines render (header +
                // badge show, body is blank). `propagate_natural_height`
                // fixes that by sizing to the actual lines_box content
                // instead when uncapped (max_height == 0).
                //
                // When Settings → Widgets → Lyrics "Maximum Height" is > 0,
                // `pin_lines_scroller` (called imperatively from `init` and
                // `SetMaxHeight`, NOT via `#[watch]` here) pins min == max ==
                // that value, so it's a real target size the lines scroll
                // within — not just a ceiling — matching the AI / VPN / App
                // Launcher menus' `fixed_height` behaviour. It must be
                // imperative and reset-then-set (see `pin_scroller_bounds`'s
                // doc comment): a static min/max setter pair asserts on
                // whichever direction (grow or shrink) the value changes
                // against the *other* attribute's stale bound.
                set_propagate_natural_height: true,
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

        // Track the inner-scroller max height (Settings → Widgets → Lyrics)
        // live, so a Settings change applies immediately without reopening
        // the panel.
        let mut effects = EffectScope::new();
        {
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let h = config_manager()
                    .config()
                    .menus()
                    .lyrics_menu()
                    .maximum_height()
                    .get();
                eff_sender.input(LyricsMenuWidgetInput::SetMaxHeight(h));
            });
        }
        {
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let lyr = config_manager().config().bars().widgets().lyrics();
                eff_sender.input(LyricsMenuWidgetInput::SetOffset(lyr.offset_ms().get()));
            });
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let lyr = config_manager().config().bars().widgets().lyrics();
                eff_sender.input(LyricsMenuWidgetInput::SetShowTranslation(
                    lyr.show_translation().get(),
                ));
            });
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let lyr = || config_manager().config().bars().widgets().lyrics();
                eff_sender.input(LyricsMenuWidgetInput::SetSources(
                    lyr().netease_fallback().get(),
                    lyr().local_dir().get(),
                ));
            });
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let lyr = || config_manager().config().bars().widgets().lyrics();
                lyr().player_allowlist().track();
                lyr().player_blocklist().track();
                eff_sender.input(LyricsMenuWidgetInput::PlayerListsChanged);
            });
        }

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
            source: LyricsSource::Lrclib,
            loading: false,
            clock: PlayClock::new(Instant::now()),
            offset_ms: 0,
            show_translation: false,
            sources: {
                let o = ui::fetch_options();
                (
                    o.netease,
                    config_manager()
                        .config()
                        .bars()
                        .widgets()
                        .lyrics()
                        .local_dir()
                        .get_untracked(),
                )
            },
            player: None,
            transport: None,
            pending_seek: None,
            seek_sender: sender.input_sender().clone(),
            active_idx: None,
            painted_split: 0,
            retry_attempt: 0,
            tick: None,
            line_labels: Vec::new(),
            max_height: 0,
            _effects: effects,
        };

        let mut model = model;
        let widgets = view_output!();
        pin_lines_scroller(&widgets.scroller, model.max_height);
        let transport = Transport::new(&sender);
        root.append(transport.root());
        model.transport = Some(transport);

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
            LyricsMenuWidgetInput::SetOffset(v) => {
                self.offset_ms = v;
                self.active_idx = self.compute_active();
            }
            LyricsMenuWidgetInput::SetShowTranslation(v) => {
                if self.show_translation != v {
                    self.show_translation = v;
                    self.rebuild_lines(widgets);
                }
            }
            LyricsMenuWidgetInput::PlayerListsChanged => {
                if self.revealed {
                    self.pick_track(&sender);
                    self.rebuild_lines(widgets);
                }
            }
            LyricsMenuWidgetInput::Previous => {
                if let Some(p) = self.player.clone() {
                    tokio::spawn(async move {
                        let _ = p.previous().await;
                    });
                }
            }
            LyricsMenuWidgetInput::Next => {
                if let Some(p) = self.player.clone() {
                    tokio::spawn(async move {
                        let _ = p.next().await;
                    });
                }
            }
            LyricsMenuWidgetInput::PlayPause => {
                if let Some(p) = self.player.clone() {
                    // Flip the icon now (feedback within a frame); the
                    // player's own state change confirms it.
                    let now = Instant::now();
                    self.clock.set_playing(!self.clock.playing(), now);
                    tokio::spawn(async move {
                        let _ = p.play_pause().await;
                    });
                }
            }
            LyricsMenuWidgetInput::SeekPreview(f) => {
                if let (Some(t), Some(len)) = (&self.transport, self.track_length()) {
                    t.preview(f, len);
                }
                return;
            }
            LyricsMenuWidgetInput::SeekCommit(f) => {
                if let Some(len) = self.track_length() {
                    self.seek_to(len.mul_f64(f));
                }
            }
            LyricsMenuWidgetInput::SeekToLine(i) => {
                if let Lyrics::Synced(lines) = &self.lyrics
                    && let Some(line) = lines.get(i)
                {
                    let ms = lyrics::seek_position_for(line.time_ms, self.total_offset());
                    self.seek_to(Duration::from_millis(ms));
                }
            }
            LyricsMenuWidgetInput::NudgeOffset(delta) => {
                if let Some(key) = &self.key {
                    offsets::nudge(key, delta);
                    self.active_idx = self.compute_active();
                }
            }
            LyricsMenuWidgetInput::SetSources(netease, local_dir) => {
                let next = (netease, local_dir);
                if std::mem::replace(&mut self.sources, next.clone()) != next
                    && self.key.take().is_some()
                {
                    if self.revealed {
                        self.pick_track(&sender);
                        self.rebuild_lines(widgets);
                    } else {
                        // Resolved again on the next reveal.
                        self.lyrics = Lyrics::None;
                    }
                }
            }
            LyricsMenuWidgetInput::Refresh => {
                if self.revealed
                    && self.has_player
                    && let Some(key) = self.key.clone().filter(TrackKey::is_valid)
                {
                    self.lyrics = Lyrics::None;
                    self.source = LyricsSource::Lrclib;
                    self.active_idx = None;
                    self.loading = true;
                    self.retry_attempt = 0;
                    kick_refetch(&sender, key);
                    self.rebuild_lines(widgets);
                }
            }
            LyricsMenuWidgetInput::SetMaxHeight(h) => {
                self.max_height = h;
                pin_lines_scroller(&widgets.scroller, h);
            }
        }
        self.apply_active(widgets);
        self.sync_tick(&sender);
        self.refresh_transport();
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
                // Right after a seek, players report the old position for a
                // tick or two — ignore samples until they reach the target.
                if let Some((target, at)) = self.pending_seek {
                    let close = pos.abs_diff(target) < Duration::from_millis(1_500);
                    if close || at.elapsed() > Duration::from_millis(1_500) {
                        self.pending_seek = None;
                    } else {
                        return;
                    }
                }
                self.clock.sample(pos.as_millis() as u64, Instant::now());
                self.refresh_transport();
                if !self.advance() {
                    return;
                }
            }
            LyricsMenuWidgetCommandOutput::Tick => {
                if !self.advance() {
                    return;
                }
            }
            LyricsMenuWidgetCommandOutput::Fetched(key, res) => {
                if self.key.as_ref() != Some(&key) {
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
                self.rebuild_lines(widgets);
            }
            LyricsMenuWidgetCommandOutput::Retry(key) => {
                if !self.revealed || self.key.as_ref() != Some(&key) || self.loading {
                    return;
                }
                self.loading = true;
                kick_fetch(&sender, key);
                self.rebuild_lines(widgets);
            }
        }
        self.apply_active(widgets);
        self.sync_tick(&sender);
        self.refresh_transport();
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
    fn badge_text(&self) -> String {
        if !self.has_player {
            return String::new();
        }
        if self.loading {
            return "Searching lyrics…".to_string();
        }
        match (&self.lyrics, self.source) {
            (l @ Lyrics::Synced(_), src) => {
                let kind = if lyrics::has_word_timing(l) {
                    "Word-synced"
                } else {
                    "Synced"
                };
                format!("{kind} · {}", src.label())
            }
            (Lyrics::Plain(_), LyricsSource::Embedded) => "Embedded".to_string(),
            (Lyrics::Plain(_), src) => format!("Unsynced · {}", src.label()),
            (Lyrics::Instrumental, _) => "Instrumental".to_string(),
            (Lyrics::None, _) if self.retry_attempt > 0 => "Offline — retrying".to_string(),
            (Lyrics::None, _) => "No lyrics found".to_string(),
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

    /// Global offset + this track's nudge.
    fn total_offset(&self) -> i32 {
        let track = self.key.as_ref().map(offsets::get).unwrap_or(0);
        lyrics::total_offset(self.offset_ms, track)
    }

    /// Lyric-clock position now: smoothed player position + timing offset.
    fn lyric_position(&self) -> u64 {
        lyrics::effective_position(self.clock.at(Instant::now()), self.total_offset())
    }

    fn track_length(&self) -> Option<Duration> {
        self.player
            .as_ref()
            .and_then(|p| p.metadata.length.get())
            .filter(|d| !d.is_zero())
    }

    /// Seek the display player and move the clock there at once.
    fn seek_to(&mut self, target: Duration) {
        let Some(p) = self.player.clone() else {
            return;
        };
        if !p.can_seek.get() {
            return;
        }
        let target = self.track_length().map_or(target, |len| target.min(len));
        self.pending_seek = Some((target, Instant::now()));
        self.clock.sample(target.as_millis() as u64, Instant::now());
        self.active_idx = self.compute_active();
        tokio::spawn(async move {
            let _ = p.set_position(target).await;
        });
    }

    /// Push the current player state into the footer.
    fn refresh_transport(&mut self) {
        let synced = matches!(self.lyrics, Lyrics::Synced(_));
        let track_offset_ms = self.key.as_ref().map(offsets::get).unwrap_or(0);
        let st = self.player.as_ref().map(|p| TransportState {
            position: Duration::from_millis(self.clock.at(Instant::now())),
            length: p.metadata.length.get(),
            playing: self.clock.playing(),
            can_seek: p.can_seek.get(),
            can_prev: p.can_go_previous.get(),
            can_next: p.can_go_next.get(),
            synced,
            track_offset_ms,
        });
        if let Some(t) = &mut self.transport {
            t.root().set_visible(st.is_some());
            if let Some(st) = st {
                t.set_state(st);
            }
        }
    }

    fn compute_active(&self) -> Option<usize> {
        match &self.lyrics {
            Lyrics::Synced(lines) => lyrics::index_for_time(lines, self.lyric_position()),
            _ => None,
        }
    }

    /// Re-evaluate the active line and repaint the karaoke fill when it
    /// moved; `true` when the active *line* changed (re-tint + scroll due).
    fn advance(&mut self) -> bool {
        let new_idx = self.compute_active();
        if new_idx != self.active_idx {
            self.active_idx = new_idx;
            return true;
        }
        self.paint_karaoke();
        false
    }

    /// Fill the active word-timed line up to the current position (and clear
    /// any previous line's fill). Cheap: one attribute list, only on change.
    fn paint_karaoke(&mut self) {
        let Lyrics::Synced(lines) = &self.lyrics else {
            return;
        };
        let pos = self.lyric_position();
        let Some((i, line)) = self
            .active_idx
            .and_then(|i| lines.get(i).map(|l| (i, l)))
            .filter(|(_, l)| !l.words.is_empty())
        else {
            return;
        };
        let split = lyrics::lrc::karaoke_split(line, pos);
        if split == self.painted_split {
            return;
        }
        self.painted_split = split;
        if let Some(label) = self.line_labels.get(i) {
            label.set_attributes(Some(&ui::karaoke_attrs(line, pos)));
        }
    }

    /// Run the fast karaoke tick only while the open panel shows word-timed
    /// lyrics of a playing track.
    fn sync_tick(&mut self, sender: &ComponentSender<Self>) {
        let want = self.revealed && self.clock.playing() && lyrics::has_word_timing(&self.lyrics);
        match (want, self.tick.is_some()) {
            (true, false) => {
                let cs = sender.command_sender().clone();
                self.tick = Some(glib::timeout_add_local(ui::KARAOKE_TICK, move || {
                    if cs.send(LyricsMenuWidgetCommandOutput::Tick).is_ok() {
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

    /// Re-pick the display player, follow its position, refresh title/artist,
    /// and kick a fetch when the track key changed.
    fn pick_track(&mut self, sender: &ComponentSender<Self>) {
        let Some(player) = ui::display_player() else {
            self.player = None;
            self.has_player = false;
            self.title.clear();
            self.artist.clear();
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
        self.player = Some(player.clone());
        self.clock.set_playing(
            player.playback_state.get() == PlaybackState::Playing,
            Instant::now(),
        );
        self.title = player.metadata.title.get();
        self.artist = player.metadata.artist.get();

        let id = player.id.to_string();
        if self.position_player_id.as_deref() != Some(id.as_str()) {
            let token = self.position_token.reset();
            subscribe_position(player.clone(), sender, token);
            self.position_player_id = Some(id);
            self.clock.sample(0, Instant::now());
        }

        let key = track_key(&player);
        if self.key.as_ref() == Some(&key) {
            return;
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

    /// Rebuild the scrolling column from the current lyrics. Synced lines are
    /// tracked in `line_labels` for active-tinting; plain lines are static.
    fn rebuild_lines(&mut self, widgets: &LyricsMenuWidgetModelWidgets) {
        while let Some(child) = widgets.lines_box.first_child() {
            widgets.lines_box.remove(&child);
        }
        self.line_labels.clear();

        self.painted_split = usize::MAX;
        match &self.lyrics {
            Lyrics::Synced(lines) => {
                for (i, line) in lines.iter().enumerate() {
                    let label = make_line_label(&line.text, false);
                    make_seekable(&label, i, &self.seek_sender);
                    let secondary = line.secondary().filter(|_| self.show_translation);
                    match secondary {
                        Some(sub) => {
                            let row = gtk::Box::builder()
                                .orientation(gtk::Orientation::Vertical)
                                .css_classes(["lyrics-line-row"])
                                .build();
                            row.append(&label);
                            row.append(
                                &gtk::Label::builder()
                                    .label(sub)
                                    .css_classes(["lyrics-line-secondary"])
                                    .xalign(0.0)
                                    .wrap(true)
                                    .wrap_mode(pango::WrapMode::WordChar)
                                    .build(),
                            );
                            widgets.lines_box.append(&row);
                        }
                        None => widgets.lines_box.append(&label),
                    }
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

    /// Light the active synced line, scroll it to the centre and repaint its
    /// karaoke fill (a line that stopped being active loses its fill).
    fn apply_active(&mut self, widgets: &LyricsMenuWidgetModelWidgets) {
        for (i, label) in self.line_labels.iter().enumerate() {
            if Some(i) == self.active_idx {
                label.add_css_class("lyrics-line-active");
            } else {
                label.remove_css_class("lyrics-line-active");
                label.set_attributes(None);
            }
        }
        if let Some(label) = self.active_idx.and_then(|i| self.line_labels.get(i)) {
            // Centre the whole row (line + its translation).
            let target = label
                .parent()
                .filter(|p| p.has_css_class("lyrics-line-row"))
                .unwrap_or_else(|| label.clone().upcast());
            scroll_center(&widgets.scroller, &target);
        }
        self.painted_split = usize::MAX;
        self.paint_karaoke();
    }
}

/// Pin the lines scroller to `max_height` (Settings → Widgets → Lyrics
/// "Maximum Height"): `0` clears both bounds (uncapped, sizes to content via
/// `propagate_natural_height`); `> 0` sets min == max, making it a real
/// target size — matching the AI / VPN / App Launcher menus' `fixed_height`
/// behaviour, not just a shrink-only ceiling. Always resets to `-1` before
/// setting the new pair (mirrors `pin_scroller_bounds` in `menu.rs`): with a
/// static setter order, whichever attribute applies second would assert
/// (`min <= max`) against the *other* one's stale bound on every grow *or*
/// shrink, depending on which direction the value changed.
fn pin_lines_scroller(scroller: &gtk::ScrolledWindow, max_height: i32) {
    let h = if max_height > 0 { max_height } else { -1 };
    scroller.set_min_content_height(-1);
    scroller.set_max_content_height(h);
    scroller.set_min_content_height(h);
}

/// Click a synced line → seek to where it starts. The seek bar covers the
/// keyboard path; this is the direct-manipulation shortcut.
fn make_seekable(label: &gtk::Label, index: usize, sender: &relm4::Sender<LyricsMenuWidgetInput>) {
    label.add_css_class("lyrics-line-seekable");
    label.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    let s = sender.clone();
    click.connect_released(move |gesture, n_press, _, _| {
        if n_press == 1 {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let _ = s.send(LyricsMenuWidgetInput::SeekToLine(index));
        }
    });
    label.add_controller(click);
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
fn scroll_center(scroller: &gtk::ScrolledWindow, label: &gtk::Widget) {
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

fn subscribe_players(sender: &ComponentSender<LyricsMenuWidgetModel>, token: &mut WatcherToken) {
    let token = token.reset();
    for player in media_service()
        .map(|m| m.player_list.get())
        .unwrap_or_default()
    {
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

/// A failed blocking task (panic / cancelled) reads as a transient miss.
fn failed_fetch() -> Resolution {
    Resolution {
        lyrics: Lyrics::None,
        source: LyricsSource::Lrclib,
        transient: true,
    }
}

fn kick_fetch(sender: &ComponentSender<LyricsMenuWidgetModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let embedded = embedded_hint();
        let opts = ui::fetch_options();
        let res = tokio::task::spawn_blocking(move || {
            lyrics::fetch(&for_fetch, embedded.as_deref(), &opts)
        })
        .await
        .unwrap_or_else(|_| failed_fetch());
        let _ = out.send(LyricsMenuWidgetCommandOutput::Fetched(key, res));
    });
}

/// Deliver a [`LyricsMenuWidgetCommandOutput::Retry`] for `key` after
/// `delay`; dropped by the handler if the track moved on or the panel closed.
fn schedule_retry(sender: &ComponentSender<LyricsMenuWidgetModel>, key: TrackKey, delay: Duration) {
    sender.command(move |out, shutdown| async move {
        tokio::select! {
            () = shutdown.wait() => {}
            () = tokio::time::sleep(delay) => {
                let _ = out.send(LyricsMenuWidgetCommandOutput::Retry(key));
            }
        }
    });
}

/// Like [`kick_fetch`] but bypasses the disk cache (manual refresh) — always
/// a real lrclib lookup, deliberately ignoring any embedded hint so the
/// refresh button can be used to double-check a track's lrclib match.
fn kick_refetch(sender: &ComponentSender<LyricsMenuWidgetModel>, key: TrackKey) {
    sender.command(move |out, _shutdown| async move {
        let for_fetch = key.clone();
        let opts = ui::fetch_options();
        let res = tokio::task::spawn_blocking(move || lyrics::refetch(&for_fetch, &opts))
            .await
            .unwrap_or_else(|_| failed_fetch());
        let _ = out.send(LyricsMenuWidgetCommandOutput::Fetched(key, res));
    });
}
