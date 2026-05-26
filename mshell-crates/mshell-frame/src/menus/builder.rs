use crate::menus::menu::{MenuModel, MenuOutput};
use crate::menus::menu_widgets::alarm_clock::alarm_clock_menu_widget::{
    AlarmClockMenuWidgetInit, AlarmClockMenuWidgetModel,
};
use crate::menus::menu_widgets::app_launcher::app_launcher::{
    AppLauncherInit, AppLauncherModel, AppLauncherOutput,
};
use crate::menus::menu_widgets::audio_dashboard::audio_dashboard_menu_widget::{
    AudioDashboardMenuWidgetInit, AudioDashboardMenuWidgetModel,
};
use crate::menus::menu_widgets::audio_in::audio_in_menu_widget::{
    AudioInMenuWidgetInit, AudioInMenuWidgetModel,
};
use crate::menus::menu_widgets::audio_out::audio_out_menu_widget::{
    AudioOutMenuWidgetInit, AudioOutMenuWidgetModel,
};
use crate::menus::menu_widgets::bluetooth::bluetooth_menu_widget::{
    BluetoothMenuWidgetInit, BluetoothMenuWidgetModel,
};
use crate::menus::menu_widgets::calendar::{CalendarInit, CalendarModel};
use crate::menus::menu_widgets::calendar_grid::{CalendarGridInit, CalendarGridModel};
use crate::menus::menu_widgets::clipboard::clipboard::{
    ClipboardInit, ClipboardModel, ClipboardOutput,
};
use crate::menus::menu_widgets::clock::{ClockInit, ClockModel};
use crate::menus::menu_widgets::compact_audio::{CompactAudioInit, CompactAudioModel};
use crate::menus::menu_widgets::connectivity::{ConnectivityInit, ConnectivityModel};
use crate::menus::menu_widgets::container::{ContainerInit, ContainerModel};
use crate::menus::menu_widgets::cpu_dashboard::cpu_dashboard_menu_widget::{
    CpuDashboardMenuWidgetInit, CpuDashboardMenuWidgetModel,
};
use crate::menus::menu_widgets::divider::{DividerMenuWidgetInit, DividerMenuWidgetModel};
use crate::menus::menu_widgets::margo_layout::margo_layout_menu_widget::{
    MargoLayoutMenuWidgetInit, MargoLayoutMenuWidgetModel, MargoLayoutMenuWidgetOutput,
};
use crate::menus::menu_widgets::media_player::media_players::{
    MediaPlayersInit, MediaPlayersModel,
};
use crate::menus::menu_widgets::dns::dns_menu_widget::{
    DnsMenuWidgetInit, DnsMenuWidgetModel,
};
use crate::menus::menu_widgets::network_toggle::network_menu_widget::{
    NetworkToggleMenuWidgetInit, NetworkToggleMenuWidgetModel,
};
use crate::menus::menu_widgets::ip::ip_menu_widget::{
    IpMenuWidgetInit, IpMenuWidgetModel,
};
use crate::menus::menu_widgets::network::network_menu_widget::{
    NetworkMenuWidgetInit, NetworkMenuWidgetModel,
};
use crate::menus::menu_widgets::notes::notes_menu_widget::{
    NotesMenuWidgetInit, NotesMenuWidgetModel,
};
use crate::menus::menu_widgets::power::power_menu_widget::{
    PowerMenuWidgetInit, PowerMenuWidgetModel,
};
use crate::menus::menu_widgets::notifications::notifications::{
    NotificationsInit, NotificationsModel, NotificationsOutput,
};
use crate::menus::menu_widgets::podman::podman_menu_widget::{
    PodmanMenuWidgetInit, PodmanMenuWidgetModel,
};
use crate::menus::menu_widgets::ufw::ufw_menu_widget::{
    UfwMenuWidgetInit, UfwMenuWidgetModel,
};
use crate::menus::menu_widgets::overview_intel::{OverviewIntelInit, OverviewIntelModel};
use crate::menus::menu_widgets::panel_header::{PanelHeaderInit, PanelHeaderModel};
use crate::menus::menu_widgets::quick_action::quick_actions::{
    QuickActionsInit, QuickActionsModel, QuickActionsOutput,
};
use crate::menus::menu_widgets::screen_record::screen_record_menu_widget::{
    ScreenRecordMenuWidgetInit, ScreenRecordMenuWidgetModel, ScreenRecordMenuWidgetOutput,
};
use crate::menus::menu_widgets::screenshot::screenshot_menu_widget::{
    ScreenshotMenuWidgetInit, ScreenshotMenuWidgetModel, ScreenshotMenuWidgetOutput,
};
use crate::menus::menu_widgets::session::session_menu_widget::{
    SessionMenuWidgetInit, SessionMenuWidgetModel, SessionMenuWidgetOutput,
};
use crate::menus::menu_widgets::spacer::{SpacerInit, SpacerModel};
use crate::menus::menu_widgets::system_status::{SystemStatusInit, SystemStatusModel};
use crate::menus::menu_widgets::system_update::system_update_menu_widget::{
    SystemUpdateMenuWidgetInit, SystemUpdateMenuWidgetModel, SystemUpdateMenuWidgetOutput,
};
use crate::menus::menu_widgets::valent::valent_menu_widget::{
    ValentMenuWidgetInit, ValentMenuWidgetModel, ValentMenuWidgetOutput,
};
use crate::menus::menu_widgets::keep_awake::keep_awake_menu_widget::{
    KeepAwakeMenuWidgetInit, KeepAwakeMenuWidgetModel, KeepAwakeMenuWidgetOutput,
};
use crate::menus::menu_widgets::keybinds::keybinds_menu_widget::{
    KeybindsMenuWidgetInit, KeybindsMenuWidgetModel,
};
use crate::menus::menu_widgets::ssh_sessions::ssh_sessions_menu_widget::{
    SshSessionsMenuWidgetInit, SshSessionsMenuWidgetModel,
};
use crate::menus::menu_widgets::twilight::twilight_menu_widget::{
    TwilightMenuWidgetInit, TwilightMenuWidgetModel,
};
use crate::menus::menu_widgets::theme_picker::theme_picker_menu_widget::{
    ThemePickerMenuWidgetInit, ThemePickerMenuWidgetModel,
};
use crate::menus::menu_widgets::wallpaper::wallpaper_menu_widget::{
    WallpaperMenuWidgetInit, WallpaperMenuWidgetModel,
};
use crate::menus::menu_widgets::weather::weather::{WeatherInit, WeatherModel};
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::{Component, ComponentSender, gtk};

