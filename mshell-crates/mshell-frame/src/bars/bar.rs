use crate::bars::bar_widgets::alarm_clock::{AlarmClockInit, AlarmClockModel};
use crate::bars::bar_widgets::control_center::{ControlCenterInit, ControlCenterModel};
use crate::bars::bar_widgets::bluetooth::{BluetoothInit, BluetoothModel};
use crate::bars::bar_widgets::clipboard::{ClipboardInit, ClipboardModel, ClipboardOutput};
use crate::bars::bar_widgets::weather::{WeatherInit, WeatherModel, WeatherOutput};
use crate::bars::bar_widgets::clock::{ClockInit, ClockModel, ClockOutput};
use crate::bars::bar_widgets::cpu_dashboard::{CpuDashboardInit, CpuDashboardModel};
use crate::bars::bar_widgets::dashboard::{DashboardInit, DashboardModel, DashboardOutput};
use crate::bars::bar_widgets::dark_mode::{DarkModeInit, DarkModeModel};
use crate::bars::bar_widgets::keep_awake::{KeepAwakeInit, KeepAwakeModel};
use crate::bars::bar_widgets::keybinds::{KeybindsInit, KeybindsModel};
use crate::bars::bar_widgets::ssh_sessions::{SshSessionsInit, SshSessionsModel};
use crate::bars::bar_widgets::twilight::{TwilightInit, TwilightModel};
use crate::bars::bar_widgets::lock_keys::{LockKeysInit, LockKeysModel};
use crate::bars::bar_widgets::color_picker::{ColorPickerInit, ColorPickerModel};
use crate::bars::bar_widgets::margo_dock::{
    MargoDockInit, MargoDockModel, MargoDockOutput,
};
use crate::bars::bar_widgets::margo_layout::{MargoLayoutInit, MargoLayoutModel, MargoLayoutOutput};
use crate::bars::bar_widgets::margo_tags::{
    MargoTagsInit, MargoTagsModel,
};
use crate::bars::bar_widgets::lock::{LockInit, LockModel, LockOutput};
use crate::bars::bar_widgets::setup::{SetupInit, SetupModel, SetupOutput};
use crate::bars::bar_widgets::logout::{LogoutInit, LogoutModel};
use crate::bars::bar_widgets::dns::{DnsInit, DnsModel};
use crate::bars::bar_widgets::ip::{IpInit, IpModel};
use crate::bars::bar_widgets::network::{NetworkInit, NetworkModel};
use crate::bars::bar_widgets::notes::{NotesInit, NotesModel};
use crate::bars::bar_widgets::podman::{PodmanInit, PodmanModel};
use crate::bars::bar_widgets::power::{PowerInit, PowerModel};
use crate::bars::bar_widgets::media_player::{
    MediaPlayerInit, MediaPlayerModel, MediaPlayerOutput,
};
use crate::bars::bar_widgets::active_window::{ActiveWindowInit, ActiveWindowModel};
use crate::bars::bar_widgets::ufw::{UfwInit, UfwModel};
use crate::bars::bar_widgets::notifications::{
    NotificationsInit, NotificationsModel, NotificationsOutput,
};
use crate::bars::bar_widgets::privacy::{PrivacyInit, PrivacyModel};
use crate::bars::bar_widgets::reboot::{RebootInit, RebootModel};
use crate::bars::bar_widgets::recording_indicator::{
    RecordingIndicatorInit, RecordingIndicatorModel,
};
use crate::bars::bar_widgets::screenshot::{ScreenshotInit, ScreenshotModel, ScreenshotOutput};
use crate::bars::bar_widgets::shutdown::{ShutdownInit, ShutdownModel};
use crate::bars::bar_widgets::system_tray::{SystemTrayInit, SystemTrayModel};
use crate::bars::bar_widgets::system_update::{SystemUpdateInit, SystemUpdateModel};
use crate::bars::bar_widgets::custom::{CustomWidgetInit, CustomWidgetModel, CustomWidgetOutput};
use mshell_config::schema::config::CustomMenuRow;
use crate::bars::bar_widgets::vpn_indicator::{VpnIndicatorInit, VpnIndicatorModel};
use crate::bars::bar_widgets::wallpaper::{WallpaperInit, WallpaperModel, WallpaperOutput};
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_common::dynamic_box::simple_widget_controller::SimpleWidgetController;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, HorizontalBarStoreFields,
};
use mshell_utils::clear_box::clear_box;
use reactive_graph::traits::*;
use relm4::{
    Component, ComponentParts, ComponentSender,
    gtk::{self, Orientation, prelude::*},
};
use std::fmt::Debug;

