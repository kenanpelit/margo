//! Per-widget menu settings — one component, parameterised by
//! `MenuKind`, that surfaces a given menu's `position` and
//! `minimum_width`. Used inside the `Widgets` sub-sidebar so each
//! menu gets its own focused settings page.
//!
//! The widgets-list editor (which BarWidget pills live inside a
//! menu) stays in the existing `menu_settings::Layout` page —
//! that's a cross-cutting view of every menu at once. These
//! per-menu pages are the "I just want to tweak THIS one"
//! shortcut.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Which menu this settings page targets. The enum carries
/// everything we need to read / write through `config_manager`
/// (descriptive label + reactive-field accessor dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    AppLauncher,
    AudioDashboard,
    Bluetooth,
    Clipboard,
    Clock,
    CpuDashboard,
    Dashboard,
    MargoLayout,
    MediaPlayer,
    Dns,
    Ip,
    Network,
    Notes,
    Notifications,
    Podman,
    Power,
    Screenshot,
    Ufw,
    Wallpaper,
}

impl MenuKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::AppLauncher => "App Launcher",
            Self::AudioDashboard => "Audio Dashboard",
            Self::Bluetooth => "Bluetooth",
            Self::Clipboard => "Clipboard",
            Self::Clock => "Clock",
            Self::CpuDashboard => "CPU Dashboard",
            Self::Dashboard => "Dashboard",
            Self::MargoLayout => "Margo Layout",
            Self::MediaPlayer => "Media Player",
            Self::Dns => "DNS / VPN",
            Self::Ip => "Public IP",
            Self::Network => "Network Console",
            Self::Notes => "Notes Hub",
            Self::Notifications => "Notifications",
            Self::Podman => "Podman",
            Self::Power => "Power Profile",
            Self::Screenshot => "Screenshot",
            Self::Ufw => "UFW Firewall",
            Self::Wallpaper => "Wallpaper",
        }
    }

    /// All known menu kinds, in the order they should appear in
    /// the cross-cutting Menus settings page. Kept stable so the
    /// scroll position survives a config reload.
    pub(crate) fn all() -> &'static [MenuKind] {
        &[
            MenuKind::Clock,
            MenuKind::Dashboard,
            MenuKind::Clipboard,
            MenuKind::Screenshot,
            MenuKind::Notifications,
            MenuKind::AppLauncher,
            MenuKind::Wallpaper,
            MenuKind::MediaPlayer,
            MenuKind::Power,
            MenuKind::Bluetooth,
            MenuKind::CpuDashboard,
            MenuKind::AudioDashboard,
            MenuKind::Ufw,
            MenuKind::Dns,
            MenuKind::Podman,
            MenuKind::Notes,
            MenuKind::Ip,
            MenuKind::Network,
            MenuKind::MargoLayout,
        ]
    }

    /// Snapshot the menu's current position. `_untracked` so the
    /// initial model load doesn't subscribe; the `EffectScope`
    /// below subscribes explicitly.
    fn read_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get_untracked(),
            Self::Clipboard => m.clipboard_menu().position().get_untracked(),
            Self::Clock => m.clock_menu().position().get_untracked(),
            Self::Dashboard => m.dashboard_menu().position().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().position().get_untracked(),
            Self::Dns => m.dns_menu().position().get_untracked(),
            Self::Ip => m.ip_menu().position().get_untracked(),
            Self::Network => m.network_menu().position().get_untracked(),
            Self::Notes => m.notes_menu().position().get_untracked(),
            Self::Notifications => m.notification_menu().position().get_untracked(),
            Self::Podman => m.podman_menu().position().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().position().get_untracked(),
            Self::Power => m.power_menu().position().get_untracked(),
            Self::Screenshot => m.screenshot_menu().position().get_untracked(),
            Self::Ufw => m.ufw_menu().position().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().position().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().position().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().position().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().position().get_untracked(),
        }
    }

    fn read_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get_untracked(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get_untracked(),
            Self::Clock => m.clock_menu().minimum_width().get_untracked(),
            Self::Dashboard => m.dashboard_menu().minimum_width().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().minimum_width().get_untracked(),
            Self::Dns => m.dns_menu().minimum_width().get_untracked(),
            Self::Ip => m.ip_menu().minimum_width().get_untracked(),
            Self::Network => m.network_menu().minimum_width().get_untracked(),
            Self::Notes => m.notes_menu().minimum_width().get_untracked(),
            Self::Notifications => m.notification_menu().minimum_width().get_untracked(),
            Self::Podman => m.podman_menu().minimum_width().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().minimum_width().get_untracked(),
            Self::Power => m.power_menu().minimum_width().get_untracked(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get_untracked(),
            Self::Ufw => m.ufw_menu().minimum_width().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().minimum_width().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().minimum_width().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().minimum_width().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().minimum_width().get_untracked(),
        }
    }

    fn tracked_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get(),
            Self::Clipboard => m.clipboard_menu().position().get(),
            Self::Clock => m.clock_menu().position().get(),
            Self::Dashboard => m.dashboard_menu().position().get(),
            Self::MediaPlayer => m.media_player_menu().position().get(),
            Self::Dns => m.dns_menu().position().get(),
            Self::Ip => m.ip_menu().position().get(),
            Self::Network => m.network_menu().position().get(),
            Self::Notes => m.notes_menu().position().get(),
            Self::Notifications => m.notification_menu().position().get(),
            Self::Podman => m.podman_menu().position().get(),
            Self::Wallpaper => m.wallpaper_menu().position().get(),
            Self::Power => m.power_menu().position().get(),
            Self::Screenshot => m.screenshot_menu().position().get(),
            Self::Ufw => m.ufw_menu().position().get(),
            Self::Bluetooth => m.bluetooth_menu().position().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().position().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().position().get(),
            Self::MargoLayout => m.margo_layout_menu().position().get(),
        }
    }

    fn tracked_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get(),
            Self::Clock => m.clock_menu().minimum_width().get(),
            Self::Dashboard => m.dashboard_menu().minimum_width().get(),
            Self::MediaPlayer => m.media_player_menu().minimum_width().get(),
            Self::Dns => m.dns_menu().minimum_width().get(),
            Self::Ip => m.ip_menu().minimum_width().get(),
            Self::Network => m.network_menu().minimum_width().get(),
            Self::Notes => m.notes_menu().minimum_width().get(),
            Self::Notifications => m.notification_menu().minimum_width().get(),
            Self::Podman => m.podman_menu().minimum_width().get(),
            Self::Wallpaper => m.wallpaper_menu().minimum_width().get(),
            Self::Power => m.power_menu().minimum_width().get(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get(),
            Self::Ufw => m.ufw_menu().minimum_width().get(),
            Self::Bluetooth => m.bluetooth_menu().minimum_width().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().minimum_width().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().minimum_width().get(),
            Self::MargoLayout => m.margo_layout_menu().minimum_width().get(),
        }
    }

    fn write_position(self, p: Position) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.position = p,
            Self::Clipboard => c.menus.clipboard_menu.position = p,
            Self::Clock => c.menus.clock_menu.position = p,
            Self::Dashboard => c.menus.dashboard_menu.position = p,
            Self::MediaPlayer => c.menus.media_player_menu.position = p,
            Self::Dns => c.menus.dns_menu.position = p,
            Self::Ip => c.menus.ip_menu.position = p,
            Self::Network => c.menus.network_menu.position = p,
            Self::Notes => c.menus.notes_menu.position = p,
            Self::Notifications => c.menus.notification_menu.position = p,
            Self::Podman => c.menus.podman_menu.position = p,
            Self::Wallpaper => c.menus.wallpaper_menu.position = p,
            Self::Power => c.menus.power_menu.position = p,
            Self::Screenshot => c.menus.screenshot_menu.position = p,
            Self::Ufw => c.menus.ufw_menu.position = p,
            Self::Bluetooth => c.menus.bluetooth_menu.position = p,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.position = p,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.position = p,
            Self::MargoLayout => c.menus.margo_layout_menu.position = p,
        });
    }

    fn write_min_width(self, w: i32) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.minimum_width = w,
            Self::Clipboard => c.menus.clipboard_menu.minimum_width = w,
            Self::Clock => c.menus.clock_menu.minimum_width = w,
            Self::Dashboard => c.menus.dashboard_menu.minimum_width = w,
            Self::MediaPlayer => c.menus.media_player_menu.minimum_width = w,
            Self::Dns => c.menus.dns_menu.minimum_width = w,
            Self::Ip => c.menus.ip_menu.minimum_width = w,
            Self::Network => c.menus.network_menu.minimum_width = w,
            Self::Notes => c.menus.notes_menu.minimum_width = w,
            Self::Notifications => c.menus.notification_menu.minimum_width = w,
            Self::Podman => c.menus.podman_menu.minimum_width = w,
            Self::Wallpaper => c.menus.wallpaper_menu.minimum_width = w,
            Self::Power => c.menus.power_menu.minimum_width = w,
            Self::Screenshot => c.menus.screenshot_menu.minimum_width = w,
            Self::Ufw => c.menus.ufw_menu.minimum_width = w,
            Self::Bluetooth => c.menus.bluetooth_menu.minimum_width = w,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.minimum_width = w,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.minimum_width = w,
            Self::MargoLayout => c.menus.margo_layout_menu.minimum_width = w,
        });
    }

    fn read_max_height(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().maximum_height().get_untracked(),
            Self::Clipboard => m.clipboard_menu().maximum_height().get_untracked(),
            Self::Clock => m.clock_menu().maximum_height().get_untracked(),
            Self::Dashboard => m.dashboard_menu().maximum_height().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().maximum_height().get_untracked(),
            Self::Dns => m.dns_menu().maximum_height().get_untracked(),
            Self::Ip => m.ip_menu().maximum_height().get_untracked(),
            Self::Network => m.network_menu().maximum_height().get_untracked(),
            Self::Notes => m.notes_menu().maximum_height().get_untracked(),
            Self::Notifications => m.notification_menu().maximum_height().get_untracked(),
            Self::Podman => m.podman_menu().maximum_height().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().maximum_height().get_untracked(),
            Self::Power => m.power_menu().maximum_height().get_untracked(),
            Self::Screenshot => m.screenshot_menu().maximum_height().get_untracked(),
            Self::Ufw => m.ufw_menu().maximum_height().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().maximum_height().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().maximum_height().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().maximum_height().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().maximum_height().get_untracked(),
        }
    }

    fn tracked_max_height(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().maximum_height().get(),
            Self::Clipboard => m.clipboard_menu().maximum_height().get(),
            Self::Clock => m.clock_menu().maximum_height().get(),
            Self::Dashboard => m.dashboard_menu().maximum_height().get(),
            Self::MediaPlayer => m.media_player_menu().maximum_height().get(),
            Self::Dns => m.dns_menu().maximum_height().get(),
            Self::Ip => m.ip_menu().maximum_height().get(),
            Self::Network => m.network_menu().maximum_height().get(),
            Self::Notes => m.notes_menu().maximum_height().get(),
            Self::Notifications => m.notification_menu().maximum_height().get(),
            Self::Podman => m.podman_menu().maximum_height().get(),
            Self::Wallpaper => m.wallpaper_menu().maximum_height().get(),
            Self::Power => m.power_menu().maximum_height().get(),
            Self::Screenshot => m.screenshot_menu().maximum_height().get(),
            Self::Ufw => m.ufw_menu().maximum_height().get(),
            Self::Bluetooth => m.bluetooth_menu().maximum_height().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().maximum_height().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().maximum_height().get(),
            Self::MargoLayout => m.margo_layout_menu().maximum_height().get(),
        }
    }

    fn write_max_height(self, h: i32) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.maximum_height = h,
            Self::Clipboard => c.menus.clipboard_menu.maximum_height = h,
            Self::Clock => c.menus.clock_menu.maximum_height = h,
            Self::Dashboard => c.menus.dashboard_menu.maximum_height = h,
            Self::MediaPlayer => c.menus.media_player_menu.maximum_height = h,
            Self::Dns => c.menus.dns_menu.maximum_height = h,
            Self::Ip => c.menus.ip_menu.maximum_height = h,
            Self::Network => c.menus.network_menu.maximum_height = h,
            Self::Notes => c.menus.notes_menu.maximum_height = h,
            Self::Notifications => c.menus.notification_menu.maximum_height = h,
            Self::Podman => c.menus.podman_menu.maximum_height = h,
            Self::Wallpaper => c.menus.wallpaper_menu.maximum_height = h,
            Self::Power => c.menus.power_menu.maximum_height = h,
            Self::Screenshot => c.menus.screenshot_menu.maximum_height = h,
            Self::Ufw => c.menus.ufw_menu.maximum_height = h,
            Self::Bluetooth => c.menus.bluetooth_menu.maximum_height = h,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.maximum_height = h,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.maximum_height = h,
            Self::MargoLayout => c.menus.margo_layout_menu.maximum_height = h,
        });
    }

    /// Snapshot the menu's current widget list. Used to seed the
    /// `MenuWidgetListModel` factory at panel-creation time.
    pub(crate) fn read_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().widgets().get_untracked(),
            Self::Clipboard => m.clipboard_menu().widgets().get_untracked(),
            Self::Clock => m.clock_menu().widgets().get_untracked(),
            Self::Dashboard => m.dashboard_menu().widgets().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().widgets().get_untracked(),
            Self::Dns => m.dns_menu().widgets().get_untracked(),
            Self::Ip => m.ip_menu().widgets().get_untracked(),
            Self::Network => m.network_menu().widgets().get_untracked(),
            Self::Notes => m.notes_menu().widgets().get_untracked(),
            Self::Notifications => m.notification_menu().widgets().get_untracked(),
            Self::Podman => m.podman_menu().widgets().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().widgets().get_untracked(),
            Self::Power => m.power_menu().widgets().get_untracked(),
            Self::Screenshot => m.screenshot_menu().widgets().get_untracked(),
            Self::Ufw => m.ufw_menu().widgets().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().widgets().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().widgets().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().widgets().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().widgets().get_untracked(),
        }
    }

    /// Tracked read — subscribes the calling effect to widget-list
    /// changes so an external `mshellctl config reload` repaints
    /// the panel without a UI restart.
    pub(crate) fn tracked_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().widgets().get(),
            Self::Clipboard => m.clipboard_menu().widgets().get(),
            Self::Clock => m.clock_menu().widgets().get(),
            Self::Dashboard => m.dashboard_menu().widgets().get(),
            Self::MediaPlayer => m.media_player_menu().widgets().get(),
            Self::Dns => m.dns_menu().widgets().get(),
            Self::Ip => m.ip_menu().widgets().get(),
            Self::Network => m.network_menu().widgets().get(),
            Self::Notes => m.notes_menu().widgets().get(),
            Self::Notifications => m.notification_menu().widgets().get(),
            Self::Podman => m.podman_menu().widgets().get(),
            Self::Wallpaper => m.wallpaper_menu().widgets().get(),
            Self::Power => m.power_menu().widgets().get(),
            Self::Screenshot => m.screenshot_menu().widgets().get(),
            Self::Ufw => m.ufw_menu().widgets().get(),
            Self::Bluetooth => m.bluetooth_menu().widgets().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().widgets().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().widgets().get(),
            Self::MargoLayout => m.margo_layout_menu().widgets().get(),
        }
    }

    /// Persist a new widget list to disk. Called from the panel
    /// when the in-UI reorder/add/remove fires.
    pub(crate) fn write_widgets(
        self,
        widgets: Vec<mshell_config::schema::menu_widgets::MenuWidget>,
    ) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.widgets = widgets,
            Self::Clipboard => c.menus.clipboard_menu.widgets = widgets,
            Self::Clock => c.menus.clock_menu.widgets = widgets,
            Self::Dashboard => c.menus.dashboard_menu.widgets = widgets,
            Self::MediaPlayer => c.menus.media_player_menu.widgets = widgets,
            Self::Dns => c.menus.dns_menu.widgets = widgets,
            Self::Ip => c.menus.ip_menu.widgets = widgets,
            Self::Network => c.menus.network_menu.widgets = widgets,
            Self::Notes => c.menus.notes_menu.widgets = widgets,
            Self::Notifications => c.menus.notification_menu.widgets = widgets,
            Self::Podman => c.menus.podman_menu.widgets = widgets,
            Self::Wallpaper => c.menus.wallpaper_menu.widgets = widgets,
            Self::Power => c.menus.power_menu.widgets = widgets,
            Self::Screenshot => c.menus.screenshot_menu.widgets = widgets,
            Self::Ufw => c.menus.ufw_menu.widgets = widgets,
            Self::Bluetooth => c.menus.bluetooth_menu.widgets = widgets,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.widgets = widgets,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.widgets = widgets,
            Self::MargoLayout => c.menus.margo_layout_menu.widgets = widgets,
        });
    }
}

