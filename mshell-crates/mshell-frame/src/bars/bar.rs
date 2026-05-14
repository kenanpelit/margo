use crate::bars::bar_widgets::audio_input::{AudioInputInit, AudioInputModel};
use crate::bars::bar_widgets::audio_output::{AudioOutputInit, AudioOutputModel};
use crate::bars::bar_widgets::battery::{BatteryInit, BatteryModel};
use crate::bars::bar_widgets::bluetooth::{BluetoothInit, BluetoothModel};
use crate::bars::bar_widgets::clipboard::{ClipboardInit, ClipboardModel, ClipboardOutput};
use crate::bars::bar_widgets::clock::{ClockInit, ClockModel, ClockOutput};
use crate::bars::bar_widgets::hypr_picker::{HyprPickerInit, HyprPickerModel};
use crate::bars::bar_widgets::margo_dock::{
    MargoDockInit, MargoDockModel, MargoDockOutput,
};
use crate::bars::bar_widgets::margo_layout::{MargoLayoutInit, MargoLayoutModel};
use crate::bars::bar_widgets::margo_tags::{
    MargoTagsInit, MargoTagsModel,
};
use crate::bars::bar_widgets::lock::{LockInit, LockModel, LockOutput};
use crate::bars::bar_widgets::logout::{LogoutInit, LogoutModel};
use crate::bars::bar_widgets::network::{NetworkInit, NetworkModel};
use crate::bars::bar_widgets::ndns::{NdnsInit, NdnsModel};
use crate::bars::bar_widgets::nip::{NipInit, NipModel};
use crate::bars::bar_widgets::nnotes::{NnotesInit, NnotesModel};
use crate::bars::bar_widgets::npodman::{NpodmanInit, NpodmanModel};
use crate::bars::bar_widgets::nufw::{NufwInit, NufwModel};
use crate::bars::bar_widgets::notifications::{
    NotificationsInit, NotificationsModel, NotificationsOutput,
};
use crate::bars::bar_widgets::power_profile::{PowerProfileInit, PowerProfileModel};
use crate::bars::bar_widgets::quick_settings::{
    QuickSettingOutput, QuickSettingsInit, QuickSettingsModel,
};
use crate::bars::bar_widgets::reboot::{RebootInit, RebootModel};
use crate::bars::bar_widgets::recording_indicator::{
    RecordingIndicatorInit, RecordingIndicatorModel,
};
use crate::bars::bar_widgets::screenshot::{ScreenshotInit, ScreenshotModel, ScreenshotOutput};
use crate::bars::bar_widgets::shutdown::{ShutdownInit, ShutdownModel};
use crate::bars::bar_widgets::system_tray::{SystemTrayInit, SystemTrayModel};
use crate::bars::bar_widgets::vpn_indicator::{VpnIndicatorInit, VpnIndicatorModel};
use crate::bars::bar_widgets::wallpaper::{WallpaperInit, WallpaperModel, WallpaperOutput};
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, HorizontalBarStoreFields,
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
    revealed: bool,
    hovered: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum BarInput {
    SetStartWidgets(Vec<BarWidget>),
    SetEndWidgets(Vec<BarWidget>),
    SetCenteredWidgets(Vec<BarWidget>),
    SetMinWidth(i32),
    SetMinHeight(i32),
    SetRevealed(bool),
    ToggleRevealed,
    SetHovered(bool),
}

#[derive(Debug)]
pub(crate) enum BarOutput {
    ClockClicked,
    ClipboardClicked,
    MainMenuClicked,
    NotificationsClicked,
    ScreenshotClicked,
    AppLauncherClicked,
    WallpaperClicked,
    NufwClicked,
    NdnsClicked,
    NpodmanClicked,
    NnotesClicked,
    NipClicked,
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
            revealed: reveal_by_default,
            hovered: false,
            _effects: effects,
        };

        let widgets = view_output!();

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
            BarInput::SetMinWidth(min) => {
                self.min_width = min;
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
            BarWidget::AudioInput => Box::new(
                AudioInputModel::builder()
                    .launch(AudioInputInit {})
                    .detach(),
            ),
            BarWidget::AudioOutput => Box::new(
                AudioOutputModel::builder()
                    .launch(AudioOutputInit {})
                    .detach(),
            ),
            BarWidget::Battery => Box::new(BatteryModel::builder().launch(BatteryInit {}).detach()),
            BarWidget::Bluetooth => {
                Box::new(BluetoothModel::builder().launch(BluetoothInit {}).detach())
            }
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
                    .detach(),
            ),
            BarWidget::MargoTags => Box::new(
                MargoTagsModel::builder()
                    .launch(MargoTagsInit { orientation })
                    .detach(),
            ),
            BarWidget::HyprPicker => Box::new(
                HyprPickerModel::builder()
                    .launch(HyprPickerInit { orientation })
                    .detach(),
            ),
            BarWidget::Lock => Box::new(
                LockModel::builder()
                    .launch(LockInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        LockOutput::CloseMenu => BarOutput::CloseMenu,
                    }),
            ),
            BarWidget::Logout => Box::new(
                LogoutModel::builder()
                    .launch(LogoutInit { orientation })
                    .detach(),
            ),
            BarWidget::QuickSettings => Box::new(
                QuickSettingsModel::builder()
                    .launch(QuickSettingsInit { orientation })
                    .forward(sender.output_sender(), |msg| match msg {
                        QuickSettingOutput::Clicked => BarOutput::MainMenuClicked,
                    }),
            ),
            BarWidget::Network => Box::new(NetworkModel::builder().launch(NetworkInit {}).detach()),
            BarWidget::Ndns => Box::new(
                NdnsModel::builder()
                    .launch(NdnsInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::ndns::NdnsOutput::Clicked => {
                            BarOutput::NdnsClicked
                        }
                    }),
            ),
            BarWidget::Nip => Box::new(
                NipModel::builder()
                    .launch(NipInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::nip::NipOutput::Clicked => {
                            BarOutput::NipClicked
                        }
                    }),
            ),
            BarWidget::Nnotes => Box::new(
                NnotesModel::builder()
                    .launch(NnotesInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::nnotes::NnotesOutput::Clicked => {
                            BarOutput::NnotesClicked
                        }
                    }),
            ),
            BarWidget::Npodman => Box::new(
                NpodmanModel::builder()
                    .launch(NpodmanInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::npodman::NpodmanOutput::Clicked => {
                            BarOutput::NpodmanClicked
                        }
                    }),
            ),
            BarWidget::Nufw => Box::new(
                NufwModel::builder()
                    .launch(NufwInit {})
                    .forward(sender.output_sender(), |msg| match msg {
                        crate::bars::bar_widgets::nufw::NufwOutput::Clicked => {
                            BarOutput::NufwClicked
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
            BarWidget::PowerProfile => Box::new(
                PowerProfileModel::builder()
                    .launch(PowerProfileInit {})
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
            BarWidget::VpnIndicator => Box::new(
                VpnIndicatorModel::builder()
                    .launch(VpnIndicatorInit {})
                    .detach(),
            ),
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