/// Bar surface kind. Margo's mshell paints only horizontal bars
/// (Top / Bottom) — the vertical Left / Right variants upstream
/// OkShell shipped have been removed because they conflict with
/// scroller-default column flow.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Clone, Copy)]
pub(crate) enum BarType {
    Top,
    Bottom,
}

pub(crate) struct BarModel {
    h_expand: bool,
    v_expand: bool,
    orientation: Orientation,
    bar_type: BarType,
    start_widgets: Vec<Box<dyn GenericWidgetController>>,
    center_widgets: Vec<Box<dyn GenericWidgetController>>,
    end_widgets: Vec<Box<dyn GenericWidgetController>>,
    // Track the BarWidget kinds backing each container so we can skip
    // the destructive clear+rebuild when the config layer fires a
    // change notification with an identical widget list. Without this
    // guard, reactive-store re-notifications (a single config save in
    // any field reaches every effect bound to the root store) cause
    // the bar to visibly disappear and re-appear as its children are
    // torn down and rebuilt — the user sees this as 2-3 rapid flickers
    // every time a menu opens.
    start_widget_kinds: Vec<BarWidget>,
    center_widget_kinds: Vec<BarWidget>,
    end_widget_kinds: Vec<BarWidget>,
    min_height: i32,
    min_width: i32,
    css_class: String,
    /// "Islands" appearance (`bars.islands`): transparent bar + opaque
    /// floating pills. Read once at build; toggling needs a restart.
    islands: bool,
    revealed: bool,
    hovered: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum BarInput {
    SetStartWidgets(Vec<BarWidget>),
    SetEndWidgets(Vec<BarWidget>),
    SetCenteredWidgets(Vec<BarWidget>),
    SetMinHeight(i32),
    SetRevealed(bool),
    ToggleRevealed,
    SetHovered(bool),
}

#[derive(Debug)]
pub(crate) enum BarOutput {
    ClockClicked,
    DashboardClicked,
    ClipboardClicked,
    NotificationsClicked,
    ScreenshotClicked,
    AppLauncherClicked,
    WallpaperClicked,
    UfwClicked,
    BluetoothClicked,
    CpuDashboardClicked,
    SystemUpdateClicked,
    ValentClicked,
    WeatherClicked,
    KeepAwakeClicked,
    TwilightClicked,
    KeybindsClicked,
    AlarmClockClicked,
    ControlCenterClicked,
    SshSessionsClicked,
    AudioDashboardClicked,
    DnsClicked,
    PodmanClicked,
    NotesClicked,
    IpClicked,
    NetworkClicked,
    PowerClicked,
    MediaPlayerClicked,
    /// Margo layout switcher bar pill clicked. Frame catches and
    /// toggles the in-stack MargoLayout menu (replaces the
    /// legacy in-popover layout chooser).
    MargoLayoutClicked,
    /// A plugin's panel pill was clicked (mplugins WASM tier). Carries the
    /// compiled panel path + resolved settings so the frame can host it in the
    /// first-class plugin-panel menu.
    PluginPanelClicked {
        name: String,
        entry: String,
        settings: String,
        min_width: i32,
        max_height: i32,
    },
    /// A plugin pill with a declarative `[[widget.menu]]` was clicked — the
    /// frame opens its command rows in the first-class plugin menu.
    PluginMenuClicked {
        name: String,
        rows: Vec<CustomMenuRow>,
        min_width: i32,
        max_height: i32,
    },
    CloseMenu,
}

pub(crate) struct BarInit {
    pub(crate) bar_type: BarType,
}

#[relm4::component(pub)]
impl Component for BarModel {
    type CommandOutput = ();
    type Input = BarInput;
    type Output = BarOutput;
    type Init = BarInit;

