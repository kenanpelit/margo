//! Control Center — bar pill that opens the Control Center menu.
//!
//! A system-preferences glyph. Click opens the Control Center menu.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ControlCenterModel {
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ControlCenterInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ControlCenterOutput {
    Clicked,
}

pub(crate) struct ControlCenterInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for ControlCenterModel {
    type CommandOutput = ();
    type Input = ControlCenterInput;
    type Output = ControlCenterOutput;
    type Init = ControlCenterInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "control-center-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_tooltip_text: Some("Control Center"),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(ControlCenterInput::Clicked);
                },

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    gtk::Image {
                        set_icon_name: Some("margo-symbolic"),
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ControlCenterModel {
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ControlCenterInput::Clicked => {
                let _ = sender.output(ControlCenterOutput::Clicked);
            }
        }
    }
}
