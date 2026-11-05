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

use crate::lyrics::{self, Lyrics, LyricsSource, PlayClock, Resolution, TrackKey, ui};
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
use reactive_graph::traits::Get;
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
    /// Last seen `netease_fallback` (tells a change from the first run).
    netease: bool,
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
    /// `bars.widgets.lyrics.netease_fallback` changed.
    SetNetease(bool),
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
                let lyr = config_manager().config().bars().widgets().lyrics();
                eff_sender.input(LyricsMenuWidgetInput::SetNetease(
                    lyr.netease_fallback().get(),
                ));
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
            netease: ui::fetch_options().netease,
            active_idx: None,
            painted_split: 0,
            retry_attempt: 0,
            tick: None,
            line_labels: Vec::new(),
            max_height: 0,
            _effects: effects,
        };

        let widgets = view_output!();
        pin_lines_scroller(&widgets.scroller, model.max_height);

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
            LyricsMenuWidgetInput::SetNetease(v) => {
                if std::mem::replace(&mut self.netease, v) != v && self.key.take().is_some() {
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
                self.clock.sample(pos.as_millis() as u64, Instant::now());
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

    /// Lyric-clock position now: smoothed player position + user offset.
    fn lyric_position(&self) -> u64 {
        lyrics::effective_position(self.clock.at(Instant::now()), self.offset_ms)
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
        let Some(player) = display_player() else {
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
                for line in lines {
                    let label = make_line_label(&line.text, false);
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

/// The player to mirror: first actually playing, else active, else first.
fn display_player() -> Option<Arc<Player>> {
    let svc = media_service();
    let players = svc
        .as_ref()
        .map(|s| s.player_list.get())
        .unwrap_or_default();
    players
        .iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .cloned()
        .or_else(|| svc.as_ref().and_then(|s| s.active_player.get()))
        .or_else(|| players.first().cloned())
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
    display_player()
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
