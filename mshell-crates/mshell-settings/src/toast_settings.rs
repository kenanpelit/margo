//! Settings → Toasts. Per-event switches for the state-change toast surface
//! (`mshell-osd::toast`) plus the battery warning toggle + critical level.
//! Shell-owned config (`config.toasts`) read/written through the reactive
//! store. Copied from `idle_settings.rs` (DESIGN.md §8b page shape).
//!
//! The battery *warn levels* ladder (`battery_warn_levels`, default 20/10/5)
//! stays config-only — edit it in the YAML profile; the default suits most.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ToastsStoreFields};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ToastSettingsModel {
    enabled: bool,
    charging: bool,
    lock_keys: bool,
    kb_layout: bool,
    audio_device: bool,
    vpn: bool,
    now_playing: bool,
    network: bool,
    bluetooth: bool,
    power_profile: bool,
    dnd: bool,
    idle_inhibitor: bool,
    game_mode: bool,
    battery: bool,
    battery_critical: u32,
    position: Position,
    distance: i32,
    width: i32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ToastSettingsInput {
    EnabledChanged(bool),
    ChargingChanged(bool),
    LockKeysChanged(bool),
    KbLayoutChanged(bool),
    AudioDeviceChanged(bool),
    VpnChanged(bool),
    NowPlayingChanged(bool),
    NetworkChanged(bool),
    BluetoothChanged(bool),
    PowerProfileChanged(bool),
    DndChanged(bool),
    IdleInhibitorChanged(bool),
    GameModeChanged(bool),
    BatteryChanged(bool),
    BatteryCriticalChanged(u32),
    PositionChanged(Position),
    DistanceChanged(i32),
    WidthChanged(i32),

    EnabledEffect(bool),
    ChargingEffect(bool),
    LockKeysEffect(bool),
    KbLayoutEffect(bool),
    AudioDeviceEffect(bool),
    VpnEffect(bool),
    NowPlayingEffect(bool),
    NetworkEffect(bool),
    BluetoothEffect(bool),
    PowerProfileEffect(bool),
    DndEffect(bool),
    IdleInhibitorEffect(bool),
    GameModeEffect(bool),
    BatteryEffect(bool),
    BatteryCriticalEffect(u8),
    PositionEffect(Position),
    DistanceEffect(i32),
    WidthEffect(i32),
}

#[derive(Debug)]
pub(crate) enum ToastSettingsOutput {}

pub(crate) struct ToastSettingsInit {}

