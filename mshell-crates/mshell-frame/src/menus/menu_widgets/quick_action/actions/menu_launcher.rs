//! A generic quick-action button that opens another shell menu.
//!
//! Used by `mdash`'s bottom shortcut grid: each button carries a menu
//! name (the `mshellctl menu <name>` subcommand), an icon, and a tooltip.
//! Clicking it toggles that menu and closes the dashboard it lives in.
//!
//! Dispatch is via spawning `mshellctl menu <name>` — the shell talking to
//! its own D-Bus service. That keeps this button decoupled from the
//! frame/menu-stack internals (no cross-component output plumbing for each
//! of the dozen-plus target menus); `mshellctl` is always on PATH as part
//! of the install. The child is reaped on a detached thread so repeated
//! clicks don't leave zombies in the long-lived shell process.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

pub(crate) struct MenuLauncherModel {
    /// `mshellctl menu <menu>` subcommand name (kebab-case).
    menu: String,
    icon: String,
    tooltip: String,
}

#[derive(Debug)]
pub(crate) enum MenuLauncherInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum MenuLauncherOutput {
    CloseMenu,
}

pub(crate) struct MenuLauncherInit {
    pub menu: String,
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
                if let Ok(mut child) = std::process::Command::new("mshellctl")
                    .arg("menu")
                    .arg(&self.menu)
                    .spawn()
                {
                    // Reap on a detached thread so the long-lived shell
                    // doesn't accumulate zombies across clicks.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                // Close the dashboard so the target menu isn't left
                // sitting behind it in another screen region.
                let _ = sender.output(MenuLauncherOutput::CloseMenu);
            }
        }
    }
}
