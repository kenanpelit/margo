//! Tune bar pill — the dedicated entry point for the `mtune` folder-first
//! music player. Distinct from the generic `MediaPlayer` MPRIS pill: it
//! mirrors *only* `mtune` (via `mtune_service()` → `org.margo.Tune`) and
//! its menu carries the library / folder-picker controls MPRIS can't
//! express.
//!
//!   * left click  → toggle the `MenuType::Mtune` panel.
//!   * right click → play / pause in place.
//!   * when mtune isn't running → a single glyph; click launches it.

use std::time::Duration;

use futures::StreamExt;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, TuneBarWidgetStoreFields,
};
use mshell_services::mtune::{mtune_service, spawn_mtune};
use mshell_services::tokio_rt_spawn;
use mshell_utils::media::format_duration;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Live snapshot of `bars.widgets.mtune` — which pill elements show, and how
/// large the cover art / title read. See [`read_bar_config`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct TuneBarConfig {
    show_cover: bool,
    show_track_info: bool,
    show_queue_number: bool,
    show_time: bool,
    show_transport: bool,
    show_repeat_badge: bool,
    show_progress_bar: bool,
    show_playlist_remaining: bool,
    cover_size_px: i32,
    label_max_width_chars: i32,
}

pub(crate) struct MtuneModel {
    running: bool,
    playing: bool,
    has_song: bool,
    title: String,
    artist: String,
    cover_art: Option<String>,
    position: Duration,
    duration: Duration,
    /// 0-based queue position, `-1` when nothing is current.
    current_index: i64,
    queue_len: u32,
    /// `"consecutive"` / `"repeat-all"` / `"repeat-one"` / `"repeat-each"`.
    repeat: String,
    /// Configured N for `"repeat-each"`.
    repeat_count: u32,
    /// Time left across the whole queue (every track after this one, plus
    /// what's left of this one) — see `MtunePlayer::playlist_progress`.
    playlist_remaining: Duration,
    /// Live from `bars.widgets.mtune` — toggling in Settings updates the pill
    /// without a restart.
    bar_cfg: TuneBarConfig,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MtuneInput {
    Clicked,
    PlayPauseClicked,
    PreviousClicked,
    NextClicked,
}

#[derive(Debug)]
pub(crate) enum MtuneOutput {
    Clicked,
}

pub(crate) struct MtuneInit {}

#[derive(Debug)]
pub(crate) enum MtuneCommandOutput {
    /// Any watched `org.margo.Tune` property moved — re-read the lot.
    Refresh,
    /// Any `bars.widgets.mtune` field changed in Settings.
    ConfigChanged(TuneBarConfig),
}

#[relm4::component(pub)]
impl Component for MtuneModel {
    type CommandOutput = MtuneCommandOutput;
    type Input = MtuneInput;
    type Output = MtuneOutput;
    type Init = MtuneInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_css_classes: &["mtune-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 0,

                #[name = "button"]
                gtk::Button {
                    set_css_classes: &["ok-button-flat"],
                    set_hexpand: true,
                    set_vexpand: true,
                    connect_clicked[sender] => move |_| {
                        sender.input(MtuneInput::Clicked);
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,

                        #[name = "cover"]
                        gtk::Image {
                            add_css_class: "mtune-bar-cover",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },

                        // Queue position (1-based), like the playlist rows.
                        #[name = "num"]
                        gtk::Label {
                            add_css_class: "mtune-bar-num",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },

                        #[name = "label"]
                        gtk::Label {
                            add_css_class: "mtune-bar-label",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                            set_ellipsize: pango::EllipsizeMode::End,
                        },

                        // Elapsed / total — never ellipsised, so the title
                        // truncates first.
                        #[name = "time"]
                        gtk::Label {
                            add_css_class: "mtune-bar-time",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },

                        // Repeat-each count ("×3") — only when that mode is on.
                        #[name = "repeat_badge"]
                        gtk::Label {
                            add_css_class: "mtune-bar-repeat",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },

                        // Time left across the *whole queue* (not just this
                        // track), e.g. "-12:34".
                        #[name = "queue_remaining"]
                        gtk::Label {
                            add_css_class: "mtune-bar-queue",
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },
                    }
                },

                // ── Transport (prev / play-pause / next) ────────────────
                // Separate buttons, not inside `button` above — clicking
                // them must not also toggle the menu.
                #[name = "prev_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-flat", "circular"],
                    set_valign: gtk::Align::Center,
                    set_icon_name: "media-skip-backward-symbolic",
                    set_tooltip_text: Some("Previous"),
                    connect_clicked[sender] => move |_| {
                        sender.input(MtuneInput::PreviousClicked);
                    },
                },
                #[name = "playpause_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-flat", "circular"],
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(MtuneInput::PlayPauseClicked);
                    },
                },
                #[name = "next_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-flat", "circular"],
                    set_valign: gtk::Align::Center,
                    set_icon_name: "media-skip-forward-symbolic",
                    set_tooltip_text: Some("Next"),
                    connect_clicked[sender] => move |_| {
                        sender.input(MtuneInput::NextClicked);
                    },
                },
            },

            #[name = "progress"]
            gtk::ProgressBar {
                add_css_class: "mtune-bar-progress",
                set_hexpand: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // One command watching every property the pill (and, indirectly,
        // the menu-open decision) cares about; each wake re-reads the lot.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let p = mtune_service().player.clone();
            let mut running = p.running.watch();
            let mut playing = p.playing.watch();
            let mut has_song = p.has_song.watch();
            let mut title = p.title.watch();
            let mut artist = p.artist.watch();
            let mut cover = p.cover_art.watch();
            let mut position = p.position.watch();
            let mut duration = p.duration.watch();
            let mut current_index = p.current_index.watch();
            let mut queue_len = p.queue_len.watch();
            let mut repeat_mode = p.repeat_mode.watch();
            let mut repeat_count = p.repeat_count.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = running.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = playing.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = has_song.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = title.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = artist.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = cover.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = position.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = duration.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = current_index.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = queue_len.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = repeat_mode.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = repeat_count.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                }
            }
        });

        // Track `bars.widgets.mtune` live so toggling a feature or resizing
        // in Settings updates the pill without a restart.
        let mut effects = EffectScope::new();
        let cs = sender.clone();
        effects.push(move |_| {
            let cfg = read_bar_config();
            let _ = cs
                .command_sender()
                .send(MtuneCommandOutput::ConfigChanged(cfg));
        });

        let mut model = MtuneModel {
            running: false,
            playing: false,
            has_song: false,
            title: String::new(),
            artist: String::new(),
            cover_art: None,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            current_index: -1,
            queue_len: 0,
            repeat: "consecutive".into(),
            repeat_count: 3,
            playlist_remaining: Duration::ZERO,
            bar_cfg: read_bar_config_untracked(),
            _effects: effects,
        };
        read(&mut model);

        let widgets = view_output!();

        // Right click → play/pause in place, wherever on the pill.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            toggle_sender.input(MtuneInput::PlayPauseClicked);
        });
        widgets.root.add_controller(gesture);

        apply(&widgets, &model);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            MtuneInput::Clicked => {
                if self.running {
                    let _ = sender.output(MtuneOutput::Clicked);
                } else {
                    spawn_mtune();
                }
            }
            MtuneInput::PlayPauseClicked => {
                if self.running {
                    tokio_rt_spawn(async move {
                        mtune_service().player.play_pause().await;
                    });
                } else {
                    spawn_mtune();
                }
            }
            MtuneInput::PreviousClicked => {
                if self.running {
                    tokio_rt_spawn(async move {
                        mtune_service().player.previous().await;
                    });
                }
            }
            MtuneInput::NextClicked => {
                if self.running {
                    tokio_rt_spawn(async move {
                        mtune_service().player.next().await;
                    });
                }
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
            MtuneCommandOutput::Refresh => read(self),
            MtuneCommandOutput::ConfigChanged(cfg) => self.bar_cfg = cfg,
        }
        apply(widgets, self);
    }
}