#[derive(Debug)]
pub(crate) enum ToastSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for ToastSettingsModel {
    type CommandOutput = ToastSettingsCommandOutput;
    type Input = ToastSettingsInput;
    type Output = ToastSettingsOutput;
    type Init = ToastSettingsInit;

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
                        set_icon_name: Some("preferences-system-notifications-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Toasts",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Brief pop-up cards announcing system state changes — separate from app notifications and the volume/brightness OSD.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Placement & size ─────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Placement & size",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Position",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Where toasts appear — a screen edge or corner. Applies after restarting mshell (systemctl --user restart mshell).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_width_request: 150,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&Position::display_names())),
                            #[watch]
                            #[block_signal(position_handler)]
                            set_selected: model.position.to_index(),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(ToastSettingsInput::PositionChanged(
                                    Position::from_index(dd.selected())
                                ));
                            } @position_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Distance (px)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Margin from the docked edge(s). Applies after an mshell restart.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 400.0),
                            set_increments: (2.0, 16.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(distance_handler)]
                            set_value: model.distance as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(ToastSettingsInput::DistanceChanged(s.value() as i32));
                            } @distance_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Width (px)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Fixed card width; 0 sizes the card to its content. Applies after an mshell restart.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 1200.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(width_handler)]
                            set_value: model.width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(ToastSettingsInput::WidthChanged(s.value() as i32));
                            } @width_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Events",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Enabled",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Master switch for all state-change toasts.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(enabled_handler)]
                            set_active: model.enabled,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::EnabledChanged(v));
                                glib::Propagation::Proceed
                            } @enabled_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "AC power",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the charger is plugged in or unplugged.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(charging_handler)]
                            set_active: model.charging,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::ChargingChanged(v));
                                glib::Propagation::Proceed
                            } @charging_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Lock keys",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When Caps Lock or Num Lock toggles.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(lock_keys_handler)]
                            set_active: model.lock_keys,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::LockKeysChanged(v));
                                glib::Propagation::Proceed
                            } @lock_keys_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Keyboard layout",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the active keyboard layout changes.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(kb_layout_handler)]
                            set_active: model.kb_layout,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::KbLayoutChanged(v));
                                glib::Propagation::Proceed
                            } @kb_layout_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Audio device",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the default output or input device changes.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(audio_device_handler)]
                            set_active: model.audio_device,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::AudioDeviceChanged(v));
                                glib::Propagation::Proceed
                            } @audio_device_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "VPN",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the VPN tunnel connects or disconnects.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(vpn_handler)]
                            set_active: model.vpn,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::VpnChanged(v));
                                glib::Propagation::Proceed
                            } @vpn_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Now playing",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "On every track change. Off by default — it's noisy.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(now_playing_handler)]
                            set_active: model.now_playing,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::NowPlayingChanged(v));
                                glib::Propagation::Proceed
                            } @now_playing_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Network",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the primary Wi-Fi or wired link connects or disconnects.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(network_handler)]
                            set_active: model.network,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::NetworkChanged(v));
                                glib::Propagation::Proceed
                            } @network_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Bluetooth",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When a Bluetooth device connects or disconnects.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(bluetooth_handler)]
                            set_active: model.bluetooth,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::BluetoothChanged(v));
                                glib::Propagation::Proceed
                            } @bluetooth_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Power profile",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the power profile changes (performance / balanced / power-saver).",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(power_profile_handler)]
                            set_active: model.power_profile,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::PowerProfileChanged(v));
                                glib::Propagation::Proceed
                            } @power_profile_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Do Not Disturb",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When Do Not Disturb is turned on or off.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(dnd_handler)]
                            set_active: model.dnd,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::DndChanged(v));
                                glib::Propagation::Proceed
                            } @dnd_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Keep awake",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When the idle inhibitor (keep awake) is turned on or off.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(idle_inhibitor_handler)]
                            set_active: model.idle_inhibitor,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::IdleInhibitorChanged(v));
                                glib::Propagation::Proceed
                            } @idle_inhibitor_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Game Mode",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When Game Mode is engaged or released.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(game_mode_handler)]
                            set_active: model.game_mode,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::GameModeChanged(v));
                                glib::Propagation::Proceed
                            } @game_mode_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Battery",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Warnings",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Warn as the battery crosses low levels (20/10/5%) and a danger toast at the critical level.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(battery_handler)]
                            set_active: model.battery,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ToastSettingsInput::BatteryChanged(v));
                                glib::Propagation::Proceed
                            } @battery_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Critical level (%)",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "0 disables the critical step; the warning levels still apply.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 50.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(battery_critical_handler)]
                            set_value: model.battery_critical as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(ToastSettingsInput::BatteryCriticalChanged(s.value() as u32));
                            } @battery_critical_handler,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        macro_rules! push_effect {
            ($field:ident, $variant:ident) => {{
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let value = config_manager().config().toasts().$field().get();
                    sender_clone.input(ToastSettingsInput::$variant(value));
                });
            }};
        }
        push_effect!(enabled, EnabledEffect);
        push_effect!(charging, ChargingEffect);
        push_effect!(lock_keys, LockKeysEffect);
        push_effect!(kb_layout, KbLayoutEffect);
        push_effect!(audio_device, AudioDeviceEffect);
        push_effect!(vpn, VpnEffect);
        push_effect!(now_playing, NowPlayingEffect);
        push_effect!(network, NetworkEffect);
        push_effect!(bluetooth, BluetoothEffect);
        push_effect!(power_profile, PowerProfileEffect);
        push_effect!(dnd, DndEffect);
        push_effect!(idle_inhibitor, IdleInhibitorEffect);
        push_effect!(game_mode, GameModeEffect);
        push_effect!(battery, BatteryEffect);
        push_effect!(battery_critical_level, BatteryCriticalEffect);
        push_effect!(position, PositionEffect);
        push_effect!(distance, DistanceEffect);
        push_effect!(width, WidthEffect);

        let model = ToastSettingsModel {
            enabled: config_manager().config().toasts().enabled().get_untracked(),
            charging: config_manager()
                .config()
                .toasts()
                .charging()
                .get_untracked(),
            lock_keys: config_manager()
                .config()
                .toasts()
                .lock_keys()
                .get_untracked(),
            kb_layout: config_manager()
                .config()
                .toasts()
                .kb_layout()
                .get_untracked(),
            audio_device: config_manager()
                .config()
                .toasts()
                .audio_device()
                .get_untracked(),
            vpn: config_manager().config().toasts().vpn().get_untracked(),
            now_playing: config_manager()
                .config()
                .toasts()
                .now_playing()
                .get_untracked(),
            network: config_manager().config().toasts().network().get_untracked(),
            bluetooth: config_manager()
                .config()
                .toasts()
                .bluetooth()
                .get_untracked(),
            power_profile: config_manager()
                .config()
                .toasts()
                .power_profile()
                .get_untracked(),
            dnd: config_manager().config().toasts().dnd().get_untracked(),
            idle_inhibitor: config_manager()
                .config()
                .toasts()
                .idle_inhibitor()
                .get_untracked(),
            game_mode: config_manager()
                .config()
                .toasts()
                .game_mode()
                .get_untracked(),
            battery: config_manager().config().toasts().battery().get_untracked(),
            battery_critical: config_manager()
                .config()
                .toasts()
                .battery_critical_level()
                .get_untracked() as u32,
            position: config_manager()
                .config()
                .toasts()
                .position()
                .get_untracked(),
            distance: config_manager()
                .config()
                .toasts()
                .distance()
                .get_untracked(),
            width: config_manager().config().toasts().width().get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

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
            ToastSettingsInput::EnabledChanged(v) => {
                config_manager().update_config(|c| c.toasts.enabled = v);
            }
            ToastSettingsInput::ChargingChanged(v) => {
                config_manager().update_config(|c| c.toasts.charging = v);
            }
            ToastSettingsInput::LockKeysChanged(v) => {
                config_manager().update_config(|c| c.toasts.lock_keys = v);
            }
            ToastSettingsInput::KbLayoutChanged(v) => {
                config_manager().update_config(|c| c.toasts.kb_layout = v);
            }
            ToastSettingsInput::AudioDeviceChanged(v) => {
                config_manager().update_config(|c| c.toasts.audio_device = v);
            }
            ToastSettingsInput::VpnChanged(v) => {
                config_manager().update_config(|c| c.toasts.vpn = v);
            }
            ToastSettingsInput::NowPlayingChanged(v) => {
                config_manager().update_config(|c| c.toasts.now_playing = v);
            }
            ToastSettingsInput::NetworkChanged(v) => {
                config_manager().update_config(|c| c.toasts.network = v);
            }
            ToastSettingsInput::BluetoothChanged(v) => {
                config_manager().update_config(|c| c.toasts.bluetooth = v);
            }
            ToastSettingsInput::PowerProfileChanged(v) => {
                config_manager().update_config(|c| c.toasts.power_profile = v);
            }
            ToastSettingsInput::DndChanged(v) => {
                config_manager().update_config(|c| c.toasts.dnd = v);
            }
            ToastSettingsInput::IdleInhibitorChanged(v) => {
                config_manager().update_config(|c| c.toasts.idle_inhibitor = v);
            }
            ToastSettingsInput::GameModeChanged(v) => {
                config_manager().update_config(|c| c.toasts.game_mode = v);
            }
            ToastSettingsInput::BatteryChanged(v) => {
                config_manager().update_config(|c| c.toasts.battery = v);
            }
            ToastSettingsInput::BatteryCriticalChanged(v) => {
                config_manager().update_config(|c| c.toasts.battery_critical_level = v as u8);
            }
            ToastSettingsInput::PositionChanged(p) => {
                config_manager().update_config(|c| c.toasts.position = p.clone());
            }
            ToastSettingsInput::DistanceChanged(v) => {
                let v = v.clamp(0, 400);
                config_manager().update_config(|c| c.toasts.distance = v);
            }
            ToastSettingsInput::WidthChanged(v) => {
                let v = v.clamp(0, 1200);
                config_manager().update_config(|c| c.toasts.width = v);
            }

            ToastSettingsInput::EnabledEffect(v) => self.enabled = v,
            ToastSettingsInput::ChargingEffect(v) => self.charging = v,
            ToastSettingsInput::LockKeysEffect(v) => self.lock_keys = v,
            ToastSettingsInput::KbLayoutEffect(v) => self.kb_layout = v,
            ToastSettingsInput::AudioDeviceEffect(v) => self.audio_device = v,
            ToastSettingsInput::VpnEffect(v) => self.vpn = v,
            ToastSettingsInput::NowPlayingEffect(v) => self.now_playing = v,
            ToastSettingsInput::NetworkEffect(v) => self.network = v,
            ToastSettingsInput::BluetoothEffect(v) => self.bluetooth = v,
            ToastSettingsInput::PowerProfileEffect(v) => self.power_profile = v,
            ToastSettingsInput::DndEffect(v) => self.dnd = v,
            ToastSettingsInput::IdleInhibitorEffect(v) => self.idle_inhibitor = v,
            ToastSettingsInput::GameModeEffect(v) => self.game_mode = v,
            ToastSettingsInput::BatteryEffect(v) => self.battery = v,
            ToastSettingsInput::BatteryCriticalEffect(v) => self.battery_critical = v as u32,
            ToastSettingsInput::PositionEffect(p) => self.position = p,
            ToastSettingsInput::DistanceEffect(v) => self.distance = v,
            ToastSettingsInput::WidthEffect(v) => self.width = v,
        }

        self.update_view(widgets, sender);
    }
}
