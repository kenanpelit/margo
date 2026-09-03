//! Tune bar pill — the dedicated entry point for the `mtune` folder-first
//! music player. Distinct from the generic `MediaPlayer` MPRIS pill: it
//! mirrors *only* `mtune` (via `mtune_service()` → `org.margo.Tune`) and
//! its menu carries the library / folder-picker controls MPRIS can't
//! express.
//!
//!   * left click  → toggle the `MenuType::Mtune` panel.
//!   * right click → play / pause in place.
//!   * when mtune isn't running → a single glyph; click launches it.

use futures::StreamExt;
use mshell_services::mtune::{mtune_service, spawn_mtune};
use mshell_services::tokio_rt_spawn;
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct MtuneModel {
    running: bool,
    playing: bool,
    has_song: bool,
    title: String,
    artist: String,
    cover_art: Option<String>,
}

#[derive(Debug)]
pub(crate) enum MtuneInput {
    Clicked,
    PlayPauseClicked,
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
            set_css_classes: &["mtune-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

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
                        set_pixel_size: 20,
                    },

                    #[name = "label"]
                    gtk::Label {
                        add_css_class: "mtune-bar-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_ellipsize: pango::EllipsizeMode::End,
                        set_max_width_chars: 40,
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
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = running.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = playing.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = has_song.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = title.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = artist.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                    _ = cover.next() => { let _ = out.send(MtuneCommandOutput::Refresh); }
                }
            }
        });

        let mut model = MtuneModel {
            running: false,
            playing: false,
            has_song: false,
            title: String::new(),
            artist: String::new(),
            cover_art: None,
        };
        read(&mut model);

        let widgets = view_output!();

        // Right click → play/pause in place.
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
        }
        apply(widgets, self);
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
}

fn apply(widgets: &MtuneModelWidgets, model: &MtuneModel) {
    if !model.running {
        widgets.cover.set_icon_name(Some("folder-music-symbolic"));
        widgets.label.set_visible(false);
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

    let title = model.title.trim();
    let artist = model.artist.trim();
    let text = match (title.is_empty(), artist.is_empty()) {
        (false, false) => format!("{title} — {artist}"),
        (false, true) => title.to_string(),
        (true, false) => artist.to_string(),
        (true, true) => "Tune".to_string(),
    };
    widgets.label.set_label(&text);
    widgets.label.set_visible(model.has_song);

    if model.playing {
        widgets.root.remove_css_class("paused");
    } else {
        widgets.root.add_css_class("paused");
    }

    widgets.root.set_tooltip_text(Some(&if model.has_song {
        format!(
            "{}  ·  {}",
            if model.playing { "Playing" } else { "Paused" },
            text
        )
    } else {
        "Tune".to_string()
    }));
}