/// Clamp a configured cover-art size (px) to a sane range so a stray value
/// can't collapse the pill or swallow the whole bar.
fn clamp_cover_size(px: i32) -> i32 {
    px.clamp(12, 28)
}

/// Clamp a configured title-label width (chars) — same reasoning as
/// [`clamp_cover_size`].
fn clamp_label_width(chars: i32) -> i32 {
    chars.clamp(10, 80)
}

/// Read `bars.widgets.mtune` untracked, for the model's initial value.
fn read_bar_config_untracked() -> TuneBarConfig {
    TuneBarConfig {
        show_cover: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_cover()
            .get_untracked(),
        show_track_info: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_track_info()
            .get_untracked(),
        show_queue_number: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_queue_number()
            .get_untracked(),
        show_time: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_time()
            .get_untracked(),
        show_transport: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_transport()
            .get_untracked(),
        show_repeat_badge: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_repeat_badge()
            .get_untracked(),
        show_progress_bar: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_progress_bar()
            .get_untracked(),
        show_playlist_remaining: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_playlist_remaining()
            .get_untracked(),
        cover_size_px: clamp_cover_size(
            config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .cover_size_px()
                .get_untracked(),
        ),
        label_max_width_chars: clamp_label_width(
            config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .label_max_width_chars()
                .get_untracked(),
        ),
    }
}

/// Read `bars.widgets.mtune`, tracked — call only from inside an effect;
/// every field read here becomes a dependency that re-fires it.
fn read_bar_config() -> TuneBarConfig {
    TuneBarConfig {
        show_cover: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_cover()
            .get(),
        show_track_info: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_track_info()
            .get(),
        show_queue_number: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_queue_number()
            .get(),
        show_time: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_time()
            .get(),
        show_transport: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_transport()
            .get(),
        show_repeat_badge: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_repeat_badge()
            .get(),
        show_progress_bar: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_progress_bar()
            .get(),
        show_playlist_remaining: config_manager()
            .config()
            .bars()
            .widgets()
            .mtune()
            .show_playlist_remaining()
            .get(),
        cover_size_px: clamp_cover_size(
            config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .cover_size_px()
                .get(),
        ),
        label_max_width_chars: clamp_label_width(
            config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .label_max_width_chars()
                .get(),
        ),
    }
}