pub fn build_widget(
    menu_widget: &MenuWidget,
    orientation: gtk::Orientation,
    sender: &ComponentSender<MenuModel>,
    // Only consulted by `MenuWidget::Weather`: true for the standalone
    // weather menu (all sections stacked), false for the dashboard embed
    // (the original compact paged view).
    weather_all_in_one: bool,
) -> Box<dyn GenericWidgetController> {
    match menu_widget {
        MenuWidget::AlarmClock => Box::new(
            AlarmClockMenuWidgetModel::builder()
                .launch(AlarmClockMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::AppLauncher => Box::new(
            AppLauncherModel::builder()
                .launch(AppLauncherInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    AppLauncherOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::AudioInput => Box::new(
            AudioInMenuWidgetModel::builder()
                .launch(AudioInMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::AudioDashboard => Box::new(
            AudioDashboardMenuWidgetModel::builder()
                .launch(AudioDashboardMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::AudioOutput => Box::new(
            AudioOutMenuWidgetModel::builder()
                .launch(AudioOutMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Bluetooth => Box::new(
            BluetoothMenuWidgetModel::builder()
                .launch(BluetoothMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Calendar => Box::new(CalendarModel::builder().launch(CalendarInit {}).detach()),
        MenuWidget::CalendarGrid => Box::new(
            CalendarGridModel::builder()
                .launch(CalendarGridInit {})
                .detach(),
        ),
        MenuWidget::Clipboard => {
            Box::new(ClipboardModel::builder().launch(ClipboardInit {}).forward(
                sender.output_sender(),
                |msg| match msg {
                    ClipboardOutput::CloseMenu => MenuOutput::CloseMenu,
                },
            ))
        }
        MenuWidget::Clock => Box::new(ClockModel::builder().launch(ClockInit {}).detach()),
        MenuWidget::CompactAudio => Box::new(
            CompactAudioModel::builder()
                .launch(CompactAudioInit {})
                .detach(),
        ),
        MenuWidget::Connectivity => Box::new(
            ConnectivityModel::builder()
                .launch(ConnectivityInit {})
                .detach(),
        ),
        MenuWidget::CpuDashboard => Box::new(
            CpuDashboardMenuWidgetModel::builder()
                .launch(CpuDashboardMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::SystemUpdate => Box::new(
            SystemUpdateMenuWidgetModel::builder()
                .launch(SystemUpdateMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    SystemUpdateMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Valent => Box::new(
            ValentMenuWidgetModel::builder()
                .launch(ValentMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    ValentMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::KeepAwake => Box::new(
            KeepAwakeMenuWidgetModel::builder()
                .launch(KeepAwakeMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    KeepAwakeMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Twilight => Box::new(
            TwilightMenuWidgetModel::builder()
                .launch(TwilightMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Keybinds => Box::new(
            KeybindsMenuWidgetModel::builder()
                .launch(KeybindsMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::SshSessions => Box::new(
            SshSessionsMenuWidgetModel::builder()
                .launch(SshSessionsMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Divider => Box::new(
            DividerMenuWidgetModel::builder()
                .launch(DividerMenuWidgetInit { orientation })
                .detach(),
        ),
        MenuWidget::MargoLayout => Box::new(
            MargoLayoutMenuWidgetModel::builder()
                .launch(MargoLayoutMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    MargoLayoutMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::MediaPlayer => Box::new(
            MediaPlayersModel::builder()
                .launch(MediaPlayersInit {})
                .detach(),
        ),
        MenuWidget::Dns => Box::new(
            DnsMenuWidgetModel::builder()
                .launch(DnsMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::NetworkToggle => Box::new(
            NetworkToggleMenuWidgetModel::builder()
                .launch(NetworkToggleMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Ip => Box::new(
            IpMenuWidgetModel::builder()
                .launch(IpMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Network => Box::new(
            NetworkMenuWidgetModel::builder()
                .launch(NetworkMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Notes => Box::new(
            NotesMenuWidgetModel::builder()
                .launch(NotesMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Power => Box::new(
            PowerMenuWidgetModel::builder()
                .launch(PowerMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Notifications => Box::new(
            NotificationsModel::builder()
                .launch(NotificationsInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    NotificationsOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Session => Box::new(
            SessionMenuWidgetModel::builder()
                .launch(SessionMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    SessionMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Podman => Box::new(
            PodmanMenuWidgetModel::builder()
                .launch(PodmanMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Ufw => Box::new(
            UfwMenuWidgetModel::builder()
                .launch(UfwMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::QuickActions(config) => Box::new(
            QuickActionsModel::builder()
                .launch(QuickActionsInit {
                    config: config.clone(),
                })
                .forward(sender.output_sender(), |msg| match msg {
                    QuickActionsOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Container(config) => Box::new(
            ContainerModel::builder()
                .launch(ContainerInit {
                    config: config.clone(),
                    menu_sender: sender.clone(),
                })
                .detach(),
        ),
        MenuWidget::Screenshots => Box::new(
            ScreenshotMenuWidgetModel::builder()
                .launch(ScreenshotMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    ScreenshotMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::ScreenRecording => Box::new(
            ScreenRecordMenuWidgetModel::builder()
                .launch(ScreenRecordMenuWidgetInit {})
                .forward(sender.output_sender(), |msg| match msg {
                    ScreenRecordMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                }),
        ),
        MenuWidget::Spacer(config) => Box::new(
            SpacerModel::builder()
                .launch(SpacerInit {
                    config: config.clone(),
                    orientation,
                })
                .detach(),
        ),
        MenuWidget::PanelHeader(config) => Box::new(
            PanelHeaderModel::builder()
                .launch(PanelHeaderInit {
                    title: config.title.clone(),
                })
                .detach(),
        ),
        MenuWidget::ThemePicker => Box::new(
            ThemePickerMenuWidgetModel::builder()
                .launch(ThemePickerMenuWidgetInit {})
                .detach(),
        ),
        MenuWidget::Wallpaper => Box::new(
            WallpaperMenuWidgetModel::builder()
                .launch(WallpaperMenuWidgetInit {
                    thumbnail_width: 180,
                    thumbnail_height: 120,
                    row_count: 3,
                })
                .detach(),
        ),
        MenuWidget::Weather => Box::new(
            WeatherModel::builder()
                .launch(WeatherInit {
                    all_in_one: weather_all_in_one,
                })
                .detach(),
        ),
        MenuWidget::SystemStatus => Box::new(
            SystemStatusModel::builder()
                .launch(SystemStatusInit {})
                .detach(),
        ),
        MenuWidget::OverviewIntel => Box::new(
            OverviewIntelModel::builder()
                .launch(OverviewIntelInit {})
                .detach(),
        ),
    }
}
