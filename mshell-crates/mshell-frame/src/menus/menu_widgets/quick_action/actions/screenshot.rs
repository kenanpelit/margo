use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotModel {}

#[derive(Debug)]
pub(crate) enum ScreenshotInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ScreenshotOutput {
    /// Close the dashboard and open the screenshot menu.
    OpenMenu,
}

pub(crate) struct ScreenshotInit {}

#[relm4::component(pub)]
impl SimpleComponent for ScreenshotModel {
    type Input = ScreenshotInput;
    type Output = ScreenshotOutput;
    type Init = ScreenshotInit;

    view! {
        #[root]
        gtk::Box {
            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-button-medium"],
                set_hexpand: false,
                set_vexpand: false,
                set_tooltip_text: Some("Screenshot"),
                connect_clicked[sender] => move |_| {
                    sender.input(ScreenshotInput::Clicked);
                },

                #[name = "action_icon_image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("applets-screenshooter-symbolic"),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ScreenshotModel {};
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            ScreenshotInput::Clicked => {
                // The frame closes the dashboard, then opens the screenshot menu
                // on the same output (in-process, no `mshellctl` fork).
                let _ = sender.output(ScreenshotOutput::OpenMenu);
            }
        }
    }
}
