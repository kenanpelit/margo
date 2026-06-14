use crate::menus::menu_widgets::quick_action::actions::airplane_mode::{
    AirplaneModeInit, AirplaneModeModel,
};
use crate::menus::menu_widgets::quick_action::actions::color_picker::{
    ColorPickerInit, ColorPickerModel, ColorPickerOutput,
};
use crate::menus::menu_widgets::quick_action::actions::do_not_disturb::{
    DoNotDisturbInit, DoNotDisturbModel,
};
use crate::menus::menu_widgets::quick_action::actions::idle_inhibitor::{
    IdleInhibitorInit, IdleInhibitorModel,
};
use crate::menus::menu_widgets::quick_action::actions::lock::{LockInit, LockModel, LockOutput};
use crate::menus::menu_widgets::quick_action::actions::logout::{LogoutInit, LogoutModel};
use crate::menus::menu_widgets::quick_action::actions::menu_launcher::{
    MenuLauncherInit, MenuLauncherModel, MenuLauncherOutput,
};
use crate::menus::menu_widgets::quick_action::actions::night_light::{
    NightLightInit, NightLightModel,
};
use crate::menus::menu_widgets::quick_action::actions::reboot::{RebootInit, RebootModel};
use crate::menus::menu_widgets::quick_action::actions::screenshot::{
    ScreenshotInit, ScreenshotModel, ScreenshotOutput,
};
use crate::menus::menu_widgets::quick_action::actions::settings::{SettingsInit, SettingsModel};
use crate::menus::menu_widgets::quick_action::actions::shutdown::{ShutdownInit, ShutdownModel};
use crate::menus::menu_widgets::quick_action::actions::wallpaper::{
    WallpaperInit, WallpaperModel, WallpaperOutput,
};
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_config::schema::menu_widgets::{QuickActionWidget, QuickActionsConfig};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct QuickActionsModel {
    _widget_controllers: Vec<Box<dyn GenericWidgetController>>,
}

#[derive(Debug)]
pub(crate) enum QuickActionsInput {}

#[derive(Debug)]
pub(crate) enum QuickActionsOutput {
    CloseMenu,
}

pub(crate) struct QuickActionsInit {
    pub config: QuickActionsConfig,
}

#[derive(Debug)]
pub(crate) enum QuickActionsCommandOutput {}

#[relm4::component(pub)]
impl Component for QuickActionsModel {
    type CommandOutput = QuickActionsCommandOutput;
    type Input = QuickActionsInput;
    type Output = QuickActionsOutput;
    type Init = QuickActionsInit;

