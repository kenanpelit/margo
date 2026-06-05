//! Settings → Appearance. Borders, gaps, opacity, cursor size in margo's
//! `config.conf`. Each change writes the key in place and runs `mctl reload`.

use crate::compositor_conf::{read_bool, read_f64, read_int, set_and_reload};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum AppearanceInput {
    SetBool(&'static str, bool),
    SetInt(&'static str, i64),
    SetF(&'static str, f64, usize),
}

#[derive(Debug)]
pub(crate) enum AppearanceOutput {}
#[derive(Debug)]
pub(crate) enum AppearanceCommandOutput {}
pub(crate) struct AppearanceInit {}

pub(crate) struct AppearanceModel {
    borderpx: f64,
    border_radius: f64,
    no_border_when_single: bool,
    no_radius_when_single: bool,
    gappih: f64,
    gappiv: f64,
    gappoh: f64,
    gappov: f64,
    smartgaps: bool,
    focused_opacity: f64,
    unfocused_opacity: f64,
    cursor_size: f64,
}

fn adj(value: f64, lo: f64, hi: f64, step: f64) -> gtk::Adjustment {
    gtk::Adjustment::new(value, lo, hi, step, step * 4.0, 0.0)
}

#[relm4::component(pub)]
impl Component for AppearanceModel {
    type CommandOutput = AppearanceCommandOutput;
    type Input = AppearanceInput;
    type Output = AppearanceOutput;
    type Init = AppearanceInit;

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
                        set_icon_name: Some("preferences-desktop-display-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Appearance",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Window borders, gaps, opacity and cursor size. Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Border", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Border thickness (px)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.borderpx, 0.0, 32.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("borderpx", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Corner radius (px)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.border_radius, 0.0, 32.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("border_radius", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "No border when single window" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.no_border_when_single,
                        connect_active_notify[sender] => move |s| sender.input(AppearanceInput::SetBool("no_border_when_single", s.is_active())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "No corner radius when single window" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.no_radius_when_single,
                        connect_active_notify[sender] => move |s| sender.input(AppearanceInput::SetBool("no_radius_when_single", s.is_active())),
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Gaps", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Inner gap — horizontal" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.gappih, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("gappih", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Inner gap — vertical" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.gappiv, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("gappiv", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Outer gap — horizontal" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.gappoh, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("gappoh", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Outer gap — vertical" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.gappov, 0.0, 64.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("gappov", s.value() as i64)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Smart gaps (drop gaps with one window)" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.smartgaps,
                        connect_active_notify[sender] => move |s| sender.input(AppearanceInput::SetBool("smartgaps", s.is_active())),
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Opacity & cursor", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Focused window opacity" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_digits: 2,
                        set_adjustment: &adj(model.focused_opacity, 0.1, 1.0, 0.05),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetF("focused_opacity", s.value(), 2)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Unfocused window opacity" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_digits: 2,
                        set_adjustment: &adj(model.unfocused_opacity, 0.1, 1.0, 0.05),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetF("unfocused_opacity", s.value(), 2)),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Cursor size (px)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &adj(model.cursor_size, 8.0, 96.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(AppearanceInput::SetInt("cursor_size", s.value() as i64)),
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
        let model = AppearanceModel {
            borderpx: read_int("borderpx", 2) as f64,
            border_radius: read_int("border_radius", 12) as f64,
            no_border_when_single: read_bool("no_border_when_single", false),
            no_radius_when_single: read_bool("no_radius_when_single", false),
            gappih: read_int("gappih", 12) as f64,
            gappiv: read_int("gappiv", 12) as f64,
            gappoh: read_int("gappoh", 12) as f64,
            gappov: read_int("gappov", 12) as f64,
            smartgaps: read_bool("smartgaps", false),
            focused_opacity: read_f64("focused_opacity", 1.0),
            unfocused_opacity: read_f64("unfocused_opacity", 0.9),
            cursor_size: read_int("cursor_size", 24) as f64,
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AppearanceInput::SetBool(k, v) => {
                set_and_reload(k, if v { "1" } else { "0" }.to_string())
            }
            AppearanceInput::SetInt(k, v) => set_and_reload(k, v.to_string()),
            AppearanceInput::SetF(k, v, d) => set_and_reload(k, format!("{:.*}", d, v)),
        }
    }
}
