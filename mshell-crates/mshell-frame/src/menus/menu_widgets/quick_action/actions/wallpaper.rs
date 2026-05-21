use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct WallpaperModel {}

#[derive(Debug)]
pub(crate) enum WallpaperInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum WallpaperOutput {
    CloseMenu,
}

pub(crate) struct WallpaperInit {}

#[relm4::component(pub)]
impl SimpleComponent for WallpaperModel {
    type Input = WallpaperInput;
    type Output = WallpaperOutput;
    type Init = WallpaperInit;

    view! {
        #[root]
        gtk::Box {
            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-button-medium"],
                set_hexpand: false,
                set_vexpand: false,
                set_tooltip_text: Some("Wallpaper"),
                connect_clicked[sender] => move |_| {
                    sender.input(WallpaperInput::Clicked);
                },

                #[name = "action_icon_image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("preferences-desktop-wallpaper-symbolic"),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = WallpaperModel {};
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            WallpaperInput::Clicked => {
                // Close the dashboard, then open the wallpaper menu the same
                // way `mshellctl menu wallpaper` does (toggles it on the
                // active monitor).
                let _ = sender.output(WallpaperOutput::CloseMenu);
                let _ = std::process::Command::new("mshellctl")
                    .args(["menu", "wallpaper"])
                    .spawn();
            }
        }
    }
}
