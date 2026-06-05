//! Settings → Effects. Drop shadows + blur toggles in margo's `config.conf`.

use crate::compositor_conf::{read_bool, read_int, set_and_reload};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum EffectsInput {
    SetBool(&'static str, bool),
    SetInt(&'static str, i64),
}

#[derive(Debug)]
pub(crate) enum EffectsOutput {}
#[derive(Debug)]
pub(crate) enum EffectsCommandOutput {}
pub(crate) struct EffectsInit {}

pub(crate) struct EffectsModel {
    shadows: bool,
    shadow_only_floating: bool,
    layer_shadows: bool,
    shadows_size: f64,
    shadows_blur: f64,
    shadows_position_x: f64,
    shadows_position_y: f64,
    blur: bool,
    blur_layer: bool,
    blur_optimized: bool,
}

fn adj(value: f64, lo: f64, hi: f64, step: f64) -> gtk::Adjustment {
    gtk::Adjustment::new(value, lo, hi, step, step * 4.0, 0.0)
}

#[relm4::component(pub)]
impl Component for EffectsModel {
    type CommandOutput = EffectsCommandOutput;
    type Input = EffectsInput;
    type Output = EffectsOutput;
    type Init = EffectsInit;

    view! {
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
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("applications-graphics-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Effects",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Drop shadows and blur. Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Shadows", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Drop shadows" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.shadows,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("shadows", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Only on floating windows" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.shadow_only_floating,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("shadow_only_floating", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Shadows on layer surfaces (bar/menus)" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.layer_shadows,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("layer_shadows", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Shadow size" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.shadows_size, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(EffectsInput::SetInt("shadows_size", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Shadow blur" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.shadows_blur, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(EffectsInput::SetInt("shadows_blur", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Shadow offset X" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.shadows_position_x, -32.0, 32.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(EffectsInput::SetInt("shadows_position_x", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Shadow offset Y" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.shadows_position_y, -32.0, 32.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(EffectsInput::SetInt("shadows_position_y", s.value() as i64)),
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Blur", set_halign: gtk::Align::Start },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_label: "Note: the Kawase blur backend isn't implemented yet, so these are stored but have no visible effect for now.",
                },

                #[template] Row {
                    #[template_child] title { set_label: "Window blur" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.blur,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("blur", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Layer-surface blur" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.blur_layer,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("blur_layer", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Optimized blur" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.blur_optimized,
                        connect_active_notify[sender] => move |s| sender.input(EffectsInput::SetBool("blur_optimized", s.is_active())),
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
        let _ = &sender;
        let model = EffectsModel {
            shadows: read_bool("shadows", true),
            shadow_only_floating: read_bool("shadow_only_floating", true),
            layer_shadows: read_bool("layer_shadows", false),
            shadows_size: read_int("shadows_size", 14) as f64,
            shadows_blur: read_int("shadows_blur", 26) as f64,
            shadows_position_x: read_int("shadows_position_x", 0) as f64,
            shadows_position_y: read_int("shadows_position_y", 4) as f64,
            blur: read_bool("blur", false),
            blur_layer: read_bool("blur_layer", false),
            blur_optimized: read_bool("blur_optimized", true),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            EffectsInput::SetBool(k, v) => set_and_reload(k, if v { "1" } else { "0" }.to_string()),
            EffectsInput::SetInt(k, v) => set_and_reload(k, v.to_string()),
        }
    }
}