    view! {
        #[root]
        gtk::Box {
            set_width_request: hover_strip_width,
            set_height_request: hover_strip_height,
            add_controller = gtk::EventControllerMotion {
                connect_enter[sender] => move |_, _, _| {
                    sender.input(BarInput::SetHovered(true));
                },
                connect_leave[sender] => move |_| {
                    sender.input(BarInput::SetHovered(false));
                },
            },

            gtk::Revealer {
                #[watch]
                set_reveal_child: model.revealed || model.hovered,
                set_transition_type: transition_type,

                #[name = "bar_center"]
                gtk::CenterBox {
                    set_css_classes: &["bar", model.css_class.as_str()],
                    #[watch]
                    set_hexpand: model.h_expand,
                    set_vexpand: model.v_expand,
                    set_orientation: model.orientation,
                    #[watch]
                    set_width_request: model.min_width,
                    #[watch]
                    set_height_request: model.min_height,

                    #[wrap(Some)]
                    #[name = "start_container"]
                    set_start_widget = &gtk::Box {
                        set_css_classes: &["bar-widget-container", "start-container"],
                        set_orientation: model.orientation,
                    },

                    #[wrap(Some)]
                    #[name = "center_container"]
                    set_center_widget = &gtk::Box {
                        set_css_classes: &["bar-widget-container", "center-container"],
                        set_orientation: model.orientation,
                    },

                    #[wrap(Some)]
                    #[name = "end_container"]
                    set_end_widget = &gtk::Box {
                        set_css_classes: &["bar-widget-container", "end-container"],
                        set_orientation: model.orientation,
                    },
                },
            },
        },
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = config_manager().config();

        let orientation: Orientation;

        let h_expand: bool;
        let v_expand: bool;
        let css_class: String;
        let transition_type: gtk::RevealerTransitionType;
        let hover_strip_width: i32;
        let hover_strip_height: i32;
        let reveal_by_default: bool;
        let mut effects = EffectScope::new();

        match params.bar_type {
            BarType::Top => {
                orientation = Orientation::Horizontal;
                h_expand = true;
                v_expand = false;
                css_class = "bar-top".to_string();
                transition_type = gtk::RevealerTransitionType::SlideDown;
                hover_strip_width = -1;
                hover_strip_height = 1;
                reveal_by_default = config_manager()
                    .config()
                    .bars()
                    .top_bar()
                    .reveal_by_default()
                    .get_untracked();

                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let min = config_manager()
                        .config()
                        .bars()
                        .top_bar()
                        .minimum_height()
                        .get();
                    sender_clone.input(BarInput::SetMinHeight(min));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().top_bar().left_widgets().get();
                    sender_clone.input(BarInput::SetStartWidgets(widgets));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().top_bar().right_widgets().get();
                    sender_clone.input(BarInput::SetEndWidgets(widgets));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().top_bar().center_widgets().get();
                    sender_clone.input(BarInput::SetCenteredWidgets(widgets));
                });
            }
            BarType::Bottom => {
                orientation = Orientation::Horizontal;
                h_expand = true;
                v_expand = false;
                css_class = "bar-bottom".to_string();
                transition_type = gtk::RevealerTransitionType::SlideUp;
                hover_strip_width = -1;
                hover_strip_height = 1;
                reveal_by_default = config_manager()
                    .config()
                    .bars()
                    .bottom_bar()
                    .reveal_by_default()
                    .get_untracked();

                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let min = config_manager()
                        .config()
                        .bars()
                        .bottom_bar()
                        .minimum_height()
                        .get();
                    sender_clone.input(BarInput::SetMinHeight(min));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().bottom_bar().left_widgets().get();
                    sender_clone.input(BarInput::SetStartWidgets(widgets));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().bottom_bar().right_widgets().get();
                    sender_clone.input(BarInput::SetEndWidgets(widgets));
                });

                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.bars().bottom_bar().center_widgets().get();
                    sender_clone.input(BarInput::SetCenteredWidgets(widgets));
                });
            }
        }

        let islands = config_manager().config().bars().islands().get_untracked();

        let model = BarModel {
            h_expand,
            v_expand,
            orientation,
            bar_type: params.bar_type,
            start_widgets: Vec::new(),
            center_widgets: Vec::new(),
            end_widgets: Vec::new(),
            start_widget_kinds: Vec::new(),
            center_widget_kinds: Vec::new(),
            end_widget_kinds: Vec::new(),
            min_width: 0,
            min_height: 0,
            css_class,
            islands,
            revealed: reveal_by_default,
            hovered: false,
            _effects: effects,
        };

        let widgets = view_output!();

        // Opt-in "islands" look: a marker class the SCSS keys off to make
        // the bar transparent and the pills float as opaque surfaces.
        if model.islands {
            widgets.bar_center.add_css_class("islands");
        }

        let _ = sender;

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
            BarInput::SetStartWidgets(bar_widgets) => {
                if self.start_widget_kinds == bar_widgets {
                    return;
                }
                clear_box(&widgets.start_container);
                self.start_widgets.clear();
                for item in &bar_widgets {
                    let controller =
                        BarModel::build_widget(self.orientation, self.bar_type, item, &sender);
                    widgets.start_container.append(&controller.root_widget());
                    self.start_widgets.push(controller);
                }
                self.start_widget_kinds = bar_widgets;
            }
            BarInput::SetEndWidgets(bar_widgets) => {
                if self.end_widget_kinds == bar_widgets {
                    return;
                }
                clear_box(&widgets.end_container);
                self.end_widgets.clear();
                for item in &bar_widgets {
                    let controller =
                        BarModel::build_widget(self.orientation, self.bar_type, item, &sender);
                    widgets.end_container.append(&controller.root_widget());
                    self.end_widgets.push(controller);
                }
                self.end_widget_kinds = bar_widgets;
            }
            BarInput::SetCenteredWidgets(bar_widgets) => {
                if self.center_widget_kinds == bar_widgets {
                    return;
                }
                clear_box(&widgets.center_container);
                self.center_widgets.clear();
                for item in &bar_widgets {
                    let controller =
                        BarModel::build_widget(self.orientation, self.bar_type, item, &sender);
                    widgets.center_container.append(&controller.root_widget());
                    self.center_widgets.push(controller);
                }
                self.center_widget_kinds = bar_widgets;
            }
            BarInput::SetMinHeight(min) => {
                self.min_height = min;
            }
            BarInput::SetRevealed(revealed) => {
                self.revealed = revealed;
            }
            BarInput::ToggleRevealed => {
                self.revealed = !self.revealed;
            }
            BarInput::SetHovered(hovered) => {
                self.hovered = hovered;
            }
        }
        self.update_view(widgets, sender);
    }
}

