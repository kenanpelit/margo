use mshell_utils::picker::spawn_color_picker;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ColorPickerModel {
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ColorPickerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ColorPickerOutput {}

pub(crate) struct ColorPickerInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for ColorPickerModel {
    type Input = ColorPickerInput;
    type Output = ColorPickerOutput;
    type Init = ColorPickerInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "hypr-picker-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(ColorPickerInput::Clicked);
                },

                #[name="image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("color-select-symbolic"),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ColorPickerModel {
            orientation: params.orientation,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            ColorPickerInput::Clicked => spawn_color_picker(0),
        }
    }
}
