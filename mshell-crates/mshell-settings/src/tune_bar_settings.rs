//! Settings → Widgets → Tune — the Tune bar pill's own behaviour: which
//! elements it shows, and how large the cover art / title read. Composed in
//! `settings.rs` with the generic per-menu geometry page (position / width /
//! height), the same way Lyrics is. The Tune *application* itself (library
//! folders, playback, behaviour) is a separate page — see `mtune_settings`.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, TuneBarWidgetStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct TuneBarSettingsModel {
    show_cover: bool,
    show_track_info: bool,
    show_queue_number: bool,
    show_time: bool,
    show_transport: bool,
    show_repeat_badge: bool,
    show_progress_bar: bool,
    show_playlist_remaining: bool,
    cover_size: f64,
    label_width: f64,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum TuneBarSettingsInput {
    ShowCoverChanged(bool),
    ShowTrackInfoChanged(bool),
    ShowQueueNumberChanged(bool),
    ShowTimeChanged(bool),
    ShowTransportChanged(bool),
    ShowRepeatBadgeChanged(bool),
    ShowProgressBarChanged(bool),
    ShowPlaylistRemainingChanged(bool),
    CoverSizeChanged(f64),
    LabelWidthChanged(f64),
    Effect(TuneBarSettingsEffect),
}

/// One `bars.widgets.mtune` field moved in Settings elsewhere (another
/// window, `mctl reload`, …) — mirror it without re-writing the file.
#[derive(Debug)]
pub(crate) enum TuneBarSettingsEffect {
    ShowCover(bool),
    ShowTrackInfo(bool),
    ShowQueueNumber(bool),
    ShowTime(bool),
    ShowTransport(bool),
    ShowRepeatBadge(bool),
    ShowProgressBar(bool),
    ShowPlaylistRemaining(bool),
    CoverSize(i32),
    LabelWidth(i32),
}

#[derive(Debug)]
pub(crate) enum TuneBarSettingsOutput {}

pub(crate) struct TuneBarSettingsInit {}

