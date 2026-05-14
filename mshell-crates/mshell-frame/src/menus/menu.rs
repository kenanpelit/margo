use crate::menus::builder::build_widget;
use crate::menus::menu_widgets::app_launcher::app_launcher::{AppLauncherInput, AppLauncherModel};
use crate::menus::menu_widgets::audio_in::audio_in_menu_widget::{
    AudioInMenuWidgetInput, AudioInMenuWidgetModel,
};
use crate::menus::menu_widgets::audio_out::audio_out_menu_widget::{
    AudioOutMenuWidgetInput, AudioOutMenuWidgetModel,
};
use crate::menus::menu_widgets::bluetooth::bluetooth_menu_widget::{
    BluetoothMenuWidgetInput, BluetoothMenuWidgetModel,
};
use crate::menus::menu_widgets::network::network_menu_widget::{
    NetworkMenuWidgetInput, NetworkMenuWidgetModel,
};
use crate::menus::menu_widgets::power_profile::power_profile_menu_widget::{
    PowerProfileMenuWidgetInput, PowerProfileMenuWidgetModel,
};
use crate::menus::menu_widgets::screenshare::screenshare_menu_widget::{
    ScreenshareMenuWidgetInit, ScreenshareMenuWidgetInput, ScreenshareMenuWidgetModel,
    ScreenshareMenuWidgetOutput,
};
use crate::menus::menu_widgets::wallpaper::wallpaper_menu_widget::{
    WallpaperMenuWidgetInput, WallpaperMenuWidgetModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::clear_box::clear_box;
use reactive_graph::traits::Get;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt,
    gtk, gtk::prelude::*,
};
use std::fmt::Debug;

pub(crate) enum MenuType {
    Clipboard,
    Clock,
    Notifications,
    QuickSettings,
    Screenshot,
    AppLauncher,
    Wallpaper,
    HyprlandScreenshare,
    Nufw,
    Ndns,
    Npodman,
    Nnotes,
    Nip,
    Nnetwork,
    Npower,
    MediaPlayer,
    Session,
}

pub(crate) struct MenuModel {
    widget_controllers: Vec<Box<dyn GenericWidgetController>>,
    // The `MenuWidget` kinds backing `widget_controllers`, so
    // `SetWidget` can skip the destructive clear+rebuild when the
    // config layer re-notifies with an identical list. The config
    // store is coarse — a write to any field reaches every effect
    // bound to it — so without this guard every unrelated config
    // touch tears down and recreates each menu's content widgets,
    // which silently re-runs their probe loops (ndns / nufw /
    // npodman shell out on init). Mirrors the bar's guard.
    widget_kinds: Vec<MenuWidget>,
    minimum_width: i32,
    css_class: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MenuInput {
    RevealChanged(bool),
    SetWidget(Vec<MenuWidget>),
    SetMinimumWidth(i32),
    AddHyprlandScreenshareWidget,
    ForwardHyprlandScreenshareReply(tokio::sync::oneshot::Sender<String>, String),
}

#[derive(Debug)]
pub(crate) enum MenuOutput {
    CloseMenu,
}

pub(crate) struct MenuInit {
    pub(crate) menu_type: MenuType,
}

#[relm4::component(pub)]
impl Component for MenuModel {
    type CommandOutput = ();
    type Input = MenuInput;
    type Output = MenuOutput;
    type Init = MenuInit;

