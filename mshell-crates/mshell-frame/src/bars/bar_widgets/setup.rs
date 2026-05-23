//! Bar pill that opens the Settings panel straight to the Setup page —
//! the in-shell setup wizard. It's a layer-shell surface (the Settings
//! panel itself), not a separate floating window, so it behaves exactly
//! like every other menu: click the pill, the panel slides in focused.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct SetupModel {
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SetupInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum SetupOutput {
    CloseMenu,
}

pub(crate) struct SetupInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for SetupModel {
    type Input = SetupInput;
    type Output = SetupOutput;
    type Init = SetupInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "setup-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                set_tooltip_text: Some("Setup"),
                connect_clicked[sender] => move |_| {
                    sender.input(SetupInput::Clicked);
                },

                #[name = "image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("emblem-system-symbolic"),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = SetupModel {
            orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            SetupInput::Clicked => {
                let _ = sender.output(SetupOutput::CloseMenu);
                // Opens the Settings panel (a layer-shell menu) at the
                // Setup section — the in-shell wizard, no floating window.
                mshell_settings::open_settings_at_section("setup");
            }
        }
    }
}
