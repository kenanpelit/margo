//! Tune menu — the dedicated panel for the `mtune` folder-first music
//! player. Now-playing + transport + the library controls (folder picker,
//! scan status, rescan) that the generic MPRIS media menu can't offer.
//! Talks only to `mtune_service()` (→ `org.margo.Tune`).

use futures::StreamExt;
use mshell_services::mtune::{mtune_service, spawn_mtune};
use mshell_services::tokio_rt_spawn;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct MtuneMenuWidgetModel {
    running: bool,
    playing: bool,
    has_song: bool,
    title: String,
    artist: String,
    album: String,
    cover_art: Option<String>,
    shuffle: bool,
    repeat: String,
    queue_len: u32,
    roots: Vec<String>,
    scanning: bool,
    scan_progress: (u32, u32),
}

#[derive(Debug)]
pub(crate) enum MtuneMenuInput {
    PlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,
    ChooseFolder,
    Rescan,
    OpenTune,
    Launch,
}

#[derive(Debug)]
pub(crate) enum MtuneMenuOutput {}

pub(crate) struct MtuneMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum MtuneMenuCmd {
    Refresh,
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
            set_spacing: 12,
            set_hexpand: true,

            // ── Now playing ────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-hero",
                set_spacing: 12,
                #[watch]
                set_visible: model.running,

                #[name = "cover"]
                gtk::Image {
                    add_css_class: "mtune-menu-cover",
                    set_pixel_size: 64,
                    set_valign: gtk::Align::Center,
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
                        set_label: if model.has_song { model.title.as_str() } else { "Nothing playing" },
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
                        add_css_class: "mtune-menu-album",
                        set_xalign: 0.0,
                        set_ellipsize: pango::EllipsizeMode::End,
                        #[watch]
                        set_visible: !model.album.is_empty(),
                        #[watch]
                        set_label: &model.album,
                    },
                },
            },

            // ── Transport ──────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-transport",
                set_halign: gtk::Align::Center,
                set_spacing: 8,
                #[watch]
                set_visible: model.running,

                gtk::Button {
                    set_css_classes: &["ok-button-flat", "circular"],
                    set_icon_name: "media-skip-backward-symbolic",
                    #[watch]
                    set_sensitive: model.queue_len > 1,
                    connect_clicked => MtuneMenuInput::Previous,
                },
                #[name = "play_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-primary", "circular"],
                    #[watch]
                    set_icon_name: if model.playing {
                        "media-playback-pause-symbolic"
                    } else {
                        "media-playback-start-symbolic"
                    },
                    #[watch]
                    set_sensitive: model.has_song,
                    connect_clicked => MtuneMenuInput::PlayPause,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-flat", "circular"],
                    set_icon_name: "media-skip-forward-symbolic",
                    #[watch]
                    set_sensitive: model.queue_len > 1,
                    connect_clicked => MtuneMenuInput::Next,
                },
            },

            // ── Shuffle / repeat ───────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-toggles",
                set_halign: gtk::Align::Center,
                set_spacing: 8,
                #[watch]
                set_visible: model.running,

                #[name = "shuffle_btn"]
                gtk::ToggleButton {
                    set_css_classes: &["ok-button-flat"],
                    set_icon_name: "media-playlist-shuffle-symbolic",
                    set_tooltip_text: Some("Shuffle"),
                    #[watch]
                    #[block_signal(shuffle_toggled)]
                    set_active: model.shuffle,
                    connect_toggled[sender] => move |_| {
                        sender.input(MtuneMenuInput::ToggleShuffle);
                    } @shuffle_toggled,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-flat"],
                    #[watch]
                    set_icon_name: match model.repeat.as_str() {
                        "repeat-one" => "media-playlist-repeat-song-symbolic",
                        "repeat-all" => "media-playlist-repeat-symbolic",
                        _ => "media-playlist-consecutive-symbolic",
                    },
                    set_tooltip_text: Some("Repeat: off / all / one"),
                    connect_clicked => MtuneMenuInput::CycleRepeat,
                },
            },

            // ── Library ────────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-menu-library",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,

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
                        set_css_classes: &["ok-button-surface"],
                        set_hexpand: true,
                        set_label: "Choose folder…",
                        connect_clicked => MtuneMenuInput::ChooseFolder,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface"],
                        set_icon_name: "view-refresh-symbolic",
                        set_tooltip_text: Some("Rescan the library"),
                        #[watch]
                        set_sensitive: model.running && !model.scanning,
                        connect_clicked => MtuneMenuInput::Rescan,
                    },
                },
            },

            // ── Footer ─────────────────────────────────────────
            gtk::Button {
                set_css_classes: &["ok-button-surface"],
                #[watch]
                set_label: if model.running { "Open Tune" } else { "Launch Tune" },
                connect_clicked[sender] => move |_| {
                    sender.input(if mtune_service().player.running.get() {
                        MtuneMenuInput::OpenTune
                    } else {
                        MtuneMenuInput::Launch
                    });
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let p = mtune_service().player.clone();
            let mut running = p.running.watch();
            let mut playing = p.playing.watch();
            let mut has_song = p.has_song.watch();
            let mut title = p.title.watch();
            let mut artist = p.artist.watch();
            let mut album = p.album.watch();
            let mut cover = p.cover_art.watch();
            let mut shuffle = p.shuffle.watch();
            let mut repeat = p.repeat_mode.watch();
            let mut qlen = p.queue_len.watch();
            let mut roots = p.library_roots.watch();
            let mut scanning = p.scanning.watch();
            let mut progress = p.scan_progress.watch();
            loop {
                let woke = tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = running.next() => true,
                    _ = playing.next() => true,
                    _ = has_song.next() => true,
                    _ = title.next() => true,
                    _ = artist.next() => true,
                    _ = album.next() => true,
                    _ = cover.next() => true,
                    _ = shuffle.next() => true,
                    _ = repeat.next() => true,
                    _ = qlen.next() => true,
                    _ = roots.next() => true,
                    _ = scanning.next() => true,
                    _ = progress.next() => true,
                };
                if woke {
                    let _ = out.send(MtuneMenuCmd::Refresh);
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
            queue_len: 0,
            roots: Vec::new(),
            scanning: false,
            scan_progress: (0, 0),
        };
        read(&mut model);

        let widgets = view_output!();
        apply_cover(&widgets, &model);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
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
            MtuneMenuInput::Rescan => {
                tokio_rt_spawn(async { mtune_service().player.rescan_library().await });
            }
            MtuneMenuInput::OpenTune => {
                tokio_rt_spawn(async { mtune_service().player.raise().await });
            }
            MtuneMenuInput::Launch => spawn_mtune(),
            MtuneMenuInput::ChooseFolder => {
                // Parent must be `None`: a layer-shell menu surface has no
                // xdg_toplevel, so handing it to the file-chooser as a
                // parent aborts GTK (crashing the shell). The wallpaper
                // and Valent menus pick folders the same way.
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
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MtuneMenuCmd::Refresh => read(self),
        }
        apply_cover(widgets, self);
    }
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
    m.queue_len = p.queue_len.get();
    m.roots = p.library_roots.get();
    m.scanning = p.scanning.get();
    m.scan_progress = p.scan_progress.get();
}

fn apply_cover(widgets: &MtuneMenuWidgetModelWidgets, m: &MtuneMenuWidgetModel) {
    match m.cover_art.as_deref() {
        Some(path) if !path.trim().is_empty() => widgets.cover.set_from_file(Some(path)),
        _ => widgets.cover.set_icon_name(Some("org.margo.Tune-symbolic")),
    }
}

impl MtuneMenuWidgetModel {
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
