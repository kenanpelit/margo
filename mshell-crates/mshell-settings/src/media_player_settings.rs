//! Media-player widget settings — the mplayerplus-derived knobs the generic
//! per-menu page (position / size) doesn't cover: the ± seek-button step and
//! a larger album cover. Reads/writes `general.media_seek_step_seconds` and
//! `general.media_large_album_art`; the media-player menu reads them when a
//! player component is built.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct MediaPlayerSettingsModel {
    seek_step: u32,
    large_art: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MediaPlayerSettingsInput {
    SeekStepChanged(u32),
    LargeArtChanged(bool),
    SeekStepEffect(u32),
    LargeArtEffect(bool),
}

#[derive(Debug)]
pub(crate) enum MediaPlayerSettingsOutput {}

pub(crate) struct MediaPlayerSettingsInit {}

#[derive(Debug)]
pub(crate) enum MediaPlayerSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for MediaPlayerSettingsModel {
    type CommandOutput = MediaPlayerSettingsCommandOutput;
    type Input = MediaPlayerSettingsInput;
    type Output = MediaPlayerSettingsOutput;
    type Init = MediaPlayerSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("media-playback-start-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_halign: gtk::Align::Start,
                            set_label: "Media Player",
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_halign: gtk::Align::Start,
                            set_label: "Seek step and album-art size for the media-player menu.",
                        },
                    },
                },

                // Seek step (seconds)
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Seek step (seconds)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How far the − / + buttons in the media-player menu jump.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 120.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(seek_handler)]
                        set_value: model.seek_step as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(MediaPlayerSettingsInput::SeekStepChanged(s.value() as u32));
                        } @seek_handler,
                    },
                },

                // Large album art
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Large album art",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Show a bigger album cover in the media-player menu.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(art_handler)]
                        set_active: model.large_art,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(MediaPlayerSettingsInput::LargeArtChanged(v));
                            glib::Propagation::Proceed
                        } @art_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Changes apply to the next track / on the next time the menu opens.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .media_seek_step_seconds()
                .get();
            sc.input(MediaPlayerSettingsInput::SeekStepEffect(v));
        });

        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .media_large_album_art()
                .get();
            sc.input(MediaPlayerSettingsInput::LargeArtEffect(v));
        });

        let model = MediaPlayerSettingsModel {
            seek_step: config_manager()
                .config()
                .general()
                .media_seek_step_seconds()
                .get_untracked(),
            large_art: config_manager()
                .config()
                .general()
                .media_large_album_art()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            MediaPlayerSettingsInput::SeekStepChanged(v) => {
                config_manager().update_config(move |c| c.general.media_seek_step_seconds = v);
            }
            MediaPlayerSettingsInput::LargeArtChanged(v) => {
                config_manager().update_config(move |c| c.general.media_large_album_art = v);
            }
            MediaPlayerSettingsInput::SeekStepEffect(v) => self.seek_step = v,
            MediaPlayerSettingsInput::LargeArtEffect(v) => self.large_art = v,
        }
    }
}
