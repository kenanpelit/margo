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
use crate::menus::menu_widgets::session::session_menu_widget::{
    SessionMenuWidgetInput, SessionMenuWidgetModel,
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
    /// Combined clock + quick-settings dashboard. Renders the
    /// hero clock card on top, then calendar + weather + the
    /// full QS stack underneath. Coexists with `Clock` and
    /// `QuickSettings`; users wire a keybind / bar pill if they
    /// prefer the combined view.
    Dashboard,
    /// Margo layout switcher. Replaces the legacy in-bar
    /// `gtk::PopoverMenu` (xdg_popup, detached window feel)
    /// with a regular menu surface that slides out from the
    /// bar like every other menu.
    MargoLayout,
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
    /// Maximum visible content height in pixels. 0 = no cap
    /// (legacy "grow to fit children" behaviour). When > 0, the
    /// outer ScrolledWindow caps the viewport at this value and
    /// the inner content scrolls vertically. Maps onto GTK's
    /// `set_max_content_height` — works as advertised here
    /// because `vscrollbar_policy` is Automatic.
    maximum_height: i32,
    css_class: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MenuInput {
    RevealChanged(bool),
    SetWidget(Vec<MenuWidget>),
    SetMinimumWidth(i32),
    SetMaximumHeight(i32),
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
            // Pin the viewport to exactly `minimum_width` on both
            // axes (min_content_width = max_content_width = w).
            // `set_width_request` alone is just a floor; the
            // ScrolledWindow would still grow if any nested
            // widget reported a larger natural width (the launcher
            // result list does — long row names + the binds-strip
            // footer push the natural well past 720). Clamping the
            // *content area* with min == max gives GTK a hard
            // outer dimension regardless of what the child wants,
            // and makes the Settings → Menus minimum-width spinner
            // actually shrink the panel.
            #[watch]
            set_width_request: model.minimum_width,
            #[watch]
            set_min_content_width: model.minimum_width,
            #[watch]
            set_max_content_width: model.minimum_width,
            set_propagate_natural_width: false,
            // Vertical height cap. 0 (config default) maps to -1
            // ("no cap"), so legacy menus keep their grow-to-fit
            // behaviour unchanged. When the user sets a positive
            // value, GTK clamps the viewport at that height and
            // the inner content scrolls — unlike the horizontal
            // axis, this one actually works because
            // `vscrollbar_policy` is Automatic (GTK's
            // `min/max_content_*` are no-ops only with the Never
            // policy, see gtkscrolledwindow.c:1896).
            #[watch]
            set_max_content_height: if model.maximum_height > 0 {
                model.maximum_height
            } else {
                -1
            },

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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().clock_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().clipboard_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().quick_settings_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().notification_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().screenshot_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().app_launcher_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().wallpaper_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().nufw_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().ndns_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().npodman_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().nnotes_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().nip_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().nnetwork_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().npower_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().media_player_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
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
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().session_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Dashboard => {
                // Same card-stack CSS as quick-settings — dashboard
                // reuses the .quick-settings-menu class so all the
                // surface-variant card + hero clock rules apply.
                css_class = "quick-settings-menu dashboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().dashboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().dashboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().dashboard_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::MargoLayout => {
                css_class = "margo-layout-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().margo_layout_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().margo_layout_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().margo_layout_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
        }

        let model = MenuModel {
            widget_controllers: Vec::new(),
            widget_kinds: Vec::new(),
            minimum_width: 410,
            maximum_height: 0,
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
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<SessionMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(SessionMenuWidgetInput::ParentRevealChanged(visible))
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
            MenuInput::SetMaximumHeight(height) => {
                self.maximum_height = height;
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
