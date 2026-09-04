//! Tune menu — the dedicated panel for the `mtune` folder-first music
//! player: now-playing, transport, speed, and the library / playlist
//! controls the generic MPRIS media menu can't offer. Talks only to
//! `mtune_service()` (→ `org.margo.Tune`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_services::mtune::{mtune_service, spawn_mtune};
use mshell_services::tokio_rt_spawn;
use mshell_utils::media::format_duration;
use reactive_graph::traits::Get;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

const RATE_PRESETS: [(f64, &str); 5] = [
    (0.75, "0.75×"),
    (1.0, "1×"),
    (1.25, "1.25×"),
    (1.5, "1.5×"),
    (2.0, "2×"),
];

pub(crate) struct MtuneMenuWidgetModel {
    running: bool,
    playing: bool,
    has_song: bool,
    title: String,
    artist: String,
    album: String,
    cover_art: Option<String>,
    shuffle: bool,
    /// `"consecutive"` / `"repeat-all"` / `"repeat-one"`.
    repeat: String,
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
    PlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,
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
            set_spacing: 14,
            set_hexpand: true,

            // ── Panel header (DESIGN.md §12) ────────────────────
            // Hand-rolled (like Clipboard) rather than the composed
            // MenuWidget::PanelHeader, since this is one monolithic
            // component, not a widget-list menu. Reuses the *generic*
            // panel-header classes (panel_header.rs / the Dashboard
            // header), not per-widget-prefixed ones.
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("org.margo.Tune-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_xalign: 0.0,
                    set_hexpand: true,
                    set_label: "Tune",
                },
                gtk::Label {
                    add_css_class: "panel-header-meta",
                    #[watch]
                    set_label: &model.queue_count_meta(),
                    #[watch]
                    set_visible: model.running,
                },
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "folder-open-symbolic",
                    set_tooltip_text: Some("Choose a music folder"),
                    connect_clicked => MtuneMenuInput::ChooseFolder,
                },
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_icon_name: if model.running { "go-next-symbolic" } else { "media-playback-start-symbolic" },
                    #[watch]
                    set_tooltip_text: Some(if model.running { "Open Tune window" } else { "Launch Tune" }),
                    connect_clicked[sender] => move |_| {
                        sender.input(if mtune_service().player.running.get() {
                            MtuneMenuInput::OpenTune
                        } else {
                            MtuneMenuInput::Launch
                        });
                    },
                },
            },

            // ── Now playing ────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-hero",
                set_spacing: 12,

                #[name = "cover"]
                gtk::Image {
                    add_css_class: "mtune-menu-cover",
                    set_pixel_size: 88,
                    set_valign: gtk::Align::Start,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,
                    set_spacing: 2,
                    gtk::Label {
                        add_css_class: "mtune-menu-title",
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                        #[watch]
                        set_label: if model.has_song {
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
                        set_visible: model.running && (model.has_song || model.queue_len > 0),
                    },
                },
            },

            // ── Seek bar ───────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-progress",
                set_spacing: 8,
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: model.running && model.has_song && !model.duration.is_zero(),

                #[name = "elapsed"]
                gtk::Label { add_css_class: "mtune-menu-time" },

                #[name = "seek"]
                gtk::Scale {
                    add_css_class: "ok-progress-bar",
                    set_hexpand: true,
                    set_can_focus: false,
                    set_focus_on_click: false,
                    set_range: (0.0, 1.0),
                },

                #[name = "total"]
                gtk::Label { add_css_class: "mtune-menu-time" },
            },

            // ── Controls (transport + shuffle/repeat + speed) ────
            gtk::Box {
                add_css_class: "mtune-menu-controls",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                set_halign: gtk::Align::Fill,
                #[watch]
                set_sensitive: model.running,

                gtk::Box {
                    add_css_class: "mtune-menu-transport",
                    set_halign: gtk::Align::Start,
                    set_spacing: 8,

                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-backward-symbolic",
                        set_tooltip_text: Some("Previous"),
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
                        #[watch]
                        set_sensitive: model.running && (model.has_song || model.queue_len > 0),
                        connect_clicked => MtuneMenuInput::PlayPause,
                    },
                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-forward-symbolic",
                        set_tooltip_text: Some("Next"),
                        #[watch]
                        set_sensitive: model.running && model.queue_len > 1,
                        connect_clicked => MtuneMenuInput::Next,
                    },
                },

                gtk::Box {
                    add_css_class: "mtune-menu-toggles",
                    set_hexpand: true,
                    set_halign: gtk::Align::End,
                    set_spacing: 6,

                    #[name = "shuffle_btn"]
                    gtk::ToggleButton {
                        set_css_classes: &["mtune-toggle"],
                        set_icon_name: "media-playlist-shuffle-symbolic",
                        set_tooltip_text: Some("Shuffle"),
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
                            "repeat-one" => "media-playlist-repeat-song-symbolic",
                            "repeat-all" => "media-playlist-repeat-symbolic",
                            _ => "media-playlist-consecutive-symbolic",
                        },
                        #[watch]
                        set_tooltip_text: Some(match model.repeat.as_str() {
                            "repeat-one" => "Repeat: one",
                            "repeat-all" => "Repeat: all",
                            _ => "Repeat: off",
                        }),
                        connect_clicked => MtuneMenuInput::CycleRepeat,
                    },
                },

                #[name = "speed_row"]
                gtk::Box {
                    add_css_class: "mtune-menu-speed",
                    set_spacing: 4,
                },
            },

            // ── Queue ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-queue-section",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_vexpand: true,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Queue",
                },

                #[name = "queue_filter_entry"]
                gtk::SearchEntry {
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
                    #[watch]
                    set_max_content_height: {
                        let h = config_manager()
                            .config()
                            .menus()
                            .mtune_menu()
                            .maximum_height()
                            .get();
                        if h > 0 { h } else { -1 }
                    },

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

            // ── Library ────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 5,

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
                    set_spacing: 6,
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
                        #[watch]
                        set_sensitive: model.running && !model.scanning,
                        connect_clicked => MtuneMenuInput::Rescan,
                    },
                },
            },

            // ── Playlists ──────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 5,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Playlists",
                },
                #[name = "playlists_list"]
                gtk::Box {
                    add_css_class: "mtune-menu-playlists",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    #[watch]
                    set_visible: !model.playlists.is_empty(),
                },
                gtk::Button {
                    set_css_classes: &["mtune-menu-action"],
                    set_hexpand: true,
                    set_label: "Open playlist file…",
                    connect_clicked => MtuneMenuInput::OpenPlaylist,
                },
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
            running: false,
            playing: false,
            has_song: false,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            cover_art: None,
            shuffle: false,
            repeat: "consecutive".into(),
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
        match message {
            MtuneMenuInput::SeekPreview(value) => {
                // Immediate feedback; the real seek is debounced.
                let pos = self.duration.mul_f64(value.clamp(0.0, 1.0));
                self.pending_seek = Some((pos, Instant::now()));
                self.position = pos;
                widgets.elapsed.set_label(&format_duration(pos));
                if let Some(sig) = &self.seek_signal {
                    widgets.seek.block_signal(sig);
                    widgets.seek.set_value(value);
                    widgets.seek.unblock_signal(sig);
                }
            }
            MtuneMenuInput::Seek(value) => {
                let secs = self.duration.mul_f64(value.clamp(0.0, 1.0)).as_secs();
                tokio_rt_spawn(async move { mtune_service().player.seek(secs).await });
            }
            MtuneMenuInput::PlayPause => {
                tokio_rt_spawn(async { mtune_service().player.play_pause().await });
            }
            MtuneMenuInput::Next => {
                tokio_rt_spawn(async { mtune_service().player.next().await });
            }
            MtuneMenuInput::Previous => {
                tokio_rt_spawn(async { mtune_service().player.previous().await });
            }
            MtuneMenuInput::ToggleShuffle => {
                let on = !self.shuffle;
                tokio_rt_spawn(async move { mtune_service().player.set_shuffle(on).await });
            }
            MtuneMenuInput::CycleRepeat => {
                let next = match self.repeat.as_str() {
                    "consecutive" => "repeat-all",
                    "repeat-all" => "repeat-one",
                    _ => "consecutive",
                };
                tokio_rt_spawn(async move { mtune_service().player.set_repeat_mode(next).await });
            }
            MtuneMenuInput::SetRate(r) => {
                tokio_rt_spawn(async move { mtune_service().player.set_rate(r).await });
            }
            MtuneMenuInput::LoadPlaylist(name) => {
                tokio_rt_spawn(async move { mtune_service().player.load_playlist(&name).await });
            }
            MtuneMenuInput::OpenPlaylist => {
                // Parent must be `None`: a layer-shell menu surface has no
                // xdg_toplevel, so handing it to the file-chooser as a
                // parent aborts GTK (crashing the shell).
                let dialog = gtk::FileDialog::builder()
                    .title("Open a playlist")
                    .modal(true)
                    .build();
                dialog.open(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res
                        && let Some(path) = file.path()
                    {
                        let path = path.to_string_lossy().into_owned();
                        tokio_rt_spawn(async move {
                            let svc = mtune_service().player.clone();
                            if !svc.running.get() {
                                spawn_mtune();
                            }
                            svc.open_playlist(&path).await;
                        });
                    }
                });
            }
            MtuneMenuInput::Rescan => {
                tokio_rt_spawn(async { mtune_service().player.rescan_library().await });
            }
            MtuneMenuInput::OpenTune => {
                tokio_rt_spawn(async { mtune_service().player.raise().await });
            }
            MtuneMenuInput::Launch => spawn_mtune(),
            MtuneMenuInput::ChooseFolder => {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose a music folder")
                    .modal(true)
                    .build();
                dialog.select_folder(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(folder) = res
                        && let Some(path) = folder.path()
                    {
                        let path = path.to_string_lossy().into_owned();
                        tokio_rt_spawn(async move {
                            let svc = mtune_service().player.clone();
                            if !svc.running.get() {
                                spawn_mtune();
                            }
                            svc.set_library_roots(vec![path.clone()]).await;
                            svc.play_folder(&path).await;
                        });
                    }
                });
            }
            MtuneMenuInput::FilterQueue(text) => {
                self.queue_filter = text;
                rebuild_queue_rows(widgets, self, &sender);
            }
            MtuneMenuInput::PlayQueueIndex(i) => {
                tokio_rt_spawn(async move { mtune_service().player.play_index(i).await });
            }
            MtuneMenuInput::RemoveQueueIndex(i) => {
                tokio_rt_spawn(async move { mtune_service().player.remove_index(i).await });
            }
        }
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
            MtuneMenuCmd::Refresh => {
                read(self);
                // A fresh song / stop clears any in-flight seek.
                self.pending_seek = None;
                rebuild_queue_rows(widgets, self, &sender);
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

/// Whether `title`/`artist` should show under `query` (case-insensitive
/// substring on either field; a blank query matches everything).
fn queue_row_matches(title: &str, artist: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&q) || artist.to_lowercase().contains(&q)
}