#[derive(Debug)]
pub(crate) struct WidgetMenuSettingsModel {
    kind: MenuKind,
    position: Position,
    minimum_width: i32,
    /// Maximum visible content height in pixels. 0 = no cap.
    maximum_height: i32,
    position_model: gtk::StringList,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsInput {
    PositionPicked(u32),
    MinWidthChanged(i32),
    MaxHeightChanged(i32),
    PositionEffect(Position),
    MinWidthEffect(i32),
    MaxHeightEffect(i32),
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsOutput {}

pub(crate) struct WidgetMenuSettingsInit {
    pub(crate) kind: MenuKind,
}

#[relm4::component(pub(crate))]
impl Component for WidgetMenuSettingsModel {
    type CommandOutput = ();
    type Input = WidgetMenuSettingsInput;
    type Output = WidgetMenuSettingsOutput;
    type Init = WidgetMenuSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-list-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Menu widget",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-menu widget configuration — which sub-widgets show up inside a given menu and in what order.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Per-menu layout. The widgets that appear inside this menu are configured under Widgets → Layout.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Position",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Which screen edge this menu anchors to.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 180,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.position_model),
                        #[watch]
                        #[block_signal(position_handler)]
                        set_selected: model.position.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WidgetMenuSettingsInput::PositionPicked(dd.selected()));
                        } @position_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Minimum Width",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Width floor in pixels. The menu may grow past this for long content.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (200.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(min_width_handler)]
                        set_value: model.minimum_width as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MinWidthChanged(s.value() as i32));
                        } @min_width_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Maximum Height",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Viewport cap in pixels. The menu scrolls vertically past this height. Set to 0 to disable the cap and let the menu grow to fit its contents.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        // 0 = uncapped; otherwise reasonable monitor-sized range.
                        set_range: (0.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(max_height_handler)]
                        set_value: model.maximum_height as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MaxHeightChanged(s.value() as i32));
                        } @max_height_handler,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let position_refs: Vec<&str> =
            Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let mut effects = EffectScope::new();

        let kind = params.kind;
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let p = kind.tracked_position();
            sender_clone.input(WidgetMenuSettingsInput::PositionEffect(p));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let w = kind.tracked_min_width();
            sender_clone.input(WidgetMenuSettingsInput::MinWidthEffect(w));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let h = kind.tracked_max_height();
            sender_clone.input(WidgetMenuSettingsInput::MaxHeightEffect(h));
        });

        let model = WidgetMenuSettingsModel {
            kind,
            position: kind.read_position(),
            minimum_width: kind.read_min_width(),
            maximum_height: kind.read_max_height(),
            position_model,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WidgetMenuSettingsInput::PositionPicked(idx) => {
                let p = Position::from_index(idx);
                if self.position != p {
                    self.position = p.clone();
                    self.kind.write_position(p);
                }
            }
            WidgetMenuSettingsInput::MinWidthChanged(w) => {
                if self.minimum_width != w {
                    self.minimum_width = w;
                    self.kind.write_min_width(w);
                }
            }
            WidgetMenuSettingsInput::MaxHeightChanged(h) => {
                if self.maximum_height != h {
                    self.maximum_height = h;
                    self.kind.write_max_height(h);
                }
            }
            WidgetMenuSettingsInput::PositionEffect(p) => self.position = p,
            WidgetMenuSettingsInput::MinWidthEffect(w) => self.minimum_width = w,
            WidgetMenuSettingsInput::MaxHeightEffect(h) => self.maximum_height = h,
        }
    }
}
