//! AI assistant bar pill. A simple launcher: click opens the native AI chat
//! menu (`MenuType::Ai`) via `AiOutput::Clicked` → `BarOutput::AiClicked`,
//! exactly like the other menu pills. The chat + provider config live in the
//! menu / Settings → AI.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct AiModel {}

#[derive(Debug)]
pub(crate) enum AiInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum AiOutput {
    Clicked,
}

pub(crate) struct AiInit {}

#[relm4::component(pub(crate))]
impl Component for AiModel {
    type CommandOutput = ();
    type Input = AiInput;
    type Output = AiOutput;
    type Init = AiInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "ai-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_tooltip_text: Some("AI assistant"),

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| sender.input(AiInput::Clicked),

                gtk::Image {
                    set_icon_name: Some("starred-symbolic"),
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = AiModel {};
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AiInput::Clicked => {
                let _ = sender.output(AiOutput::Clicked);
            }
        }
    }
}
