use mshell_utils::picker::spawn_color_picker;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ColorPickerModel {}

#[derive(Debug)]
pub(crate) enum ColorPickerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ColorPickerOutput {
    CloseMenu,
}

pub(crate) struct ColorPickerInit {}

#[relm4::component(pub)]
impl SimpleComponent for ColorPickerModel {
    type Input = ColorPickerInput;
    type Output = ColorPickerOutput;
    type Init = ColorPickerInit;

    view! {
        #[root]
        gtk::Box {
            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-button-medium"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(ColorPickerInput::Clicked);
                },

                #[name = "action_icon_image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("color-select-symbolic"),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ColorPickerModel {};

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            ColorPickerInput::Clicked => {
                let _ = sender.output(ColorPickerOutput::CloseMenu);
                spawn_color_picker(300);
            }
        }
    }
}