    view! {
        #[root]
        #[name = "scrolled_window"]
        gtk::ScrolledWindow {
            set_css_classes: &["menu-scroll-window", model.css_class.as_str()],
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: true,
            #[watch]
            set_width_request: model.minimum_width,
            set_propagate_natural_width: false,

            #[name = "widget_container"]
            gtk::Box {
                set_margin_all: 20,
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: false,
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        let mut effects = EffectScope::new();

        let css_class: String;

        match params.menu_type {
            MenuType::Clock => {
                css_class = "clock-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().clock_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().clock_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Clipboard => {
                css_class = "clipboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().clipboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().clipboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::QuickSettings => {
                css_class = "quick-settings-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().quick_settings_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().quick_settings_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Notifications => {
                css_class = "notifications-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().notification_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().notification_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Screenshot => {
                css_class = "screenshot-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().screenshot_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().screenshot_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::AppLauncher => {
                css_class = "app-launcher-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().app_launcher_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().app_launcher_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Wallpaper => {
                css_class = "wallpaper-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().wallpaper_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().wallpaper_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::HyprlandScreenshare => {
                css_class = "hyprland-screenshare-menu".to_string();
                sender.input(MenuInput::AddHyprlandScreenshareWidget);
            }
            MenuType::Nufw => {
                css_class = "nufw-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().nufw_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().nufw_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Ndns => {
                css_class = "ndns-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().ndns_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().ndns_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Npodman => {
                css_class = "npodman-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().npodman_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().npodman_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Nnotes => {
                css_class = "nnotes-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().nnotes_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().nnotes_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Nip => {
                css_class = "nip-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().nip_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().nip_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Nnetwork => {
                css_class = "nnetwork-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().nnetwork_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().nnetwork_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Npower => {
                css_class = "npower-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().npower_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().npower_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::MediaPlayer => {
                css_class = "media-player-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().media_player_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().media_player_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
            MenuType::Session => {
                css_class = "session-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().session_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().session_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
            }
        }

        let model = MenuModel {
            widget_controllers: Vec::new(),
            widget_kinds: Vec::new(),
            minimum_width: 410,
            css_class,
            _effects: effects,
        };

        let widgets = view_output!();

        if let MenuType::Wallpaper = params.menu_type {
            widgets.widget_container.set_margin_all(8);
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MenuInput::RevealChanged(visible) => {
                // Let widgets that care know they are being revealed
                for controller in &self.widget_controllers {
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AppLauncherModel>>()
                    {
                        controller
                            .sender()
                            .send(AppLauncherInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<NetworkMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(NetworkMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<BluetoothMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(BluetoothMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AudioOutMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(AudioOutMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AudioInMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(AudioInMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<PowerProfileMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(PowerProfileMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<ScreenshareMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(ScreenshareMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<WallpaperMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(WallpaperMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                }
            }
            MenuInput::SetWidget(menu_widgets) => {
                // Skip the destructive clear+rebuild when the config
                // layer re-notifies with an identical widget list —
                // see the `widget_kinds` field comment.
                if self.widget_kinds != menu_widgets {
                    clear_box(&widgets.widget_container);
                    self.widget_controllers.clear();
                    for item in &menu_widgets {
                        let controller =
                            build_widget(item, gtk::Orientation::Vertical, &sender);
                        widgets.widget_container.append(&controller.root_widget());
                        self.widget_controllers.push(controller);
                    }
                    self.widget_kinds = menu_widgets;
                }
            }
            MenuInput::SetMinimumWidth(width) => {
                self.minimum_width = width;
            }
            MenuInput::AddHyprlandScreenshareWidget => {
                let controller = Box::new(
                    ScreenshareMenuWidgetModel::builder()
                        .launch(ScreenshareMenuWidgetInit {})
                        .forward(sender.output_sender(), |msg| match msg {
                            ScreenshareMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                        }),
                );
                widgets.widget_container.append(&controller.root_widget());
                self.widget_controllers.push(controller);
            }
            MenuInput::ForwardHyprlandScreenshareReply(reply, payload) => {
                if let Some(first_controller) = self.widget_controllers.first()
                    && let Some(controller) =
                        first_controller.downcast_ref::<Controller<ScreenshareMenuWidgetModel>>()
                {
                    controller
                        .sender()
                        .send(ScreenshareMenuWidgetInput::SetReply(reply, payload))
                        .ok();
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

impl Debug for MenuModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuModel").finish()
    }
}
