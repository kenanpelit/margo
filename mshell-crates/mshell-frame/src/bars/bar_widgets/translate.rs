//! Bar translate pill — opens the Translate menu. Static icon, no
//! live service (unlike e.g. bluetooth/audio); the menu does all the
//! work on open/keybind trigger.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct TranslateModel {
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum TranslateInput {}

#[derive(Debug)]
pub(crate) enum TranslateOutput {
    Clicked,
}

pub(crate) struct TranslateInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for TranslateModel {
    type Input = TranslateInput;
    type Output = TranslateOutput;
    type Init = TranslateInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "translate-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.output(TranslateOutput::Clicked).unwrap_or_default();
                },

                #[name="image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("preferences-desktop-locale-symbolic"),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = TranslateModel {
            orientation: params.orientation,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }
}
