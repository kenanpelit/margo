//! Settings → Widgets → Lyrics — the lyrics bar pill's behaviour knob: whether
//! the current synced line scrolls in the bar, or the words live only in the
//! menu. Composed in `settings.rs` with the generic per-menu geometry page
//! (position / width / height), the same way Media Player is.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, LyricsBarWidgetStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct LyricsSettingsModel {
    show_line: bool,
    max_width: f64,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum LyricsSettingsInput {
    ShowLineChanged(bool),
    ShowLineEffect(bool),
    MaxWidthChanged(f64),
    MaxWidthEffect(i32),
}

#[derive(Debug)]
pub(crate) enum LyricsSettingsOutput {}

pub(crate) struct LyricsSettingsInit {}

#[derive(Debug)]
pub(crate) enum LyricsSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for LyricsSettingsModel {
    type CommandOutput = LyricsSettingsCommandOutput;
    type Input = LyricsSettingsInput;
    type Output = LyricsSettingsOutput;
    type Init = LyricsSettingsInit;

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
                    set_label: "Bar",
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
                                set_label: "Show current line in the bar",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "On: the active synced line scrolls in the bar pill. Off: the pill is just an icon and the lyrics show only in the menu.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_line_handler)]
                            set_active: model.show_line,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(LyricsSettingsInput::ShowLineChanged(v));
                                glib::Propagation::Proceed
                            } @show_line_handler,
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
                                set_label: "Line width in the bar",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "How many characters of the line fit before it ellipsizes, so a long lyric can't stretch the bar.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(32.0, 8.0, 120.0, 1.0, 4.0, 0.0),
                            #[watch]
                            #[block_signal(max_width_handler)]
                            set_value: model.max_width,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(LyricsSettingsInput::MaxWidthChanged(sb.value()));
                            } @max_width_handler,
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
        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .show_line_in_bar()
                .get();
            sc.input(LyricsSettingsInput::ShowLineEffect(v));
        });
        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .max_width_chars()
                .get();
            sc.input(LyricsSettingsInput::MaxWidthEffect(v));
        });

        let model = LyricsSettingsModel {
            show_line: config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .show_line_in_bar()
                .get_untracked(),
            max_width: config_manager()
                .config()
                .bars()
                .widgets()
                .lyrics()
                .max_width_chars()
                .get_untracked() as f64,
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LyricsSettingsInput::ShowLineChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.lyrics.show_line_in_bar = v);
            }
            LyricsSettingsInput::ShowLineEffect(v) => self.show_line = v,
            LyricsSettingsInput::MaxWidthChanged(v) => {
                let chars = (v as i32).clamp(8, 120);
                config_manager().update_config(|c| c.bars.widgets.lyrics.max_width_chars = chars);
            }
            LyricsSettingsInput::MaxWidthEffect(v) => self.max_width = v as f64,
        }
    }
}