fn read(model: &mut MtuneModel) {
    let p = mtune_service().player.clone();
    model.running = p.running.get();
    model.playing = p.playing.get();
    model.has_song = p.has_song.get();
    model.title = p.title.get();
    model.artist = p.artist.get();
    model.cover_art = p.cover_art.get();
    model.position = p.position.get();
    model.duration = p.duration.get();
    model.current_index = p.current_index.get();
    model.queue_len = p.queue_len.get();
    model.repeat = p.repeat_mode.get();
    model.repeat_count = p.repeat_count.get();
    let (_, _, remaining) = p.playlist_progress();
    model.playlist_remaining = remaining;
}

fn apply(widgets: &MtuneModelWidgets, model: &MtuneModel) {
    let cfg = &model.bar_cfg;
    widgets.cover.set_pixel_size(cfg.cover_size_px);
    widgets.label.set_max_width_chars(cfg.label_max_width_chars);

    if !model.running {
        widgets.cover.set_icon_name(Some("org.margo.Tune-symbolic"));
        widgets.num.set_visible(false);
        widgets.label.set_visible(false);
        widgets.time.set_visible(false);
        widgets.repeat_badge.set_visible(false);
        widgets.prev_btn.set_visible(false);
        widgets.playpause_btn.set_visible(false);
        widgets.next_btn.set_visible(false);
        widgets.progress.set_visible(false);
        widgets.queue_remaining.set_visible(false);
        widgets.root.remove_css_class("paused");
        widgets
            .root
            .set_tooltip_text(Some("Tune — click to launch"));
        return;
    }

    match model.cover_art.as_deref() {
        Some(path) if !path.trim().is_empty() => widgets.cover.set_from_file(Some(path)),
        _ => widgets.cover.set_icon_name(Some(if model.playing {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        })),
    }
    widgets.cover.set_visible(cfg.show_cover);

    let title = model.title.trim();
    let artist = model.artist.trim();
    let text = match (title.is_empty(), artist.is_empty()) {
        (false, false) => format!("{title} — {artist}"),
        (false, true) => title.to_string(),
        (true, false) => artist.to_string(),
        (true, true) => "Tune".to_string(),
    };
    widgets.label.set_label(&text);
    widgets
        .label
        .set_visible(model.has_song && cfg.show_track_info);

    let num = if model.has_song && model.current_index >= 0 {
        format!("{}", model.current_index + 1)
    } else {
        String::new()
    };
    widgets.num.set_label(&num);
    widgets
        .num
        .set_visible(!num.is_empty() && cfg.show_queue_number);

    let time = if model.has_song && !model.duration.is_zero() {
        format!(
            "{} / {}",
            format_duration(model.position),
            format_duration(model.duration)
        )
    } else {
        String::new()
    };
    widgets.time.set_label(&time);
    widgets.time.set_visible(!time.is_empty() && cfg.show_time);

    let is_repeat_each = model.repeat == "repeat-each";
    widgets
        .repeat_badge
        .set_label(&format!("×{}", model.repeat_count));
    widgets
        .repeat_badge
        .set_visible(is_repeat_each && cfg.show_repeat_badge);

    // Queue-wide remaining time, not just this track's — only worth
    // showing once there's more than the current track left to play.
    let show_queue_remaining = cfg.show_playlist_remaining
        && model.playlist_remaining > model.duration.saturating_sub(model.position);
    widgets
        .queue_remaining
        .set_label(&format!("-{}", format_duration(model.playlist_remaining)));
    widgets.queue_remaining.set_visible(show_queue_remaining);

    let show_transport = cfg.show_transport;
    widgets.prev_btn.set_visible(show_transport);
    widgets.next_btn.set_visible(show_transport);
    widgets.playpause_btn.set_visible(show_transport);
    widgets.playpause_btn.set_icon_name(if model.playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    });
    widgets
        .playpause_btn
        .set_tooltip_text(Some(if model.playing { "Pause" } else { "Play" }));

    widgets.progress.set_visible(cfg.show_progress_bar);
    let fraction = if model.duration.is_zero() {
        0.0
    } else {
        (model.position.as_secs_f64() / model.duration.as_secs_f64()).clamp(0.0, 1.0)
    };
    widgets.progress.set_fraction(fraction);

    if model.playing {
        widgets.root.remove_css_class("paused");
    } else {
        widgets.root.add_css_class("paused");
    }

    widgets.root.set_tooltip_text(Some(&if model.has_song {
        let head = if model.playing { "Playing" } else { "Paused" };
        let pos = if model.current_index >= 0 && model.queue_len > 0 {
            format!("  ·  {} of {}", model.current_index + 1, model.queue_len)
        } else {
            String::new()
        };
        if time.is_empty() {
            format!("{head}  ·  {text}{pos}")
        } else {
            format!("{head}  ·  {text}{pos}  ·  {time}")
        }
    } else {
        "Tune".to_string()
    }));
}
