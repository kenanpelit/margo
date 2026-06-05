use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

/// On-screen keyboard pill: a single button that toggles `mkeys`.
#[derive(Debug, Clone)]
pub(crate) struct KeyboardModel {
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeyboardInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum KeyboardOutput {
    CloseMenu,
}

pub(crate) struct KeyboardInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for KeyboardModel {
    type Input = KeyboardInput;
    type Output = KeyboardOutput;
    type Init = KeyboardInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "keyboard-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                set_tooltip_text: Some("Toggle the on-screen keyboard"),
                connect_clicked[sender] => move |_| {
                    sender.input(KeyboardInput::Clicked);
                },

                #[name="image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("input-keyboard-symbolic"),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = KeyboardModel {
            orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            KeyboardInput::Clicked => {
                let _ = sender.output(KeyboardOutput::CloseMenu);
                // `mkeys toggle`: starts the keyboard if hidden, hides it if
                // shown. Detached in a tokio task so it never blocks the bar.
                relm4::spawn(async move {
                    let _ = tokio::process::Command::new("mkeys")
                        .arg("toggle")
                        .status()
                        .await;
                });
            }
        }
    }
}