fn read(m: &mut MtuneMenuWidgetModel) {
    let p = mtune_service().player.clone();
    m.running = p.running.get();
    m.playing = p.playing.get();
    m.has_song = p.has_song.get();
    m.title = p.title.get();
    m.artist = p.artist.get();
    m.album = p.album.get();
    m.cover_art = p.cover_art.get();
    m.shuffle = p.shuffle.get();
    m.repeat = p.repeat_mode.get();
    m.rate = p.rate.get();
    m.queue_len = p.queue_len.get();
    m.current_index = p.current_index.get();
    m.queue_entries = p.queue_entries.get();
    m.roots = p.library_roots.get();
    m.playlists = p.playlists.get();
    m.scanning = p.scanning.get();
    m.scan_progress = p.scan_progress.get();
    m.position = p.position.get();
    m.duration = p.duration.get();
}

/// Widget updates the `#[watch]` macro can't express: the cover image,
/// the repeat/speed highlight, and the saved-playlist rows.
fn apply_dynamic(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    // Cover
    match m.cover_art.as_deref() {
        Some(path) if !path.trim().is_empty() => widgets.cover.set_from_file(Some(path)),
        _ => widgets.cover.set_icon_name(Some("org.margo.Tune-symbolic")),
    }

    // Seek bar + time readouts
    let frac = if m.duration.is_zero() {
        0.0
    } else {
        (m.position.as_secs_f64() / m.duration.as_secs_f64()).clamp(0.0, 1.0)
    };
    match &m.seek_signal {
        Some(sig) => {
            widgets.seek.block_signal(sig);
            widgets.seek.set_value(frac);
            widgets.seek.unblock_signal(sig);
        }
        None => widgets.seek.set_value(frac),
    }
    widgets.elapsed.set_label(&format_duration(m.position));
    widgets.total.set_label(&format_duration(m.duration));

    // Repeat active state
    if m.repeat == "consecutive" {
        widgets.repeat_btn.remove_css_class("selected");
    } else {
        widgets.repeat_btn.add_css_class("selected");
    }

    // Speed preset highlight
    let mut child = widgets.speed_row.first_child();
    let mut i = 0;
    while let Some(w) = child {
        child = w.next_sibling();
        if let Some((rate, _)) = RATE_PRESETS.get(i) {
            if (m.rate - rate).abs() < 0.01 {
                w.add_css_class("selected");
            } else {
                w.remove_css_class("selected");
            }
        }
        i += 1;
    }

    // Saved-playlist rows
    while let Some(c) = widgets.playlists_list.first_child() {
        widgets.playlists_list.remove(&c);
    }
    for name in &m.playlists {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        let row = gtk::Button::builder()
            .child(&label)
            .css_classes(["mtune-playlist-row"])
            .build();
        let name = name.clone();
        let s = sender.clone();
        row.connect_clicked(move |_| s.input(MtuneMenuInput::LoadPlaylist(name.clone())));
        widgets.playlists_list.append(&row);
    }
}

