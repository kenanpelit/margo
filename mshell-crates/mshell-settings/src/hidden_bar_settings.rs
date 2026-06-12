//! Hidden Bar widget settings — the drawer behaviour knobs
//! (`bars.widgets.hidden_bar`). The *which widgets* part is handled by the
//! reusable bar-widget section editors (TopHidden / BottomHidden) composed
//! alongside this on the Widgets → Hidden Bar page.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, HiddenBarConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct HiddenBarSettingsModel {
    start_expanded: bool,
    auto_expand: bool,
    hover_delay_ms: u32,
    auto_collapse: bool,
    collapse_delay_ms: u32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsInput {
    StartExpandedChanged(bool),
    AutoExpandChanged(bool),
    HoverDelayChanged(u32),
    AutoCollapseChanged(bool),
    CollapseDelayChanged(u32),
    StartExpandedEffect(bool),
    AutoExpandEffect(bool),
    HoverDelayEffect(u32),
    AutoCollapseEffect(bool),
    CollapseDelayEffect(u32),
}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsOutput {}

pub(crate) struct HiddenBarSettingsInit {}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for HiddenBarSettingsModel {
    type CommandOutput = HiddenBarSettingsCommandOutput;
    type Input = HiddenBarSettingsInput;
    type Output = HiddenBarSettingsOutput;
    type Init = HiddenBarSettingsInit;

    view! {
        #[root]
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
                    set_icon_name: Some("view-more-horizontal-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Hidden Bar",
                    },
                    gtk::Label {
                        add_css_class: "settings-hero-subtitle",
                        set_halign: gtk::Align::Start,
                        set_label: "Collapse bar widgets behind a drawer. Pick which widgets to hide below; left-click the trigger to toggle, right-click to pin.",
                        set_wrap: true,
                        set_xalign: 0.0,
                    },
                },
            },

            gtk::Box {
                add_css_class: "boxed-list",
                set_orientation: gtk::Orientation::Vertical,

                // Reveal on hover
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
                            set_label: "Reveal on hover",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Expand the drawer when the pointer hovers the trigger (in addition to clicking).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(auto_expand_handler)]
                        set_active: model.auto_expand,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::AutoExpandChanged(v));
                            glib::Propagation::Proceed
                        } @auto_expand_handler,
                    },
                },

                // Hover delay
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
                            set_label: "Hover delay (ms)",
                            set_hexpand: true,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 5000.0),
                        set_increments: (50.0, 250.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(hover_delay_handler)]
                        set_value: model.hover_delay_ms as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(HiddenBarSettingsInput::HoverDelayChanged(s.value() as u32));
                        } @hover_delay_handler,
                    },
                },

                // Auto-collapse
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
                            set_label: "Auto-collapse",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Collapse again after the pointer leaves (unless pinned with right-click).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(auto_collapse_handler)]
                        set_active: model.auto_collapse,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::AutoCollapseChanged(v));
                            glib::Propagation::Proceed
                        } @auto_collapse_handler,
                    },
                },

                // Collapse delay
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
                            set_label: "Collapse delay (ms)",
                            set_hexpand: true,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 10000.0),
                        set_increments: (100.0, 500.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(collapse_delay_handler)]
                        set_value: model.collapse_delay_ms as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(HiddenBarSettingsInput::CollapseDelayChanged(s.value() as u32));
                        } @collapse_delay_handler,
                    },
                },

                // Start expanded
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
                            set_label: "Start expanded",
                            set_hexpand: true,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(start_expanded_handler)]
                        set_active: model.start_expanded,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::StartExpandedChanged(v));
                            glib::Propagation::Proceed
                        } @start_expanded_handler,
                    },
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

        macro_rules! push_effect {
            ($field:ident, $variant:ident) => {{
                let sc = sender.clone();
                effects.push(move |_| {
                    let v = config_manager()
                        .config()
                        .bars()
                        .widgets()
                        .hidden_bar()
                        .$field()
                        .get();
                    sc.input(HiddenBarSettingsInput::$variant(v));
                });
            }};
        }
        push_effect!(start_expanded, StartExpandedEffect);
        push_effect!(auto_expand, AutoExpandEffect);
        push_effect!(hover_delay_ms, HoverDelayEffect);
        push_effect!(auto_collapse, AutoCollapseEffect);
        push_effect!(collapse_delay_ms, CollapseDelayEffect);

        macro_rules! read {
            ($field:ident) => {
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .hidden_bar()
                    .$field()
                    .get_untracked()
            };
        }
        let model = HiddenBarSettingsModel {
            start_expanded: read!(start_expanded),
            auto_expand: read!(auto_expand),
            hover_delay_ms: read!(hover_delay_ms),
            auto_collapse: read!(auto_collapse),
            collapse_delay_ms: read!(collapse_delay_ms),
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            HiddenBarSettingsInput::StartExpandedChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.start_expanded = v);
            }
            HiddenBarSettingsInput::AutoExpandChanged(v) => {
                config_manager().update_config(move |c| c.bars.widgets.hidden_bar.auto_expand = v);
            }
            HiddenBarSettingsInput::HoverDelayChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.hover_delay_ms = v);
            }
            HiddenBarSettingsInput::AutoCollapseChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.auto_collapse = v);
            }
            HiddenBarSettingsInput::CollapseDelayChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.collapse_delay_ms = v);
            }
            HiddenBarSettingsInput::StartExpandedEffect(v) => self.start_expanded = v,
            HiddenBarSettingsInput::AutoExpandEffect(v) => self.auto_expand = v,
            HiddenBarSettingsInput::HoverDelayEffect(v) => self.hover_delay_ms = v,
            HiddenBarSettingsInput::AutoCollapseEffect(v) => self.auto_collapse = v,
            HiddenBarSettingsInput::CollapseDelayEffect(v) => self.collapse_delay_ms = v,
        }
    }
}
