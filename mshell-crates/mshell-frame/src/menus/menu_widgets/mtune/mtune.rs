//! Tune menu — the dedicated panel for the `mtune` folder-first music
//! player: now-playing, transport, speed, and the library / playlist
//! controls the generic MPRIS media menu can't offer. Talks only to
//! `mtune_service()` (→ `org.margo.Tune`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use mshell_services::mtune::{mtune_service, spawn_mtune};
use mshell_services::tokio_rt_spawn;
use mshell_utils::media::format_duration;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[path = "actions.rs"]
mod actions;
#[path = "presentation.rs"]
mod presentation;
#[path = "queue.rs"]
mod queue;
#[path = "radio.rs"]
mod radio;
#[path = "radio_state.rs"]
mod radio_state;
#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::list_fit;
use crate::menus::menu_widgets::panel_header::PanelHeader;
use presentation::{apply_dynamic, read, rebuild_playlists, setup_seek};
use queue::rebuild_queue_rows;
use radio_state::{RadioState, Station};

const RATE_PRESETS: [(f64, &str); 5] = [
    (0.75, "0.75×"),
    (1.0, "1×"),
    (1.25, "1.25×"),
    (1.5, "1.5×"),
    (2.0, "2×"),
];

pub(crate) struct MtuneMenuWidgetModel {
    tab: usize,
    is_live: bool,
    station_name: String,
    stream_error: String,
    radio_stations: Vec<Station>,
    radio: RadioState,
    radio_busy: bool,
    radio_message: String,
    radio_panel: Option<radio::RadioPanel>,
    volume: f64,
    volume_signal: Option<gtk::glib::SignalHandlerId>,
    running: bool,
    playing: bool,
    has_song: bool,
    title: String,
    artist: String,
    album: String,
    cover_art: Option<String>,
    shuffle: bool,
    /// `"consecutive"` / `"repeat-all"` / `"repeat-one"` / `"repeat-each"`.
    repeat: String,
    /// Configured N for `"repeat-each"`.
    repeat_count: u32,
    /// Which play of the current track this is (0-based) under repeat-each.
    repeat_plays: u32,
    rate: f64,
    queue_len: u32,
    current_index: i64,
    /// (title, artist, duration_secs) per entry, queue order.
    queue_entries: Vec<(String, String, u64)>,
    /// Live text from the queue filter entry.
    queue_filter: String,
    roots: Vec<String>,
    playlists: Vec<String>,
    scanning: bool,
    scan_progress: (u32, u32),
    position: Duration,
    duration: Duration,
    /// Set while a seek is in flight — ignore incoming position ticks
    /// until mtune catches up (or the ~1s guard expires) so the bar
    /// doesn't snap back under the cursor.
    pending_seek: Option<(Duration, Instant)>,
    /// The seek scale's `value-changed` handler, blocked while the bar
    /// is moved programmatically.
    seek_signal: Option<gtk::glib::SignalHandlerId>,
}

#[derive(Debug)]
pub(crate) enum MtuneMenuInput {
    Tab(usize),
    SearchRadio(String, String),
    RadioAction(radio::Action),
    SetVolume(f64),
    PlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,
    /// The repeat-count stepper's − / + was clicked (delta = ±1).
    AdjustRepeatCount(i32),
    SetRate(f64),
    /// Scale moved (0.0–1.0) — update the readout, debounce the real seek.
    SeekPreview(f64),
    /// Debounce elapsed — actually seek to this fraction.
    Seek(f64),
    ChooseFolder,
    OpenPlaylist,
    LoadPlaylist(String),
    Rescan,
    OpenTune,
    Launch,
    /// Queue filter text changed.
    FilterQueue(String),
    /// A queue row was clicked.
    PlayQueueIndex(u32),
    /// A queue row's remove (×) button was clicked.
    RemoveQueueIndex(u32),
}

#[derive(Debug)]
pub(crate) enum MtuneMenuOutput {}

pub(crate) struct MtuneMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum MtuneMenuCmd {
    SearchResult(u64, Result<Vec<Station>, String>),
    RadioFinished(Result<(), String>),
    Refresh,
    PositionTick(Duration),
}

#[relm4::component(pub)]
impl Component for MtuneMenuWidgetModel {
    type CommandOutput = MtuneMenuCmd;
    type Input = MtuneMenuInput;
    type Output = MtuneMenuOutput;
    type Init = MtuneMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "mtune-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 16,
            set_hexpand: true,

            #[name = "header_host"]
            gtk::Box {},

