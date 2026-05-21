//! Keybinds — bar pill that opens the keybind cheatsheet.
//!
//! A plain keyboard glyph; click opens the cheatsheet menu (parsed
//! live from `config.conf`). Stateless — there's nothing to poll.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct KeybindsModel {
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeybindsInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum KeybindsOutput {
    Clicked,
}

pub(crate) struct KeybindsInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for KeybindsModel {
    type CommandOutput = ();
    type Input = KeybindsInput;
    type Output = KeybindsOutput;
    type Init = KeybindsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "keybinds-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_tooltip_text: Some("Keyboard shortcuts"),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(KeybindsInput::Clicked);
                },

                gtk::Image {
                    set_icon_name: Some("input-keyboard-symbolic"),
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = KeybindsModel {
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            KeybindsInput::Clicked => {
                let _ = sender.output(KeybindsOutput::Clicked);
            }
        }
    }
}