impl BarModel {
    fn build_widget(
        orientation: Orientation,
        bar_type: BarType,
        widget: &BarWidget,
        sender: &ComponentSender<Self>,
    ) -> Box<dyn GenericWidgetController> {
        match widget {
            BarWidget::AudioDashboard => Box::new(
                crate::bars::bar_widgets::audio_dashboard::AudioDashboardModel::builder()
                    .launch(crate::bars::bar_widgets::audio_dashboard::AudioDashboardInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::audio_dashboard::AudioDashboardOutput::Clicked
                            => BarOutput::AudioDashboardClicked,
                    }),
            ),
            BarWidget::ActiveWindow => Box::new(
                ActiveWindowModel::builder()
                    .launch(ActiveWindowInit {})
                    .detach(),
            ),
            BarWidget::Bluetooth => Box::new(
                BluetoothModel::builder()
                    .launch(BluetoothInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::bluetooth::BluetoothOutput::Clicked
                            => BarOutput::BluetoothClicked,
                    }),
            ),
            BarWidget::Weather => Box::new(
                WeatherModel::builder()
                    .launch(WeatherInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        WeatherOutput::Clicked => BarOutput::WeatherClicked,
                    }),
            ),
            BarWidget::Clipboard => Box::new(
                ClipboardModel::builder()
                    .launch(ClipboardInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        ClipboardOutput::Clicked => BarOutput::ClipboardClicked,
                    }),
            ),
            BarWidget::Clock => Box::new(
                ClockModel::builder()
                    .launch(ClockInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        ClockOutput::Clicked => BarOutput::ClockClicked,
                    }),
            ),
            BarWidget::CpuDashboard => Box::new(
                CpuDashboardModel::builder()
                    .launch(CpuDashboardInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::cpu_dashboard::CpuDashboardOutput::Clicked
                            => BarOutput::CpuDashboardClicked,
                    }),
            ),
            BarWidget::Dashboard => Box::new(
                DashboardModel::builder()
                    .launch(DashboardInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        DashboardOutput::Clicked => BarOutput::DashboardClicked,
                    }),
            ),
            BarWidget::DarkMode => Box::new(
                DarkModeModel::builder()
                    .launch(DarkModeInit { orientation })
                    .detach(),
            ),
            BarWidget::KeepAwake => Box::new(
                KeepAwakeModel::builder()
                    .launch(KeepAwakeInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::keep_awake::KeepAwakeOutput::Clicked
                            => BarOutput::KeepAwakeClicked,
                    }),
            ),
            BarWidget::Twilight => Box::new(
                TwilightModel::builder()
                    .launch(TwilightInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::twilight::TwilightOutput::Clicked
                            => BarOutput::TwilightClicked,
                    }),
            ),
            BarWidget::Keybinds => Box::new(
                KeybindsModel::builder()
                    .launch(KeybindsInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::keybinds::KeybindsOutput::Clicked
                            => BarOutput::KeybindsClicked,
                    }),
            ),
            BarWidget::AlarmClock => Box::new(
                AlarmClockModel::builder()
                    .launch(AlarmClockInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::alarm_clock::AlarmClockOutput::Clicked
                            => BarOutput::AlarmClockClicked,
                    }),
            ),
            BarWidget::ControlCenter => Box::new(
                ControlCenterModel::builder()
                    .launch(ControlCenterInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::control_center::ControlCenterOutput::Clicked
                            => BarOutput::ControlCenterClicked,
                    }),
            ),
            BarWidget::SshSessions => Box::new(
                SshSessionsModel::builder()
                    .launch(SshSessionsInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::ssh_sessions::SshSessionsOutput::Clicked
                            => BarOutput::SshSessionsClicked,
                    }),
            ),
            BarWidget::LockKeys => Box::new(
                LockKeysModel::builder()
                    .launch(LockKeysInit { orientation })
                    .detach(),
            ),
            BarWidget::MargoDock => Box::new(
                MargoDockModel::builder()
                    .launch(MargoDockInit {
                        orientation,
                        bar_type,
                    })
                    .forward(sender.output_sender(), |msg| match msg {
                        MargoDockOutput::AppLauncherClicked => BarOutput::AppLauncherClicked,
                    }),
            ),
            BarWidget::MargoLayoutSwitcher => Box::new(
                MargoLayoutModel::builder()
                    .launch(MargoLayoutInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        MargoLayoutOutput::Clicked => BarOutput::MargoLayoutClicked,
                    }),
            ),
            BarWidget::MargoTags => Box::new(
                MargoTagsModel::builder()
                    .launch(MargoTagsInit { orientation })
                    .detach(),
            ),
            BarWidget::ColorPicker => Box::new(
                ColorPickerModel::builder()
                    .launch(ColorPickerInit { orientation })
                    .detach(),
            ),
            BarWidget::Lock => Box::new(
                LockModel::builder()
                    .launch(LockInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        LockOutput::CloseMenu => BarOutput::CloseMenu,
                    }),
            ),
            BarWidget::Setup => Box::new(
                SetupModel::builder()
                    .launch(SetupInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        SetupOutput::CloseMenu => BarOutput::CloseMenu,
                    }),
            ),
            BarWidget::Logout => Box::new(
                LogoutModel::builder()
                    .launch(LogoutInit { orientation })
                    .detach(),
            ),
            BarWidget::Dns => Box::new(
                DnsModel::builder()
                    .launch(DnsInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::dns::DnsOutput::Clicked => {
                            BarOutput::DnsClicked
                        }
                    }),
            ),
            BarWidget::Ip => Box::new(
                IpModel::builder()
                    .launch(IpInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::ip::IpOutput::Clicked => {
                            BarOutput::IpClicked
                        }
                    }),
            ),
            BarWidget::Network => Box::new(
                NetworkModel::builder()
                    .launch(NetworkInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::network::NetworkOutput::Clicked => {
                            BarOutput::NetworkClicked
                        }
                    }),
            ),
            BarWidget::Notes => Box::new(
                NotesModel::builder()
                    .launch(NotesInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::notes::NotesOutput::Clicked => {
                            BarOutput::NotesClicked
                        }
                    }),
            ),
            BarWidget::Podman => Box::new(
                PodmanModel::builder()
                    .launch(PodmanInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::podman::PodmanOutput::Clicked => {
                            BarOutput::PodmanClicked
                        }
                    }),
            ),
            BarWidget::Power => Box::new(
                PowerModel::builder()
                    .launch(PowerInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::power::PowerOutput::Clicked => {
                            BarOutput::PowerClicked
                        }
                    }),
            ),
            BarWidget::MediaPlayer => Box::new(
                MediaPlayerModel::builder()
                    .launch(MediaPlayerInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        MediaPlayerOutput::Clicked => BarOutput::MediaPlayerClicked,
                    }),
            ),
            BarWidget::Ufw => Box::new(
                UfwModel::builder()
                    .launch(UfwInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::ufw::UfwOutput::Clicked => {
                            BarOutput::UfwClicked
                        }
                    }),
            ),
            BarWidget::Notifications => Box::new(
                NotificationsModel::builder()
                    .launch(NotificationsInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        NotificationsOutput::Clicked => BarOutput::NotificationsClicked,
                    }),
            ),
            BarWidget::Privacy => Box::new(
                PrivacyModel::builder()
                    .launch(PrivacyInit { orientation })
                    .detach(),
            ),
            BarWidget::Reboot => Box::new(
                RebootModel::builder()
                    .launch(RebootInit { orientation })
                    .detach(),
            ),
            BarWidget::RecordingIndicator => Box::new(
                RecordingIndicatorModel::builder()
                    .launch(RecordingIndicatorInit { orientation })
                    .detach(),
            ),
            BarWidget::Screenshot => Box::new(
                ScreenshotModel::builder()
                    .launch(ScreenshotInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        ScreenshotOutput::Clicked => BarOutput::ScreenshotClicked,
                    }),
            ),
            BarWidget::Shutdown => Box::new(
                ShutdownModel::builder()
                    .launch(ShutdownInit { orientation })
                    .detach(),
            ),
            BarWidget::Tray => Box::new(
                SystemTrayModel::builder()
                    .launch(SystemTrayInit { orientation })
                    .detach(),
            ),
            BarWidget::SystemUpdate => Box::new(
                SystemUpdateModel::builder()
                    .launch(SystemUpdateInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::system_update::SystemUpdateOutput::Clicked
                            => BarOutput::SystemUpdateClicked,
                    }),
            ),
            BarWidget::Valent => Box::new(
                crate::bars::bar_widgets::valent::ValentModel::builder()
                    .launch(crate::bars::bar_widgets::valent::ValentInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::valent::ValentOutput::Clicked
                            => BarOutput::ValentClicked,
                    }),
            ),
            BarWidget::VpnIndicator => Box::new(
                VpnIndicatorModel::builder()
                    .launch(VpnIndicatorInit {})
                    .detach(),
            ),
            BarWidget::Spacer(width) => {
                // Render-only blank gap of the configured pixel width.
                let b = gtk::Box::new(Orientation::Horizontal, 0);
                b.add_css_class("bar-spacer");
                b.set_size_request(*width as i32, -1);
                Box::new(SimpleWidgetController::new(b.upcast()))
            }
            BarWidget::Separator => {
                // Render-only thin vertical divider line.
                let b = gtk::Box::new(Orientation::Horizontal, 0);
                b.add_css_class("bar-separator");
                Box::new(SimpleWidgetController::new(b.upcast()))
            }
            BarWidget::Custom(name) => {
                // Resolve the named definition from bars.widgets.custom_widgets;
                // an unknown name falls back to an empty (inert) pill.
                let config = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .custom_widgets()
                    .get_untracked()
                    .into_iter()
                    .find(|c| &c.name == name)
                    .unwrap_or_default();
                Box::new(
                    CustomWidgetModel::builder()
                        .launch(CustomWidgetInit { config })
                        .forward(sender.output_sender(), |msg| match msg {
                            CustomWidgetOutput::OpenPanel {
                                name,
                                entry,
                                settings,
                                min_width,
                                max_height,
                            } => BarOutput::PluginPanelClicked {
                                name,
                                entry,
                                settings,
                                min_width,
                                max_height,
                            },
                            CustomWidgetOutput::OpenMenu {
                                name,
                                rows,
                                min_width,
                                max_height,
                            } => BarOutput::PluginMenuClicked {
                                name,
                                rows,
                                min_width,
                                max_height,
                            },
                        }),
                )
            }
            BarWidget::Wallpaper => Box::new(
                WallpaperModel::builder()
                    .launch(WallpaperInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        WallpaperOutput::Clicked => BarOutput::WallpaperClicked,
                    }),
            ),
        }
    }
}

impl Debug for BarModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BarModel")
            .field("expanded", &self.h_expand)
            .field("orientation", &self.orientation)
            .finish()
    }
}