#[derive(Debug)]
pub(crate) enum TuneBarSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for TuneBarSettingsModel {
    type CommandOutput = TuneBarSettingsCommandOutput;
    type Input = TuneBarSettingsInput;
    type Output = TuneBarSettingsOutput;
    type Init = TuneBarSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bar pill",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Cover art",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "The album/track artwork thumbnail (a play/pause glyph when there's none).",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_cover_handler)]
                            set_active: model.show_cover,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowCoverChanged(v));
                                glib::Propagation::Proceed
                            } @show_cover_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Title — Artist",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "The current track's title and artist.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_track_info_handler)]
                            set_active: model.show_track_info,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowTrackInfoChanged(v));
                                glib::Propagation::Proceed
                            } @show_track_info_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Queue position",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "The 1-based position in the queue, e.g. \"3\".",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_queue_number_handler)]
                            set_active: model.show_queue_number,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowQueueNumberChanged(v));
                                glib::Propagation::Proceed
                            } @show_queue_number_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Elapsed / total time",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Playback position and track length.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_time_handler)]
                            set_active: model.show_time,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowTimeChanged(v));
                                glib::Propagation::Proceed
                            } @show_time_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Transport buttons",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Previous / play-pause / next, right in the pill. Right-click anywhere on the pill still toggles play/pause either way.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_transport_handler)]
                            set_active: model.show_transport,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowTransportChanged(v));
                                glib::Propagation::Proceed
                            } @show_transport_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Repeat-each badge",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "The \"2/3\"-style play count while repeat-each is active.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_repeat_badge_handler)]
                            set_active: model.show_repeat_badge,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowRepeatBadgeChanged(v));
                                glib::Propagation::Proceed
                            } @show_repeat_badge_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Progress line",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "A thin playback-position line under the pill.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_progress_bar_handler)]
                            set_active: model.show_progress_bar,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowProgressBarChanged(v));
                                glib::Propagation::Proceed
                            } @show_progress_bar_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Queue remaining",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Time left across the whole queue, e.g. \"-12:34\" — not just this track. Hidden on the last track, where it would just repeat the time above.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_playlist_remaining_handler)]
                            set_active: model.show_playlist_remaining,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TuneBarSettingsInput::ShowPlaylistRemainingChanged(v));
                                glib::Propagation::Proceed
                            } @show_playlist_remaining_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Size",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Cover art size",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "In pixels.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(16.0, 12.0, 28.0, 1.0, 2.0, 0.0),
                            #[watch]
                            #[block_signal(cover_size_handler)]
                            set_value: model.cover_size,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(TuneBarSettingsInput::CoverSizeChanged(sb.value()));
                            } @cover_size_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Title width",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "How many characters of \"Title — Artist\" fit before it ellipsizes.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(40.0, 10.0, 80.0, 1.0, 4.0, 0.0),
                            #[watch]
                            #[block_signal(label_width_handler)]
                            set_value: model.label_width,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(TuneBarSettingsInput::LabelWidthChanged(sb.value()));
                            } @label_width_handler,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        macro_rules! push_effect {
            ($field:ident, $variant:ident) => {
                let cs = sender.clone();
                effects.push(move |_| {
                    let v = config_manager()
                        .config()
                        .bars()
                        .widgets()
                        .mtune()
                        .$field()
                        .get();
                    cs.input(TuneBarSettingsInput::Effect(
                        TuneBarSettingsEffect::$variant(v),
                    ));
                });
            };
        }
        push_effect!(show_cover, ShowCover);
        push_effect!(show_track_info, ShowTrackInfo);
        push_effect!(show_queue_number, ShowQueueNumber);
        push_effect!(show_time, ShowTime);
        push_effect!(show_transport, ShowTransport);
        push_effect!(show_repeat_badge, ShowRepeatBadge);
        push_effect!(show_progress_bar, ShowProgressBar);
        push_effect!(show_playlist_remaining, ShowPlaylistRemaining);
        push_effect!(cover_size_px, CoverSize);
        push_effect!(label_max_width_chars, LabelWidth);

        let model = TuneBarSettingsModel {
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
            cover_size: config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .cover_size_px()
                .get_untracked() as f64,
            label_width: config_manager()
                .config()
                .bars()
                .widgets()
                .mtune()
                .label_max_width_chars()
                .get_untracked() as f64,
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TuneBarSettingsInput::ShowCoverChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_cover = v);
            }
            TuneBarSettingsInput::ShowTrackInfoChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_track_info = v);
            }
            TuneBarSettingsInput::ShowQueueNumberChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_queue_number = v);
            }
            TuneBarSettingsInput::ShowTimeChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_time = v);
            }
            TuneBarSettingsInput::ShowTransportChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_transport = v);
            }
            TuneBarSettingsInput::ShowRepeatBadgeChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_repeat_badge = v);
            }
            TuneBarSettingsInput::ShowProgressBarChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.mtune.show_progress_bar = v);
            }
            TuneBarSettingsInput::ShowPlaylistRemainingChanged(v) => {
                config_manager()
                    .update_config(|c| c.bars.widgets.mtune.show_playlist_remaining = v);
            }
            TuneBarSettingsInput::CoverSizeChanged(v) => {
                let px = (v as i32).clamp(12, 28);
                config_manager().update_config(|c| c.bars.widgets.mtune.cover_size_px = px);
            }
            TuneBarSettingsInput::LabelWidthChanged(v) => {
                let chars = (v as i32).clamp(10, 80);
                config_manager()
                    .update_config(|c| c.bars.widgets.mtune.label_max_width_chars = chars);
            }
            TuneBarSettingsInput::Effect(effect) => match effect {
                TuneBarSettingsEffect::ShowCover(v) => self.show_cover = v,
                TuneBarSettingsEffect::ShowTrackInfo(v) => self.show_track_info = v,
                TuneBarSettingsEffect::ShowQueueNumber(v) => self.show_queue_number = v,
                TuneBarSettingsEffect::ShowTime(v) => self.show_time = v,
                TuneBarSettingsEffect::ShowTransport(v) => self.show_transport = v,
                TuneBarSettingsEffect::ShowRepeatBadge(v) => self.show_repeat_badge = v,
                TuneBarSettingsEffect::ShowProgressBar(v) => self.show_progress_bar = v,
                TuneBarSettingsEffect::ShowPlaylistRemaining(v) => self.show_playlist_remaining = v,
                TuneBarSettingsEffect::CoverSize(v) => self.cover_size = v as f64,
                TuneBarSettingsEffect::LabelWidth(v) => self.label_width = v as f64,
            },
        }
    }
}