    view! {
        #[root]
        #[name = "quick_actions_container"]
        gtk::Box {
            add_css_class: "quick-actions-menu-widget",
            set_orientation: gtk::Orientation::Horizontal,
            set_align: gtk::Align::Center,
            set_spacing: 12,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut widget_controllers: Vec<Box<dyn GenericWidgetController>> = Vec::new();

        let widgets = view_output!();

        params.config.widgets.iter().for_each(|widget| {
            let controller = Self::build_widget(widget, &sender);
            widgets
                .quick_actions_container
                .append(&controller.root_widget());
            widget_controllers.push(controller);
        });

        let model = QuickActionsModel {
            _widget_controllers: widget_controllers,
        };

        ComponentParts { model, widgets }
    }
}

impl QuickActionsModel {
    fn build_widget(
        widget: &QuickActionWidget,
        sender: &ComponentSender<Self>,
    ) -> Box<dyn GenericWidgetController> {
        match widget {
            QuickActionWidget::AirplaneMode => Box::new(
                AirplaneModeModel::builder()
                    .launch(AirplaneModeInit {})
                    .detach(),
            ),
            QuickActionWidget::DoNotDisturb => Box::new(
                DoNotDisturbModel::builder()
                    .launch(DoNotDisturbInit {})
                    .detach(),
            ),
            QuickActionWidget::ColorPicker => Box::new(
                ColorPickerModel::builder()
                    .launch(ColorPickerInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        ColorPickerOutput::CloseMenu => QuickActionsOutput::CloseMenu,
                    }),
            ),
            QuickActionWidget::IdleInhibitor => Box::new(
                IdleInhibitorModel::builder()
                    .launch(IdleInhibitorInit {})
                    .detach(),
            ),
            QuickActionWidget::Lock => Box::new(LockModel::builder().launch(LockInit {}).forward(
                sender.output_sender(),
                |msg| match msg {
                    LockOutput::CloseMenu => QuickActionsOutput::CloseMenu,
                },
            )),
            QuickActionWidget::Logout => {
                Box::new(LogoutModel::builder().launch(LogoutInit {}).detach())
            }
            QuickActionWidget::Nightlight => Box::new(
                NightLightModel::builder()
                    .launch(NightLightInit {})
                    .detach(),
            ),
            QuickActionWidget::Reboot => {
                Box::new(RebootModel::builder().launch(RebootInit {}).detach())
            }
            QuickActionWidget::Wallpaper => {
                Box::new(WallpaperModel::builder().launch(WallpaperInit {}).forward(
                    sender.output_sender(),
                    |msg| match msg {
                        WallpaperOutput::CloseMenu => QuickActionsOutput::CloseMenu,
                    },
                ))
            }
            QuickActionWidget::Screenshot => Box::new(
                ScreenshotModel::builder()
                    .launch(ScreenshotInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        ScreenshotOutput::CloseMenu => QuickActionsOutput::CloseMenu,
                    }),
            ),
            QuickActionWidget::Settings => {
                // No output to forward — the Settings button toggles
                // the embedded Settings menu directly via
                // `open_settings()`, which also hides this menu.
                Box::new(SettingsModel::builder().launch(SettingsInit {}).detach())
            }
            QuickActionWidget::Shutdown => {
                Box::new(ShutdownModel::builder().launch(ShutdownInit {}).detach())
            }
            // ── Menu-launcher buttons ──
            QuickActionWidget::Network => {
                Self::menu_btn(sender, "network", "network-wireless-symbolic", "Network")
            }
            QuickActionWidget::Bluetooth => {
                Self::menu_btn(sender, "bluetooth", "bluetooth-symbolic", "Bluetooth")
            }
            QuickActionWidget::CpuDashboard => Self::menu_btn(
                sender,
                "cpu-dashboard",
                "utilities-system-monitor-symbolic",
                "CPU dashboard",
            ),
            QuickActionWidget::AudioDashboard => Self::menu_btn(
                sender,
                "audio-dashboard",
                "audio-volume-high-symbolic",
                "Audio dashboard",
            ),
            QuickActionWidget::Vpn => Self::menu_btn(sender, "vpn", "network-vpn-symbolic", "VPN"),
            QuickActionWidget::ControlCenter => Self::menu_btn(
                sender,
                "control-center",
                "preferences-system-symbolic",
                "Control center",
            ),
            QuickActionWidget::Twilight => {
                Self::menu_btn(sender, "twilight", "night-light-symbolic", "Twilight")
            }
            QuickActionWidget::Keybinds => {
                Self::menu_btn(sender, "keybinds", "input-keyboard-symbolic", "Keybinds")
            }
            QuickActionWidget::Dns => {
                Self::menu_btn(sender, "dns", "network-server-symbolic", "DNS")
            }
            QuickActionWidget::Power => {
                Self::menu_btn(sender, "power", "battery-symbolic", "Power")
            }
            QuickActionWidget::Session => {
                Self::menu_btn(sender, "session", "system-shutdown-symbolic", "Session")
            }
            QuickActionWidget::Ip => Self::menu_btn(sender, "ip", "network-wired-symbolic", "IP"),
            QuickActionWidget::AlarmClock => {
                Self::menu_btn(sender, "alarm-clock", "alarm-symbolic", "Alarm clock")
            }
            QuickActionWidget::SystemUpdate => Self::menu_btn(
                sender,
                "system-update",
                "software-update-available-symbolic",
                "System updates",
            ),
        }
    }

    /// Build a menu-launcher button: an icon button that opens
    /// `mshellctl menu <menu>` and closes the dashboard it lives in.
    fn menu_btn(
        sender: &ComponentSender<Self>,
        menu: &str,
        icon: &str,
        tooltip: &str,
    ) -> Box<dyn GenericWidgetController> {
        Box::new(
            MenuLauncherModel::builder()
                .launch(MenuLauncherInit {
                    menu: menu.to_string(),
                    icon: icon.to_string(),
                    tooltip: tooltip.to_string(),
                })
                .forward(sender.output_sender(), |msg| match msg {
                    MenuLauncherOutput::CloseMenu => QuickActionsOutput::CloseMenu,
                }),
        )
    }
}