            // ── Now playing ────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-hero",
                set_spacing: 12,

                #[name = "cover"]
                gtk::Image {
                    add_css_class: "mtune-menu-cover",
                    set_pixel_size: 112,
                    set_valign: gtk::Align::Start,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,
                    set_spacing: 4,
                    gtk::Label {
                        add_css_class: "mtune-menu-section-label",
                        set_xalign: 0.0,
                        #[watch]
                        set_label: if model.is_live { "LIVE RADIO" } else if model.playing { "NOW PLAYING" } else { "READY TO LISTEN" },
                    },
                    gtk::Label {
                        add_css_class: "mtune-menu-title",
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                        #[watch]
                        set_label: if model.is_live && model.title.is_empty() {
                            model.station_name.as_str()
                        } else if model.has_song {
                            model.title.as_str()
                        } else if model.running {
                            "Nothing playing"
                        } else {
                            "Tune"
                        },
                    },
                    gtk::Label {
                        add_css_class: "mtune-menu-artist",
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                        #[watch]
                        set_visible: !model.artist.is_empty(),
                        #[watch]
                        set_label: &model.artist,
                    },
                    gtk::Label {
                        add_css_class: "mtune-menu-meta",
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                        #[watch]
                        set_label: &model.now_playing_meta(),
                        #[watch]
                        set_visible: model.running && (model.is_live || model.has_song || model.queue_len > 0),
                    },
                },
            },

            gtk::Label {
                add_css_class: "mtune-stream-error",
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_label: &model.stream_error,
                #[watch]
                set_visible: !model.stream_error.is_empty(),
            },
            // ── Seek bar ───────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-progress",
                set_spacing: 8,
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: model.running && !model.is_live && model.has_song && !model.duration.is_zero(),

                #[name = "elapsed"]
                gtk::Label { add_css_class: "mtune-menu-time" },

                #[name = "seek"]
                gtk::Scale {
                    add_css_class: "ok-progress-bar",
                    set_hexpand: true,
                    set_can_focus: true,
                    set_tooltip_text: Some("Seek in track"),
                        update_property: &[gtk::accessible::Property::Label("Seek in track")],
                    set_range: (0.0, 1.0),
                },

                #[name = "total"]
                gtk::Label { add_css_class: "mtune-menu-time" },
            },

            // ── Controls (transport + shuffle/repeat + speed) ────
            gtk::Box {
                add_css_class: "mtune-menu-controls",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                set_halign: gtk::Align::Fill,
                #[watch]
                set_sensitive: model.running,

                gtk::Box {
                    add_css_class: "mtune-menu-transport",
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Center,
                    set_spacing: 8,

                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-backward-symbolic",
                        set_tooltip_text: Some("Previous"),
                        update_property: &[gtk::accessible::Property::Label("Previous")],
                        #[watch]
                        set_sensitive: model.running && model.queue_len > 1,
                        connect_clicked => MtuneMenuInput::Previous,
                    },
                    #[name = "play_btn"]
                    gtk::Button {
                        set_css_classes: &["mtune-round", "mtune-round-primary"],
                        #[watch]
                        set_icon_name: if model.playing {
                            "media-playback-pause-symbolic"
                        } else {
                            "media-playback-start-symbolic"
                        },
                        set_tooltip_text: Some("Play / Pause"),
                        update_property: &[gtk::accessible::Property::Label("Play / Pause")],
                        #[watch]
                        set_sensitive: model.running && (model.is_live || model.has_song || model.queue_len > 0),
                        connect_clicked => MtuneMenuInput::PlayPause,
                    },
                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-forward-symbolic",
                        set_tooltip_text: Some("Next"),
                        update_property: &[gtk::accessible::Property::Label("Next")],
                        #[watch]
                        set_sensitive: model.running && model.queue_len > 1,
                        connect_clicked => MtuneMenuInput::Next,
                    },
                },

                gtk::Image {
                    set_icon_name: Some("audio-volume-high-symbolic"),
                    set_hexpand: true,
                    set_halign: gtk::Align::End,
                },
                #[name = "volume"]
                gtk::Scale {
                    set_width_request: 80,
                    set_range: (0.0, 1.0),
                    set_valign: gtk::Align::Center,
                    set_draw_value: false,
                    set_tooltip_text: Some("Playback volume"),
                        update_property: &[gtk::accessible::Property::Label("Playback volume")],
                    #[watch]
                    set_sensitive: model.running,
                },
            },
                    #[name = "repeat_count_box"]
                    gtk::Box {
                        add_css_class: "mtune-repeat-count",
                        set_spacing: 4,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: !model.is_live && model.repeat == "repeat-each",

                        gtk::Button {
                            set_css_classes: &["mtune-repeat-count-btn"],
                            set_icon_name: "list-remove-symbolic",
                            set_tooltip_text: Some("Repeat fewer times"),
                        update_property: &[gtk::accessible::Property::Label("Repeat fewer times")],
                            connect_clicked => MtuneMenuInput::AdjustRepeatCount(-1),
                        },
                        gtk::Label {
                            add_css_class: "mtune-repeat-count-label",
                            #[watch]
                            set_label: &model.repeat_count.to_string(),
                        },
                        gtk::Button {
                            set_css_classes: &["mtune-repeat-count-btn"],
                            set_icon_name: "list-add-symbolic",
                            set_tooltip_text: Some("Repeat more times"),
                        update_property: &[gtk::accessible::Property::Label("Repeat more times")],
                            connect_clicked => MtuneMenuInput::AdjustRepeatCount(1),
                        },
                    },
            gtk::Box {
                set_spacing: 8,
                #[watch]
                set_visible: !model.is_live,
                gtk::MenuButton {
                    #[watch]
                    set_label: &format!("Speed {:.2}×", model.rate),
                    set_tooltip_text: Some("Playback speed"),
                    update_property: &[gtk::accessible::Property::Label("Playback speed")],
                    #[watch]
                    set_visible: !model.is_live,
                    #[watch]
                    set_sensitive: model.running,
                    #[wrap(Some)]
                    set_popover = &gtk::Popover {
                        #[name = "speed_row"]
                        gtk::Box {
                            add_css_class: "mtune-menu-speed",
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 4,
                        },
                    },
                },
                gtk::Box {
                    add_css_class: "mtune-menu-toggles",
                #[watch]
                set_visible: !model.is_live,
                    set_hexpand: true,
                    set_halign: gtk::Align::End,
                    set_valign: gtk::Align::Center,
                    set_spacing: 8,

                    #[name = "shuffle_btn"]
                    gtk::ToggleButton {
                        set_css_classes: &["mtune-toggle"],
                        set_icon_name: "media-playlist-shuffle-symbolic",
                        set_tooltip_text: Some("Shuffle"),
                        update_property: &[gtk::accessible::Property::Label("Shuffle")],
                        #[watch]
                        #[block_signal(shuffle_toggled)]
                        set_active: model.shuffle,
                        connect_toggled[sender] => move |_| {
                            sender.input(MtuneMenuInput::ToggleShuffle);
                        } @shuffle_toggled,
                    },
                    #[name = "repeat_btn"]
                    gtk::Button {
                        set_css_classes: &["mtune-toggle"],
                        #[watch]
                        set_icon_name: match model.repeat.as_str() {
                            "repeat-one" | "repeat-each" => "media-playlist-repeat-song-symbolic",
                            "repeat-all" => "media-playlist-repeat-symbolic",
                            _ => "media-playlist-consecutive-symbolic",
                        },
                        #[watch]
                        set_tooltip_text: Some(&match model.repeat.as_str() {
                            "repeat-one" => "Repeat: one".to_string(),
                            "repeat-all" => "Repeat: all".to_string(),
                            "repeat-each" => format!(
                                "Repeat: each — play {} of {}",
                                model.repeat_plays + 1,
                                model.repeat_count
                            ),
                            _ => "Repeat: off".to_string(),
                        }),
                        connect_clicked => MtuneMenuInput::CycleRepeat,
                    },

                },

            },
            #[name = "tabs_host"]
            gtk::Box {},

            // ── Queue ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-queue-section",
                #[watch]
                set_visible: model.tab == 0,
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                set_vexpand: true,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Queue",
                },

                #[name = "queue_filter_entry"]
                gtk::SearchEntry {
                    add_css_class: mshell_common::autofocus::CLASS,
                    add_css_class: "mtune-queue-filter",
                    set_placeholder_text: Some("Filter queue…"),
                    #[watch]
                    set_visible: model.queue_len > 0,
                    connect_search_changed[sender] => move |e| {
                        sender.input(MtuneMenuInput::FilterQueue(e.text().to_string()));
                    },
                },

                #[name = "queue_scroller"]
                gtk::ScrolledWindow {
                    add_css_class: "mtune-queue-scroller",
                    set_hexpand: true,
                    set_vexpand: true,
                    set_propagate_natural_height: true,
                    set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),
                    #[watch]
                    set_visible: model.queue_len > 0,
                    set_min_content_height: 0,

                    #[name = "queue_rows"]
                    gtk::Box {
                        add_css_class: "mtune-queue-rows",
                        set_orientation: gtk::Orientation::Vertical,
                    },
                },

                gtk::Label {
                    add_css_class: "mtune-menu-status",
                    set_xalign: 0.0,
                    set_label: "Queue is empty — choose a folder or open a playlist.",
                    #[watch]
                    set_visible: model.queue_len == 0,
                },
            },

            #[name = "library_scroller"]
            gtk::ScrolledWindow {
                set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),
                set_propagate_natural_height: true,
                set_min_content_height: 0,
                set_vexpand: true,
                #[watch]
                set_visible: model.tab == 1,
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 16,
            // ── Library ────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Library",
                },
                gtk::Label {
                    add_css_class: "mtune-menu-status",
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.library_status(),
                },
                gtk::Box {
                    set_spacing: 8,
                    gtk::Button {
                        set_css_classes: &["mtune-menu-action"],
                        set_hexpand: true,
                        set_label: "Choose folder…",
                        connect_clicked => MtuneMenuInput::ChooseFolder,
                    },
                    gtk::Button {
                        set_css_classes: &["mtune-menu-action"],
                        set_icon_name: "view-refresh-symbolic",
                        set_tooltip_text: Some("Rescan the library"),
                        update_property: &[gtk::accessible::Property::Label("Rescan the library")],
                        #[watch]
                        set_sensitive: model.running && !model.scanning,
                        connect_clicked => MtuneMenuInput::Rescan,
                    },
                },
            },

            // ── Playlists ──────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Playlists",
                },
                #[name = "playlists_list"]
                gtk::Box {
                    add_css_class: "mtune-menu-playlists",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    #[watch]
                    set_visible: !model.playlists.is_empty(),
                },
                gtk::Button {
                    set_css_classes: &["mtune-menu-action"],
                    set_hexpand: true,
                    set_label: "Open playlist file…",
                    connect_clicked => MtuneMenuInput::OpenPlaylist,
                },
            },                },
            },
            #[name = "radio_host"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_vexpand: true,
                #[watch]
                set_visible: model.tab == 2,
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // One command watching every `org.margo.Tune` property the menu
        // renders; each wake re-reads the lot and re-renders.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let p = mtune_service().player.clone();
            let mut streams: Vec<std::pin::Pin<Box<dyn futures::Stream<Item = ()> + Send>>> = vec![
                Box::pin(p.radio_stations.watch().map(|_| ())),
                Box::pin(p.is_live.watch().map(|_| ())),
                Box::pin(p.station_name.watch().map(|_| ())),
                Box::pin(p.stream_error.watch().map(|_| ())),
                Box::pin(p.volume.watch().map(|_| ())),
                Box::pin(p.running.watch().map(|_| ())),
                Box::pin(p.playing.watch().map(|_| ())),
                Box::pin(p.has_song.watch().map(|_| ())),
                Box::pin(p.title.watch().map(|_| ())),
                Box::pin(p.artist.watch().map(|_| ())),
                Box::pin(p.album.watch().map(|_| ())),
                Box::pin(p.cover_art.watch().map(|_| ())),
                Box::pin(p.shuffle.watch().map(|_| ())),
                Box::pin(p.repeat_mode.watch().map(|_| ())),
                Box::pin(p.rate.watch().map(|_| ())),
                Box::pin(p.repeat_count.watch().map(|_| ())),
                Box::pin(p.repeat_plays.watch().map(|_| ())),
                Box::pin(p.queue_len.watch().map(|_| ())),
                Box::pin(p.current_index.watch().map(|_| ())),
                Box::pin(p.queue_entries.watch().map(|_| ())),
                Box::pin(p.library_roots.watch().map(|_| ())),
                Box::pin(p.playlists.watch().map(|_| ())),
                Box::pin(p.scanning.watch().map(|_| ())),
                Box::pin(p.scan_progress.watch().map(|_| ())),
                Box::pin(p.duration.watch().map(|_| ())),
            ];
            let mut merged = futures::stream::select_all(streams.drain(..));
            let mut position = p.position.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = merged.next() => {
                        if next.is_none() {
                            break;
                        }
                        let _ = out.send(MtuneMenuCmd::Refresh);
                    }
                    Some(d) = position.next() => {
                        let _ = out.send(MtuneMenuCmd::PositionTick(d));
                    }
                }
            }
        });

        let mut model = MtuneMenuWidgetModel {
            tab: 0,
            is_live: false,
            station_name: String::new(),
            stream_error: String::new(),
            radio_stations: Vec::new(),
            radio: RadioState::default(),
            radio_busy: false,
            radio_message: String::new(),
            radio_panel: None,
            volume: 1.0,
            volume_signal: None,
            running: false,
            playing: false,
            has_song: false,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            cover_art: None,
            shuffle: false,
            repeat: "consecutive".into(),
            repeat_count: 3,
            repeat_plays: 0,
            rate: 1.0,
            queue_len: 0,
            current_index: -1,
            queue_entries: Vec::new(),
            queue_filter: String::new(),
            roots: Vec::new(),
            playlists: Vec::new(),
            scanning: false,
            scan_progress: (0, 0),
            position: Duration::ZERO,
            duration: Duration::ZERO,
            pending_seek: None,
            seek_signal: None,
        };
        read(&mut model);

        let widgets = view_output!();

        let s = sender.clone();
        let header = PanelHeader::new("org.margo.Tune-symbolic", "Tune").action(
            "folder-open-symbolic",
            "Choose a music folder",
            move || s.input(MtuneMenuInput::ChooseFolder),
        );
        let s = sender.clone();
        let header = header.action("go-next-symbolic", "Open or launch Tune", move || {
            s.input(if mtune_service().player.running.get() {
                MtuneMenuInput::OpenTune
            } else {
                MtuneMenuInput::Launch
            });
        });
        widgets.header_host.append(header.root());
        let s = sender.clone();
        let tabs = Segmented::new(
            &["Queue", "Library", "Radio"],
            0,
            SegmentedStyle::Capsule,
            move |i| s.input(MtuneMenuInput::Tab(i)),
        );
        widgets.tabs_host.append(tabs.root());
        // Classes are updated by the same model-owned selection on every change.
        model.radio_panel = Some(radio::RadioPanel::new(&widgets.radio_host, &sender));
        list_fit::mark_menu_list(&widgets.queue_scroller, 320);
        list_fit::mark_menu_list(&widgets.library_scroller, 320);
        let s = sender.clone();
        model.volume_signal =
            Some(widgets.volume.connect_value_changed(move |scale| {
                s.input(MtuneMenuInput::SetVolume(scale.value()))
            }));
        rebuild_playlists(&widgets, &model, &sender);
        queue::keyboard_rows(&widgets.queue_rows);
        model.seek_signal = Some(setup_seek(&widgets.seek, &sender));

        // Build the speed-preset row once; state (highlight) is applied
        // on every refresh.
        for (rate, label) in RATE_PRESETS {
            let btn = gtk::Button::builder()
                .label(label)
                .css_classes(["mtune-speed-preset"])
                .build();
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(MtuneMenuInput::SetRate(rate)));
            widgets.speed_row.append(&btn);
        }

        apply_dynamic(&widgets, &model, &sender);
        rebuild_queue_rows(&widgets, &model, &sender);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        actions::handle(self, widgets, message, &sender);
        apply_dynamic(widgets, self, &sender);
        // relm4 doesn't re-run `#[watch]` after `update_with_view` — do it.
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
            MtuneMenuCmd::SearchResult(generation, result) => {
                self.radio.complete(generation, result)
            }
            MtuneMenuCmd::RadioFinished(result) => {
                self.radio_busy = false;
                self.radio_message = result.err().unwrap_or_default();
            }
            MtuneMenuCmd::Refresh => {
                let old_queue = (self.queue_entries.clone(), self.current_index, self.is_live);
                let old_playlists = self.playlists.clone();
                read(self);
                if old_playlists != self.playlists {
                    rebuild_playlists(widgets, self, &sender);
                }
                // A fresh song / stop clears any in-flight seek.
                self.pending_seek = None;
                if old_queue != (self.queue_entries.clone(), self.current_index, self.is_live) {
                    rebuild_queue_rows(widgets, self, &sender);
                }
            }
            MtuneMenuCmd::PositionTick(d) => {
                // Ignore ticks while a seek is settling.
                let settling = self
                    .pending_seek
                    .is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(1200));
                if !settling {
                    self.pending_seek = None;
                    self.position = d;
                }
            }
        }
        apply_dynamic(widgets, self, &sender);
        // CRITICAL: re-run the `#[watch]` bindings (title, icons,
        // sensitivity, …). relm4 does *not* do this automatically after
        // `update_cmd_with_view` — every other menu widget calls it too.
        self.update_view(widgets, sender);
    }
}
