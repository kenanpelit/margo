//! Catwalk widget settings — CPU threshold + background toggle for the
//! animated-cat pill (`bars.widgets.catwalk`).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, CatwalkConfigStoreFields, ConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct CatwalkSettingsModel {
    minimum_threshold: u32,
    hide_background: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CatwalkSettingsInput {
    ThresholdChanged(u32),
    HideBackgroundChanged(bool),
    ThresholdEffect(u32),
    HideBackgroundEffect(bool),
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
                    set_label: "Background changes apply when the bar next rebuilds the widget.",
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
            CatwalkSettingsInput::ThresholdEffect(v) => self.minimum_threshold = v,
            CatwalkSettingsInput::HideBackgroundEffect(v) => self.hide_background = v,
        }
    }
}
