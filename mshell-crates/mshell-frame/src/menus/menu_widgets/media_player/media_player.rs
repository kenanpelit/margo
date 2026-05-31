use mshell_common::WatcherToken;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_utils::media::spawn_media_player_watcher;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wayle_media::core::player::Player;
use wayle_media::types::{LoopMode, PlaybackState, ShuffleMode};

#[derive(Clone, Copy)]
enum ScrollState {
    PauseStart(u32),
    Scrolling,
    PauseEnd(u32),
}

pub(crate) struct MediaPlayerModel {
    pub player: Arc<Player>,
    /// Keeps the player's metadata / position / state watchers
    /// alive for the lifetime of the model — dropping a
    /// `WatcherToken` cancels its watchers, so this MUST be a
    /// field, not a local in `init`.
    _watcher_token: WatcherToken,
    track_name: String,
    track_name_scroll_source: Option<glib::SourceId>,
    artist_name: String,
    artist_name_scroll_source: Option<glib::SourceId>,
    current_track_time: String,
    track_length: String,
    scale_value_changed_signal: Option<glib::SignalHandlerId>,
    position: Duration,
    pending_seek: Option<(Duration, Instant)>, // (target, when_we_sent_it)
    /// Step for the ⏪ / ⏩ relative-seek buttons (general.media_seek_step_seconds).
    seek_step: Duration,
    /// Album-cover pixel size, from general.media_large_album_art.
    cover_size: i32,
    can_loop: bool,
    can_shuffle: bool,
    can_go_next: bool,
    can_go_previous: bool,
    can_play: bool,
    can_seek: bool,
    playback_state: PlaybackState,
    shuffle_mode: ShuffleMode,
    loop_mode: LoopMode,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerInput {
    ScaleChanged(f64), // fires continuously while dragging, only updates pending_seek display
    ScaleClicked(f64), // fires once on mouse up, triggers actual seek
    SeekBackClicked,   // jump back by seek_step
    SeekForwardClicked, // jump forward by seek_step
    ShuffleClicked,
    LoopClicked,
    PreviousClicked,
    NextClicked,
    PlayPauseClicked,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerOutput {}

pub(crate) struct MediaPlayerInit {
    pub player: Arc<Player>,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerCommandOutput {
    PlaybackStateChanged,
    MetaDataChanged,
    LoopModeChanged,
    ShuffleModeChanged,
    CapabilitiesChanged,
    PositionChanged(Duration),
}

#[relm4::component(pub)]
impl Component for MediaPlayerModel {
    type CommandOutput = MediaPlayerCommandOutput;
    type Input = MediaPlayerInput;
    type Output = MediaPlayerOutput;
    type Init = MediaPlayerInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "media-player-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,

            // ── Top row: album cover + info column ──────────────
            gtk::Box {
                add_css_class: "media-player-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_hexpand: true,
                // Gap between the album cover and the title/artist column.
                // GtkBox spacing is a widget property, not CSS, so it has to
                // be set here (--space-3 = 12px) — the cover and text read as
                // glued together at 0.
                set_spacing: 12,

                #[name = "cover"]
                gtk::Image {
                    add_css_class: "media-player-cover",
                    set_pixel_size: model.cover_size,
                    set_valign: gtk::Align::Center,
                },

                // Info column: title / artist / progress
                gtk::Box {
                    add_css_class: "media-player-info",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                    set_spacing: 2,

                    // Title — single line, ellipsised, left-aligned
                    #[name = "track_scroll_window"]
                    gtk::ScrolledWindow {
                        set_policy: (gtk::PolicyType::External, gtk::PolicyType::Never),
                        set_overflow: gtk::Overflow::Hidden,
                        set_hexpand: true,

                        #[name = "track"]
                        gtk::Label {
                            add_css_class: "media-player-track",
                            #[watch]
                            set_label: model.track_name.as_str(),
                            set_xalign: 0.0,
                            set_single_line_mode: true,
                            set_wrap: false,
                            set_max_width_chars: -1,
                            set_ellipsize: pango::EllipsizeMode::End,
                        },
                    },

                    // Artist — separate line, dimmed, single line, ellipsised
                    #[name = "artist_scroll_window"]
                    gtk::ScrolledWindow {
                        set_policy: (gtk::PolicyType::External, gtk::PolicyType::Never),
                        set_overflow: gtk::Overflow::Hidden,
                        set_hexpand: true,

                        #[name = "artist"]
                        gtk::Label {
                            add_css_class: "media-player-artist",
                            #[watch]
                            set_label: model.artist_name.as_str(),
                            set_xalign: 0.0,
                            set_single_line_mode: true,
                            set_wrap: false,
                            set_max_width_chars: -1,
                            set_ellipsize: pango::EllipsizeMode::End,
                        },
                    },

                    // Progress row — time labels flanking the seek bar
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

            // ── Bottom row: centred playback controls ────────────
            // Inter-button gap is a GtkBox property (CSS `spacing` is a
            // no-op in GTK), so it's set here — the row read cramped
            // ("iç içe") before because only the SCSS had it.
            gtk::Box {
                add_css_class: "media-player-controls",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Center,
                set_spacing: 8,

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.can_shuffle,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::ShuffleClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_icon_name: match model.shuffle_mode {
                            ShuffleMode::On => Some("media-shuffle-symbolic"),
                            ShuffleMode::Off => Some("media-shuffle-off-symbolic"),
                            ShuffleMode::Unsupported => Some("media-shuffle-off-symbolic"),
                        },
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.can_go_previous,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::PreviousClicked);
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
                    #[watch]
                    set_sensitive: model.can_play,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::PlayPauseClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_icon_name: match model.playback_state {
                            PlaybackState::Playing => Some("media-pause-symbolic"),
                            PlaybackState::Paused => Some("media-play-symbolic"),
                            PlaybackState::Stopped => Some("media-play-symbolic"),
                        },
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.can_go_next,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::NextClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("media-skip-next-symbolic"),
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    #[watch]
                    set_sensitive: model.can_loop,
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::LoopClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_icon_name: match model.loop_mode {
                            LoopMode::None => Some("media-repeat-off-symbolic"),
                            LoopMode::Track => Some("media-repeat-once-symbolic"),
                            LoopMode::Playlist => Some("media-repeat-symbolic"),
                            LoopMode::Unsupported => Some("media-repeat-off-symbolic"),
                        },
                    },
                },
            },

            // ── Relative-seek buttons (±seek_step), ported from
            // the mplayerplus plugin. Hidden when the player can't
            // seek; the draggable progress bar above is unaffected.
            gtk::Box {
                add_css_class: "media-player-seek",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Center,
                set_spacing: 8,
                #[watch]
                set_visible: model.can_seek,

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_label: &format!("−{}s", model.seek_step.as_secs()),
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::SeekBackClicked);
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_label: &format!("+{}s", model.seek_step.as_secs()),
                    connect_clicked[sender] => move |_| {
                        sender.input(MediaPlayerInput::SeekForwardClicked);
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut watcher_token = WatcherToken::new();

        let token = watcher_token.reset();

        spawn_media_player_watcher(
            &params.player,
            &sender,
            token,
            || MediaPlayerCommandOutput::PlaybackStateChanged,
            || MediaPlayerCommandOutput::MetaDataChanged,
            || MediaPlayerCommandOutput::LoopModeChanged,
            || MediaPlayerCommandOutput::ShuffleModeChanged,
            || MediaPlayerCommandOutput::CapabilitiesChanged,
            MediaPlayerCommandOutput::PositionChanged,
        );

        let position = params.player.position.get();
        let current_track_time = format_duration(position);
        let track_length = format_duration(
            params
                .player
                .metadata
                .length
                .get()
                .unwrap_or(Duration::new(0, 0)),
        );

        let can_shuffle = params.player.can_shuffle.get();
        let can_loop = params.player.can_loop.get();
        let can_go_next = params.player.can_go_next.get();
        let can_go_previous = params.player.can_go_previous.get();
        let can_play = params.player.can_play.get();
        let can_seek = params.player.can_seek.get();
        let playback_state = params.player.playback_state.get();
        let shuffle_mode = params.player.shuffle_mode.get();
        let loop_mode = params.player.loop_mode.get();

        // Media-player knobs from general config (ported from the mplayerplus
        // plugin's settings). Read once at construction; a new player spawns a
        // fresh component, so toggling these in Settings takes effect on the
        // next track / restart.
        let seek_step = Duration::from_secs(
            config_manager()
                .config()
                .general()
                .media_seek_step_seconds()
                .get_untracked()
                .max(1) as u64,
        );
        let cover_size = if config_manager()
            .config()
            .general()
            .media_large_album_art()
            .get_untracked()
        {
            128
        } else {
            64
        };

        let mut model = MediaPlayerModel {
            player: params.player,
            _watcher_token: watcher_token,
            track_name: "".to_string(),
            track_name_scroll_source: None,
            artist_name: "".to_string(),
            artist_name_scroll_source: None,
            current_track_time,
            track_length,
            scale_value_changed_signal: None,
            position,
            pending_seek: None,
            seek_step,
            cover_size,
            can_shuffle,
            can_loop,
            can_go_next,
            can_go_previous,
            can_play,
            can_seek,
            playback_state,
            shuffle_mode,
            loop_mode,
        };

        let widgets = view_output!();

        apply_cover(&widgets.cover, &model.player);

        model.track_name_scroll_source = Some(start_scroll(&widgets.track_scroll_window));
        model.artist_name_scroll_source = Some(start_scroll(&widgets.artist_scroll_window));

        model.scale_value_changed_signal = Some(setup_scale_seek(&widgets.scale, &sender));

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
            MediaPlayerInput::ScaleChanged(value) => {
                if let Some(length) = self.player.metadata.length.get() {
                    let position = length.mul_f64(value);
                    let now = Instant::now();
                    self.pending_seek = Some((position, now));
                    self.current_track_time = format_duration(position);
                    if let Some(scale_signal) = &self.scale_value_changed_signal {
                        widgets.scale.block_signal(scale_signal);
                        widgets.scale.set_value(value);
                        widgets.scale.unblock_signal(scale_signal);
                    }
                }
            }

            MediaPlayerInput::ScaleClicked(value) => {
                let player = self.player.clone();
                if let Some(length) = player.metadata.length.get() {
                    let new_position = length.mul_f64(value);
                    let now = Instant::now();
                    self.pending_seek = Some((new_position, now));
                    tokio::spawn(async move {
                        let _ = player.set_position(new_position).await;
                    });
                }
            }
            MediaPlayerInput::SeekBackClicked => {
                let new_position = self.position.saturating_sub(self.seek_step);
                self.pending_seek = Some((new_position, Instant::now()));
                self.current_track_time = format_duration(new_position);
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.set_position(new_position).await;
                });
            }
            MediaPlayerInput::SeekForwardClicked => {
                let mut new_position = self.position + self.seek_step;
                if let Some(length) = self.player.metadata.length.get()
                    && new_position > length
                {
                    new_position = length;
                }
                self.pending_seek = Some((new_position, Instant::now()));
                self.current_track_time = format_duration(new_position);
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.set_position(new_position).await;
                });
            }
            MediaPlayerInput::ShuffleClicked => {
                let current_mode = self.player.shuffle_mode.get();
                let new_mode = match current_mode {
                    ShuffleMode::On => ShuffleMode::Off,
                    ShuffleMode::Off => ShuffleMode::On,
                    ShuffleMode::Unsupported => ShuffleMode::Off,
                };

                self.shuffle_mode = new_mode;

                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.set_shuffle_mode(new_mode).await;
                });
            }
            MediaPlayerInput::LoopClicked => {
                let current_mode = self.player.loop_mode.get();
                let new_mode = match current_mode {
                    LoopMode::None => LoopMode::Playlist,
                    LoopMode::Track => LoopMode::None,
                    LoopMode::Playlist => LoopMode::Track,
                    LoopMode::Unsupported => LoopMode::None,
                };

                self.loop_mode = new_mode;

                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.set_loop_mode(new_mode).await;
                });
            }
            MediaPlayerInput::NextClicked => {
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.next().await;
                });
            }
            MediaPlayerInput::PreviousClicked => {
                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.previous().await;
                });
            }
            MediaPlayerInput::PlayPauseClicked => {
                let current_mode = self.player.playback_state.get();
                let new_mode = match current_mode {
                    PlaybackState::Playing => PlaybackState::Paused,
                    PlaybackState::Paused => PlaybackState::Playing,
                    PlaybackState::Stopped => PlaybackState::Playing,
                };

                self.playback_state = new_mode;

                let player = self.player.clone();
                tokio::spawn(async move {
                    let _ = player.play_pause().await;
                });
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
            MediaPlayerCommandOutput::PlaybackStateChanged => {
                self.playback_state = self.player.playback_state.get();
            }
            MediaPlayerCommandOutput::MetaDataChanged => {
                let title = self.player.metadata.title.get();
                let artist = self.player.metadata.artist.get();

                // Title and artist are shown on separate lines.
                if self.track_name != title {
                    self.track_name = title;
                    widgets.track_scroll_window.hadjustment().set_value(0.0);
                }
                if self.artist_name != artist {
                    self.artist_name = artist;
                    widgets.artist_scroll_window.hadjustment().set_value(0.0);
                }

                apply_cover(&widgets.cover, &self.player);
            }
            MediaPlayerCommandOutput::LoopModeChanged => {
                self.loop_mode = self.player.loop_mode.get();
            }
            MediaPlayerCommandOutput::ShuffleModeChanged => {
                self.shuffle_mode = self.player.shuffle_mode.get();
            }
            MediaPlayerCommandOutput::CapabilitiesChanged => {
                self.can_loop = self.player.can_loop.get();
                self.can_shuffle = self.player.can_shuffle.get();
                self.can_go_next = self.player.can_go_next.get();
                self.can_go_previous = self.player.can_go_previous.get();
                self.can_play = self.player.can_play.get();
                self.can_seek = self.player.can_seek.get();
            }
            MediaPlayerCommandOutput::PositionChanged(position) => {
                self.position = position;

                if let Some((_, sent_at)) = self.pending_seek {
                    let timed_out = sent_at.elapsed() > Duration::from_secs(3);
                    if timed_out {
                        self.pending_seek = None;
                    } else {
                        // Still in the ignore window, don't update scale
                        self.update_view(widgets, sender);
                        return;
                    }
                }

                self.current_track_time = format_duration(position);
                if let Some(length) = self.player.metadata.length.get() {
                    self.track_length = format_duration(length);
                    if let Some(scale_signal) = &self.scale_value_changed_signal {
                        if !length.is_zero() {
                            widgets.scale.block_signal(scale_signal);
                            widgets
                                .scale
                                .set_value(position.as_secs_f64() / length.as_secs_f64());
                            widgets.scale.unblock_signal(scale_signal);
                        } else {
                            widgets.scale.set_value(0.0);
                        }
                    } else {
                        widgets.scale.set_value(0.0);
                    }
                } else {
                    widgets.scale.set_value(0.0);
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

fn setup_scale_seek(
    scale: &gtk::Scale,
    sender: &ComponentSender<MediaPlayerModel>,
) -> glib::SignalHandlerId {
    let pending_source: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));

    let sender = sender.clone();
    scale.connect_value_changed(move |scale| {
        // Cancel previous debounced seek
        if let Some(source_id) = pending_source.take() {
            source_id.remove();
        }

        let value = scale.value();

        // Immediately update display so it doesn't jump while dragging
        sender.input(MediaPlayerInput::ScaleChanged(value));

        let seek_sender = sender.clone();
        let pending = pending_source.clone();

        // Only actually seek after dragging stops
        let source_id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
            pending.set(None);
            seek_sender.input(MediaPlayerInput::ScaleClicked(value));
        });

        pending_source.set(Some(source_id));
    })
}

