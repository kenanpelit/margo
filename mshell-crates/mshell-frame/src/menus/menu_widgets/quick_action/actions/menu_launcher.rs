//! A generic quick-action button that opens another shell menu.
//!
//! Used by `mdash`'s bottom shortcut grid: each button carries a target
//! [`MenuId`], an icon, and a tooltip. Clicking it closes the dashboard it
//! lives in and opens that menu on the same frame — an in-process output
//! (`OpenMenu` → `MenuOutput::OpenMenu` → `FrameInput::SwitchToMenu`), not a
//! `mshellctl menu <name>` fork that round-tripped through the shell's own
//! D-Bus service.

use crate::frame::MenuId;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

pub(crate) struct MenuLauncherModel {
    /// The menu this button opens.
    menu: MenuId,
    icon: String,
    tooltip: String,
}

#[derive(Debug)]
pub(crate) enum MenuLauncherInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum MenuLauncherOutput {
    /// Close the dashboard and open this menu.
    OpenMenu(MenuId),
}

pub(crate) struct MenuLauncherInit {
    pub menu: MenuId,
    pub icon: String,
    pub tooltip: String,
}

#[relm4::component(pub)]
impl SimpleComponent for MenuLauncherModel {
    type Input = MenuLauncherInput;
    type Output = MenuLauncherOutput;
    type Init = MenuLauncherInit;

    view! {
        #[root]
        gtk::Box {
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-button-medium"],
                set_hexpand: false,
                set_vexpand: false,
                set_tooltip_text: Some(model.tooltip.as_str()),
                connect_clicked[sender] => move |_| {
                    sender.input(MenuLauncherInput::Clicked);
                },

                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some(model.icon.as_str()),
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = MenuLauncherModel {
            menu: params.menu,
            icon: params.icon,
            tooltip: params.tooltip,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            MenuLauncherInput::Clicked => {
                // The frame closes the dashboard first, so the target menu
                // isn't left sitting behind it in another screen region.
                let _ = sender.output(MenuLauncherOutput::OpenMenu(self.menu));
            }
        }
    }
}