/// Rebuild the queue row list from `m.queue_entries`, filtered by
/// `m.queue_filter`. Called after every refresh and every filter
/// keystroke — the list is small enough (a folder-first personal
/// library queue, not a virtualized thousand-row history) that a full
/// rebuild is simpler and cheap enough, same call shape as the
/// existing saved-playlist rows.
fn rebuild_queue_rows(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    while let Some(c) = widgets.queue_rows.first_child() {
        widgets.queue_rows.remove(&c);
    }

    let mut current_row: Option<gtk::Widget> = None;
    for (i, (title, artist, duration)) in m.queue_entries.iter().enumerate() {
        if !queue_row_matches(title, artist, &m.queue_filter) {
            continue;
        }
        let idx = i as u32;
        let is_current = m.current_index >= 0 && m.current_index as usize == i;

        let row = gtk::Box::builder()
            .css_classes(["mtune-queue-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        if is_current {
            row.add_css_class("mtune-queue-row-current");
        }

        let num = gtk::Label::new(Some(&(i + 1).to_string()));
        num.add_css_class("mtune-queue-row-num");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title_label = gtk::Label::new(Some(title));
        title_label.set_xalign(0.0);
        title_label.set_ellipsize(pango::EllipsizeMode::End);
        title_label.add_css_class("mtune-queue-row-title");
        let artist_label = gtk::Label::new(Some(artist));
        artist_label.set_xalign(0.0);
        artist_label.set_ellipsize(pango::EllipsizeMode::End);
        artist_label.add_css_class("mtune-queue-row-artist");
        text.append(&title_label);
        text.append(&artist_label);
        text.set_hexpand(true);

        let dur = gtk::Label::new(Some(&format_duration(Duration::from_secs(*duration))));
        dur.add_css_class("mtune-queue-row-duration");

        let remove_btn = gtk::Button::builder()
            .css_classes(["mtune-queue-row-remove"])
            .icon_name("window-close-symbolic")
            .tooltip_text("Remove from queue")
            .valign(gtk::Align::Center)
            .build();
        let s = sender.clone();
        remove_btn.connect_clicked(move |_| s.input(MtuneMenuInput::RemoveQueueIndex(idx)));

        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_released(move |_, _, _, _| s.input(MtuneMenuInput::PlayQueueIndex(idx)));
        row.add_controller(click);

        row.append(&num);
        row.append(&text);
        row.append(&dur);
        row.append(&remove_btn);
        widgets.queue_rows.append(&row);

        if is_current {
            current_row = Some(row.upcast::<gtk::Widget>());
        }
    }

    if let Some(row) = current_row {
        scroll_queue_to(&widgets.queue_scroller, &row);
    }
}

/// Smoothly centre `row` in `scroller`. Deferred to idle so its geometry
/// is laid out (freshly appended on a rebuild) — same technique as the
/// Lyrics menu's `scroll_center`.
fn scroll_queue_to(scroller: &gtk::ScrolledWindow, row: &gtk::Widget) {
    let scroller = scroller.clone();
    let row = row.clone();
    gtk::glib::idle_add_local_once(move || {
        let Some(parent) = row.parent() else { return };
        let Some(bounds) = row.compute_bounds(&parent) else {
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

/// Wire the seek scale: immediate readout on drag, the real seek
/// debounced ~280ms after the last move. Returns the `value-changed`
/// handler id so programmatic updates can block it.
fn setup_seek(
    scale: &gtk::Scale,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) -> gtk::glib::SignalHandlerId {
    let pending: Rc<Cell<Option<gtk::glib::SourceId>>> = Rc::new(Cell::new(None));
    let sender = sender.clone();
    scale.connect_value_changed(move |scale| {
        if let Some(id) = pending.take() {
            id.remove();
        }
        let value = scale.value();
        // Fallible send: `value-changed` also fires on a programmatic
        // `set_value` during teardown, where `input()` would panic.
        let _ = sender
            .input_sender()
            .send(MtuneMenuInput::SeekPreview(value));

        let s = sender.clone();
        let p = pending.clone();
        let id = gtk::glib::timeout_add_local_once(Duration::from_millis(280), move || {
            p.set(None);
            let _ = s.input_sender().send(MtuneMenuInput::Seek(value));
        });
        pending.set(Some(id));
    })
}

impl MtuneMenuWidgetModel {
    /// "12 songs" for the header's trailing meta; empty when there's no
    /// queue yet (header hides it via `set_visible` in that case, but an
    /// empty string is the harmless fallback either way).
    fn queue_count_meta(&self) -> String {
        if self.queue_len == 0 {
            String::new()
        } else {
            format!("{} songs", self.queue_len)
        }
    }

    /// "3 of 240 · 1.5×" — position in the queue plus a non-default speed.
    fn now_playing_meta(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.queue_len > 0 {
            let n = if self.current_index >= 0 {
                self.current_index as u32 + 1
            } else {
                0
            };
            parts.push(format!("{n} of {}", self.queue_len));
        }
        if (self.rate - 1.0).abs() >= 0.01 {
            parts.push(format!("{:.2}×", self.rate));
        }
        parts.join("  ·  ")
    }

    fn library_status(&self) -> String {
        if !self.running {
            return "Tune isn't running.".into();
        }
        if self.scanning {
            let (done, total) = self.scan_progress;
            return if total > 0 {
                format!("Scanning… {done}/{total}")
            } else {
                "Scanning…".into()
            };
        }
        let roots = if self.roots.is_empty() {
            "no folder set".to_string()
        } else {
            self.roots.join(", ")
        };
        format!("{} tracks  ·  {roots}", self.queue_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_on_title_or_artist_case_insensitively() {
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "lucky"));
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "DAFT"));
        assert!(!queue_row_matches("Get Lucky", "Daft Punk", "acoustic"));
    }

    #[test]
    fn blank_query_matches_everything() {
        assert!(queue_row_matches("Anything", "Anyone", ""));
        assert!(queue_row_matches("Anything", "Anyone", "   "));
    }
}