fn start_scroll(scrolled_window: &gtk::ScrolledWindow) -> glib::SourceId {
    let state = Rc::new(Cell::new(ScrollState::PauseStart(0)));
    let scroll = scrolled_window.clone();
    glib::timeout_add_local(Duration::from_millis(30), move || {
        let adj = scroll.hadjustment();
        let max = adj.upper() - adj.page_size();
        if max <= 0.0 {
            return glib::ControlFlow::Continue;
        }
        match state.get() {
            ScrollState::PauseStart(n) => {
                if n >= 40 {
                    state.set(ScrollState::Scrolling);
                } else {
                    state.set(ScrollState::PauseStart(n + 1));
                }
            }
            ScrollState::Scrolling => {
                let current = adj.value();
                if current >= max {
                    state.set(ScrollState::PauseEnd(0));
                } else {
                    adj.set_value(current + 2.0);
                }
            }
            ScrollState::PauseEnd(n) => {
                if n >= 40 {
                    adj.set_value(0.0);
                    state.set(ScrollState::PauseStart(0));
                } else {
                    state.set(ScrollState::PauseEnd(n + 1));
                }
            }
        }
        glib::ControlFlow::Continue
    })
}

/// Paint the album cover from `TrackMetadata::cover_art` — a
/// local path resolved by wayle-media's art cache (downloaded for
/// remote http(s) covers, used directly for `file://` art). When
/// there's no cover, fall back to a generic audio glyph.
fn apply_cover(image: &gtk::Image, player: &Player) {
    match player.metadata.cover_art.get() {
        Some(path) if !path.trim().is_empty() => {
            image.set_from_file(Some(&path));
        }
        _ => {
            image.set_icon_name(Some("audio-x-generic-symbolic"));
        }
    }
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
