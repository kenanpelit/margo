//! Catwalk widget settings — cat style, display mode, sprite size, CPU
//! threshold + poll cadence, and the background toggle for the animated-cat
//! pill (`bars.widgets.catwalk`).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, CatStyle, CatwalkConfigStoreFields, CatwalkDisplay,
    ConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

fn style_index(s: CatStyle) -> u32 {
    match s {
        CatStyle::Noctalia => 0,
        CatStyle::RunCat => 1,
    }
}
fn style_from_index(i: u32) -> CatStyle {
    match i {
        0 => CatStyle::Noctalia,
        _ => CatStyle::RunCat,
    }
}
fn display_index(d: CatwalkDisplay) -> u32 {
    match d {
        CatwalkDisplay::Icon => 0,
        CatwalkDisplay::Text => 1,
        CatwalkDisplay::Both => 2,
    }
}
fn display_from_index(i: u32) -> CatwalkDisplay {
    match i {
        1 => CatwalkDisplay::Text,
        2 => CatwalkDisplay::Both,
        _ => CatwalkDisplay::Icon,
    }
}

pub(crate) struct CatwalkSettingsModel {
    minimum_threshold: u32,
    hide_background: bool,
    style_idx: u32,
    display_idx: u32,
    size: u32,
    poll_secs: u32,
    styles: gtk::StringList,
    displays: gtk::StringList,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CatwalkSettingsInput {
    ThresholdChanged(u32),
    HideBackgroundChanged(bool),
    StyleChanged(u32),
    DisplayChanged(u32),
    SizeChanged(u32),
    PollChanged(u32),
    ThresholdEffect(u32),
    HideBackgroundEffect(bool),
    SizeEffect(u32),
    PollEffect(u32),
}

#[derive(Debug)]
pub(crate) enum CatwalkSettingsOutput {}

pub(crate) struct CatwalkSettingsInit {}

#[derive(Debug)]
pub(crate) enum CatwalkSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for CatwalkSettingsModel {
    type CommandOutput = CatwalkSettingsCommandOutput;
    type Input = CatwalkSettingsInput;
    type Output = CatwalkSettingsOutput;
    type Init = CatwalkSettingsInit;

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
                        set_icon_name: Some("face-smile-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_halign: gtk::Align::Start,
                            set_label: "Catwalk",
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_halign: gtk::Align::Start,
                            set_label: "An animated cat that walks faster as your CPU works harder. Click it for the CPU dashboard.",
                            set_wrap: true,
                            set_xalign: 0.0,
                        },
                    },
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    // Cat style
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
                                set_label: "Cat style",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Noctalia (4 walk + 4 idle frames) or RunCat — the classic macOS running cat (5 run frames + a sleeping pose).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 160,
                            set_model: Some(&model.styles),
                            #[block_signal(style_h)]
                            set_selected: model.style_idx,
                            connect_selected_notify[sender] => move |d| sender.input(CatwalkSettingsInput::StyleChanged(d.selected())) @style_h,
                        },
                    },

                    // Display mode
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
                                set_label: "Display",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show the cat, the CPU percentage (severity-coloured), or both.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 160,
                            set_model: Some(&model.displays),
                            #[block_signal(display_h)]
                            set_selected: model.display_idx,
                            connect_selected_notify[sender] => move |d| sender.input(CatwalkSettingsInput::DisplayChanged(d.selected())) @display_h,
                        },
                    },

                    // Sprite size
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
                                set_label: "Cat size (px)",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Sprite size in the bar (12–48).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (12.0, 48.0),
                            set_increments: (1.0, 4.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(size_handler)]
                            set_value: model.size as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(CatwalkSettingsInput::SizeChanged(s.value() as u32));
                            } @size_handler,
                        },
                    },

                    // CPU threshold
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
                                set_label: "Run threshold (CPU %)",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Below this the cat idles; above it walks, speeding up with load.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 90.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(threshold_handler)]
                            set_value: model.minimum_threshold as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(CatwalkSettingsInput::ThresholdChanged(s.value() as u32));
                            } @threshold_handler,
                        },
                    },

                    // CPU poll interval
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
                                set_label: "CPU poll interval (sec)",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "How often to re-sample CPU load (1–10).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 10.0),
                            set_increments: (1.0, 2.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(poll_handler)]
                            set_value: model.poll_secs as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(CatwalkSettingsInput::PollChanged(s.value() as u32));
                            } @poll_handler,
                        },
                    },

                    // Hide background
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
                                set_label: "Hide background",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Drop the pill background so the cat floats on the bar.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(hide_bg_handler)]
                            set_active: model.hide_background,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(CatwalkSettingsInput::HideBackgroundChanged(v));
                                glib::Propagation::Proceed
                            } @hide_bg_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Style and background changes apply when the bar next rebuilds the widget.",
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
                .bars()
                .widgets()
                .catwalk()
                .minimum_threshold()
                .get();
            sc.input(CatwalkSettingsInput::ThresholdEffect(v));
        });
        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .hide_background()
                .get();
            sc.input(CatwalkSettingsInput::HideBackgroundEffect(v));
        });
        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .size()
                .get();
            sc.input(CatwalkSettingsInput::SizeEffect(v));
        });
        let sc = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .poll_secs()
                .get();
            sc.input(CatwalkSettingsInput::PollEffect(v));
        });

        let model = CatwalkSettingsModel {
            minimum_threshold: config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .minimum_threshold()
                .get_untracked(),
            hide_background: config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .hide_background()
                .get_untracked(),
            // The enum fields aren't reactive sub-stores, so read them off a
            // whole-struct snapshot (the proven pattern from dock settings).
            style_idx: style_index(
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .catwalk()
                    .get_untracked()
                    .style,
            ),
            display_idx: display_index(
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .catwalk()
                    .get_untracked()
                    .display,
            ),
            size: config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .size()
                .get_untracked(),
            poll_secs: config_manager()
                .config()
                .bars()
                .widgets()
                .catwalk()
                .poll_secs()
                .get_untracked(),
            styles: gtk::StringList::new(&["Noctalia", "RunCat"]),
            displays: gtk::StringList::new(&["Cat only", "CPU % only", "Cat + CPU %"]),
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CatwalkSettingsInput::ThresholdChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.catwalk.minimum_threshold = v);
            }
            CatwalkSettingsInput::HideBackgroundChanged(v) => {
                config_manager().update_config(move |c| c.bars.widgets.catwalk.hide_background = v);
            }
            CatwalkSettingsInput::StyleChanged(i) => {
                self.style_idx = i;
                let s = style_from_index(i);
                config_manager().update_config(move |c| c.bars.widgets.catwalk.style = s);
            }
            CatwalkSettingsInput::DisplayChanged(i) => {
                self.display_idx = i;
                let d = display_from_index(i);
                config_manager().update_config(move |c| c.bars.widgets.catwalk.display = d);
            }
            CatwalkSettingsInput::SizeChanged(v) => {
                config_manager().update_config(move |c| c.bars.widgets.catwalk.size = v);
            }
            CatwalkSettingsInput::PollChanged(v) => {
                config_manager().update_config(move |c| c.bars.widgets.catwalk.poll_secs = v);
            }
            CatwalkSettingsInput::ThresholdEffect(v) => self.minimum_threshold = v,
            CatwalkSettingsInput::HideBackgroundEffect(v) => self.hide_background = v,
            CatwalkSettingsInput::SizeEffect(v) => self.size = v,
            CatwalkSettingsInput::PollEffect(v) => self.poll_secs = v,
        }
    }
}
