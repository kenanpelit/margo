use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, MenuStoreFields, MenusStoreFields, NotificationsStoreFields,
};
use mshell_config::schema::position::NotificationPosition;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct NotificationSettingsModel {
    position: NotificationPosition,
    show_close_button: bool,
    show_action_buttons: bool,
    group_notifications: bool,
    popup_width: i32,
    show_timeout_bar: bool,
    popup_duration_ms: u32,
    history_limit: u32,
    /// History-menu surface size (config.menus.notification_menu) — the
    /// panel that opens on the bar pill, distinct from the popup toasts.
    menu_min_width: i32,
    menu_max_height: i32,
    blocklist: Vec<String>,
    inline_reply: bool,
    show_progress: bool,
    sound_enabled: bool,
    sound_low: bool,
    sound_normal: bool,
    sound_critical: bool,
    sound_from_client: bool,
    quiet_enabled: bool,
    quiet_start: String,
    quiet_end: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotificationSettingsInput {
    PositionChanged(NotificationPosition),
    PositionEffect(NotificationPosition),
    ShowCloseChanged(bool),
    ShowCloseEffect(bool),
    ShowActionsChanged(bool),
    ShowActionsEffect(bool),
    GroupChanged(bool),
    GroupEffect(bool),
    PopupWidthChanged(i32),
    PopupWidthEffect(i32),
    ShowTimeoutBarChanged(bool),
    PopupDurationChanged(u32),
    HistoryLimitChanged(u32),
    HistoryLimitEffect(u32),
    MenuMinWidthChanged(i32),
    MenuMinWidthEffect(i32),
    MenuMaxHeightChanged(i32),
    MenuMaxHeightEffect(i32),
    BlocklistAdd(String),
    BlocklistRemove(String),
    BlocklistEffect(Vec<String>),
    InlineReplyChanged(bool),
    ShowProgressChanged(bool),
    SoundEnabledChanged(bool),
    SoundLowChanged(bool),
    SoundNormalChanged(bool),
    SoundCriticalChanged(bool),
    SoundFromClientChanged(bool),
    QuietEnabledChanged(bool),
    QuietStartChanged(String),
    QuietEndChanged(String),
    /// One grouped mirror of the reply/progress/sound block — fired by a
    /// single effect subscribed to all ten fields.
    ReplySoundEffect {
        inline_reply: bool,
        show_progress: bool,
        sound_enabled: bool,
        sound_low: bool,
        sound_normal: bool,
        sound_critical: bool,
        sound_from_client: bool,
        quiet_enabled: bool,
        quiet_start: String,
        quiet_end: String,
    },
}

#[derive(Debug)]
pub(crate) enum NotificationSettingsOutput {}

pub(crate) struct NotificationSettingsInit {}

#[derive(Debug)]
pub(crate) enum NotificationSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for NotificationSettingsModel {
    type CommandOutput = NotificationSettingsCommandOutput;
    type Input = NotificationSettingsInput;
    type Output = NotificationSettingsOutput;
    type Init = NotificationSettingsInit;

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
                        set_icon_name: Some("dialog-information-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Notifications",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Toast geometry, inline reply, sounds & quiet hours, progress bars, history retention.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Notifications",
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

                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Position",
                                set_hexpand: true,
                            },

                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Where popup notifications should be positioned.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::DropDown {
                            set_width_request: 150,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&NotificationPosition::display_names())),
                            #[watch]
                            #[block_signal(handler)]
                            set_selected: model.position.to_index(),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(NotificationSettingsInput::PositionChanged(
                                    NotificationPosition::from_index(dd.selected())
                                ));
                            } @handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Toast content",
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
                                set_label: "Close button",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show the small ✕ button on each notification (swipe also dismisses).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "show_close_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(close_handler)]
                            set_active: model.show_close_button,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::ShowCloseChanged(s.is_active()));
                            } @close_handler,
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
                                set_label: "Action buttons",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show app-provided buttons (View / Open / Reply / …). Off keeps toasts clean.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "show_actions_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(actions_handler)]
                            set_active: model.show_action_buttons,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::ShowActionsChanged(s.is_active()));
                            } @actions_handler,
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
                                set_label: "Popup width",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Width (px) of the corner popup toasts. Separate from the history menu width in Widgets → Notifications.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "popup_width_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (200.0, 1200.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(popup_width_handler)]
                            set_value: model.popup_width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(NotificationSettingsInput::PopupWidthChanged(s.value() as i32));
                            } @popup_width_handler,
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
                                set_label: "Timeout bar",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show a shrinking bar across the top of each popup counting down its on-screen time.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "timeout_bar_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(timeout_bar_handler)]
                            set_active: model.show_timeout_bar,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::ShowTimeoutBarChanged(s.is_active()));
                            } @timeout_bar_handler,
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
                                set_label: "Popup duration",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "How long (ms) a popup stays on screen. A shorter app-supplied timeout still wins.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "popup_duration_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1000.0, 60000.0),
                            set_increments: (500.0, 2000.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(popup_duration_handler)]
                            set_value: model.popup_duration_ms as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(NotificationSettingsInput::PopupDurationChanged(s.value() as u32));
                            } @popup_duration_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Reply, progress & sound",
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
                                set_label: "Inline reply",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show a reply box on notifications that support it (chat apps, Valent SMS). Sending answers straight from the toast.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "inline_reply_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(inline_reply_handler)]
                            set_active: model.inline_reply,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::InlineReplyChanged(s.is_active()));
                            } @inline_reply_handler,
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
                                set_label: "Progress bars",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Render a progress bar on notifications that report one (downloads, file transfers, backups).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "show_progress_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(progress_handler)]
                            set_active: model.show_progress,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::ShowProgressChanged(s.is_active()));
                            } @progress_handler,
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
                                set_label: "Notification sounds",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Play a chime when a popup appears. Do Not Disturb always silences (it suppresses the popups themselves).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "sound_enabled_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(snd_en_handler)]
                            set_active: model.sound_enabled,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::SoundEnabledChanged(s.is_active()));
                            } @snd_en_handler,
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
                                set_label: "Low urgency",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Sound for low-urgency notifications.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "sound_low_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: model.sound_enabled,
                            #[watch]
                            #[block_signal(snd_low_handler)]
                            set_active: model.sound_low,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::SoundLowChanged(s.is_active()));
                            } @snd_low_handler,
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
                                set_label: "Normal urgency",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Sound for normal-urgency notifications.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "sound_normal_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: model.sound_enabled,
                            #[watch]
                            #[block_signal(snd_norm_handler)]
                            set_active: model.sound_normal,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::SoundNormalChanged(s.is_active()));
                            } @snd_norm_handler,
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
                                set_label: "Critical urgency",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Sound for critical notifications (a brighter, rising tone).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "sound_critical_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: model.sound_enabled,
                            #[watch]
                            #[block_signal(snd_crit_handler)]
                            set_active: model.sound_critical,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::SoundCriticalChanged(s.is_active()));
                            } @snd_crit_handler,
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
                                set_label: "App-provided sounds",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "When an app supplies its own sound file, play that instead of the built-in chime.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "sound_client_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: model.sound_enabled,
                            #[watch]
                            #[block_signal(snd_client_handler)]
                            set_active: model.sound_from_client,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::SoundFromClientChanged(s.is_active()));
                            } @snd_client_handler,
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
                                set_label: "Quiet hours",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Mute notification sounds inside the window below (popups still show). An end before the start wraps past midnight.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "quiet_enabled_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_sensitive: model.sound_enabled,
                            #[watch]
                            #[block_signal(quiet_handler)]
                            set_active: model.quiet_enabled,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::QuietEnabledChanged(s.is_active()));
                            } @quiet_handler,
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    #[watch]
                    set_sensitive: model.sound_enabled && model.quiet_enabled,

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "From",
                    },
                    #[name = "quiet_start_entry"]
                    gtk::Entry {
                        add_css_class: "ok-entry-with-border",
                        set_width_chars: 6,
                        set_max_length: 5,
                        set_placeholder_text: Some("22:00"),
                        connect_changed[sender] => move |e| {
                            sender.input(NotificationSettingsInput::QuietStartChanged(e.text().to_string()));
                        },
                    },
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "to",
                    },
                    #[name = "quiet_end_entry"]
                    gtk::Entry {
                        add_css_class: "ok-entry-with-border",
                        set_width_chars: 6,
                        set_max_length: 5,
                        set_placeholder_text: Some("08:00"),
                        connect_changed[sender] => move |e| {
                            sender.input(NotificationSettingsInput::QuietEndChanged(e.text().to_string()));
                        },
                    },
                },

                // ── History menu size (the panel opened from the bar pill) ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "History menu",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
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
                                set_label: "Width",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Width (px) of the notification history menu — the panel that opens when you click the bar pill. Separate from the popup toast width above.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "menu_width_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (280.0, 1200.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(menu_width_handler)]
                            set_value: model.menu_min_width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(NotificationSettingsInput::MenuMinWidthChanged(s.value() as i32));
                            } @menu_width_handler,
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
                                set_label: "Max height",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Maximum height (px) before the history scrolls. 0 = grow to fit (no cap).",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "menu_height_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 2000.0),
                            set_increments: (20.0, 100.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(menu_height_handler)]
                            set_value: model.menu_max_height as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(NotificationSettingsInput::MenuMaxHeightChanged(s.value() as i32));
                            } @menu_height_handler,
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
                                set_label: "History limit",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Max number of recent notifications the history renders. A large persisted history slows the first open; lower this if it lags. 0 = unlimited.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "history_limit_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 250.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(history_limit_handler)]
                            set_value: model.history_limit as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(NotificationSettingsInput::HistoryLimitChanged(s.value() as u32));
                            } @history_limit_handler,
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
                                set_label: "Group by app",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Collapse 2+ notifications from the same app into an expandable group in the history. Off = flat list.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        #[name = "group_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(group_handler)]
                            set_active: model.group_notifications,
                            connect_active_notify[sender] => move |s| {
                                sender.input(NotificationSettingsInput::GroupChanged(s.is_active()));
                            } @group_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Muted apps",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_label: "Notifications whose app name contains one of these entries (case-insensitive) are silently dropped. Type an app name and press Enter or Add.",
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    #[name = "blocklist_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("App name (e.g. Spotify)"),
                    },

                    #[name = "blocklist_add"]
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_label: "Add",
                    },
                },

                #[name = "blocklist_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.notifications().notification_position().get();
            sender_clone.input(NotificationSettingsInput::PositionEffect(value));
        });

        // Mirror external blocklist edits (e.g. hand-edited YAML) back
        // into the UI.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let list = config_manager().config().notifications().blocklist().get();
            sender_clone.input(NotificationSettingsInput::BlocklistEffect(list));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .show_close_button()
                .get();
            sender_clone.input(NotificationSettingsInput::ShowCloseEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .show_action_buttons()
                .get();
            sender_clone.input(NotificationSettingsInput::ShowActionsEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .group_notifications()
                .get();
            sender_clone.input(NotificationSettingsInput::GroupEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .popup_width()
                .get();
            sender_clone.input(NotificationSettingsInput::PopupWidthEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .history_limit()
                .get();
            sender_clone.input(NotificationSettingsInput::HistoryLimitEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .menus()
                .notification_menu()
                .minimum_width()
                .get();
            sender_clone.input(NotificationSettingsInput::MenuMinWidthEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .menus()
                .notification_menu()
                .maximum_height()
                .get();
            sender_clone.input(NotificationSettingsInput::MenuMaxHeightEffect(v));
        });

        // One grouped mirror for the reply/progress/sound block: reading
        // all ten fields subscribes this effect to each, so any external
        // change re-sends the whole snapshot.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let c = config_manager().config();
            sender_clone.input(NotificationSettingsInput::ReplySoundEffect {
                inline_reply: c.clone().notifications().inline_reply().get(),
                show_progress: c.clone().notifications().show_progress().get(),
                sound_enabled: c.clone().notifications().sound_enabled().get(),
                sound_low: c.clone().notifications().sound_low().get(),
                sound_normal: c.clone().notifications().sound_normal().get(),
                sound_critical: c.clone().notifications().sound_critical().get(),
                sound_from_client: c.clone().notifications().sound_from_client().get(),
                quiet_enabled: c.clone().notifications().quiet_hours_enabled().get(),
                quiet_start: c.clone().notifications().quiet_hours_start().get(),
                quiet_end: c.notifications().quiet_hours_end().get(),
            });
        });

        let model = NotificationSettingsModel {
            position: config_manager()
                .config()
                .notifications()
                .notification_position()
                .get_untracked(),
            show_close_button: config_manager()
                .config()
                .notifications()
                .show_close_button()
                .get_untracked(),
            show_action_buttons: config_manager()
                .config()
                .notifications()
                .show_action_buttons()
                .get_untracked(),
            group_notifications: config_manager()
                .config()
                .notifications()
                .group_notifications()
                .get_untracked(),
            popup_width: config_manager()
                .config()
                .notifications()
                .popup_width()
                .get_untracked(),
            show_timeout_bar: config_manager()
                .config()
                .notifications()
                .show_timeout_bar()
                .get_untracked(),
            popup_duration_ms: config_manager()
                .config()
                .notifications()
                .popup_duration_ms()
                .get_untracked(),
            history_limit: config_manager()
                .config()
                .notifications()
                .history_limit()
                .get_untracked(),
            menu_min_width: config_manager()
                .config()
                .menus()
                .notification_menu()
                .minimum_width()
                .get_untracked(),
            menu_max_height: config_manager()
                .config()
                .menus()
                .notification_menu()
                .maximum_height()
                .get_untracked(),
            blocklist: config_manager()
                .config()
                .notifications()
                .blocklist()
                .get_untracked(),
            inline_reply: config_manager()
                .config()
                .notifications()
                .inline_reply()
                .get_untracked(),
            show_progress: config_manager()
                .config()
                .notifications()
                .show_progress()
                .get_untracked(),
            sound_enabled: config_manager()
                .config()
                .notifications()
                .sound_enabled()
                .get_untracked(),
            sound_low: config_manager()
                .config()
                .notifications()
                .sound_low()
                .get_untracked(),
            sound_normal: config_manager()
                .config()
                .notifications()
                .sound_normal()
                .get_untracked(),
            sound_critical: config_manager()
                .config()
                .notifications()
                .sound_critical()
                .get_untracked(),
            sound_from_client: config_manager()
                .config()
                .notifications()
                .sound_from_client()
                .get_untracked(),
            quiet_enabled: config_manager()
                .config()
                .notifications()
                .quiet_hours_enabled()
                .get_untracked(),
            quiet_start: config_manager()
                .config()
                .notifications()
                .quiet_hours_start()
                .get_untracked(),
            quiet_end: config_manager()
                .config()
                .notifications()
                .quiet_hours_end()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

        // Seed the quiet-hours entries once. They are deliberately not
        // #[watch]-bound: re-setting the text on every model change would
        // fight the user's in-progress typing.
        widgets.quiet_start_entry.set_text(&model.quiet_start);
        widgets.quiet_end_entry.set_text(&model.quiet_end);

        // Wire the add entry + button, and paint the initial rows.
        let entry = widgets.blocklist_entry.clone();
        let sender_clone = sender.clone();
        let submit =
            move |entry: &gtk::Entry, sender: &ComponentSender<NotificationSettingsModel>| {
                let name = entry.text().trim().to_string();
                if !name.is_empty() {
                    sender.input(NotificationSettingsInput::BlocklistAdd(name));
                    entry.set_text("");
                }
            };
        {
            let entry = entry.clone();
            let sender = sender_clone.clone();
            widgets
                .blocklist_add
                .connect_clicked(move |_| submit(&entry, &sender));
        }
        {
            let sender = sender_clone.clone();
            widgets
                .blocklist_entry
                .connect_activate(move |e| submit(e, &sender));
        }
        rebuild_blocklist_rows(&widgets.blocklist_list, &model.blocklist, &sender);

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
            NotificationSettingsInput::PositionChanged(position) => {
                self.position = position.clone();
                config_manager().update_config(|config| {
                    config.notifications.notification_position = position;
                });
            }
            NotificationSettingsInput::PositionEffect(position) => {
                self.position = position;
            }
            NotificationSettingsInput::ShowCloseChanged(v) => {
                self.show_close_button = v;
                config_manager().update_config(move |config| {
                    config.notifications.show_close_button = v;
                });
            }
            NotificationSettingsInput::ShowCloseEffect(v) => {
                self.show_close_button = v;
            }
            NotificationSettingsInput::ShowActionsChanged(v) => {
                self.show_action_buttons = v;
                config_manager().update_config(move |config| {
                    config.notifications.show_action_buttons = v;
                });
            }
            NotificationSettingsInput::ShowActionsEffect(v) => {
                self.show_action_buttons = v;
            }
            NotificationSettingsInput::GroupChanged(v) => {
                self.group_notifications = v;
                config_manager().update_config(move |config| {
                    config.notifications.group_notifications = v;
                });
            }
            NotificationSettingsInput::GroupEffect(v) => {
                self.group_notifications = v;
            }
            NotificationSettingsInput::PopupWidthChanged(w) => {
                self.popup_width = w;
                config_manager().update_config(move |config| {
                    config.notifications.popup_width = w;
                });
            }
            NotificationSettingsInput::ShowTimeoutBarChanged(v) => {
                self.show_timeout_bar = v;
                config_manager().update_config(move |config| {
                    config.notifications.show_timeout_bar = v;
                });
            }
            NotificationSettingsInput::PopupDurationChanged(v) => {
                let v = v.max(1);
                self.popup_duration_ms = v;
                config_manager().update_config(move |config| {
                    config.notifications.popup_duration_ms = v;
                });
            }
            NotificationSettingsInput::HistoryLimitChanged(v) => {
                self.history_limit = v;
                config_manager().update_config(move |config| {
                    config.notifications.history_limit = v;
                });
            }
            NotificationSettingsInput::HistoryLimitEffect(v) => {
                self.history_limit = v;
            }
            NotificationSettingsInput::PopupWidthEffect(w) => {
                self.popup_width = w;
            }
            NotificationSettingsInput::MenuMinWidthChanged(w) => {
                let w = w.max(1);
                self.menu_min_width = w;
                config_manager().update_config(move |config| {
                    config.menus.notification_menu.minimum_width = w;
                });
            }
            NotificationSettingsInput::MenuMinWidthEffect(w) => {
                self.menu_min_width = w;
            }
            NotificationSettingsInput::MenuMaxHeightChanged(h) => {
                let h = h.max(0);
                self.menu_max_height = h;
                config_manager().update_config(move |config| {
                    config.menus.notification_menu.maximum_height = h;
                });
            }
            NotificationSettingsInput::MenuMaxHeightEffect(h) => {
                self.menu_max_height = h;
            }
            NotificationSettingsInput::InlineReplyChanged(v) => {
                self.inline_reply = v;
                config_manager().update_config(|c| c.notifications.inline_reply = v);
            }
            NotificationSettingsInput::ShowProgressChanged(v) => {
                self.show_progress = v;
                config_manager().update_config(|c| c.notifications.show_progress = v);
            }
            NotificationSettingsInput::SoundEnabledChanged(v) => {
                self.sound_enabled = v;
                config_manager().update_config(|c| c.notifications.sound_enabled = v);
            }
            NotificationSettingsInput::SoundLowChanged(v) => {
                self.sound_low = v;
                config_manager().update_config(|c| c.notifications.sound_low = v);
            }
            NotificationSettingsInput::SoundNormalChanged(v) => {
                self.sound_normal = v;
                config_manager().update_config(|c| c.notifications.sound_normal = v);
            }
            NotificationSettingsInput::SoundCriticalChanged(v) => {
                self.sound_critical = v;
                config_manager().update_config(|c| c.notifications.sound_critical = v);
            }
            NotificationSettingsInput::SoundFromClientChanged(v) => {
                self.sound_from_client = v;
                config_manager().update_config(|c| c.notifications.sound_from_client = v);
            }
            NotificationSettingsInput::QuietEnabledChanged(v) => {
                self.quiet_enabled = v;
                config_manager().update_config(|c| c.notifications.quiet_hours_enabled = v);
            }
            NotificationSettingsInput::QuietStartChanged(v) => {
                // Commit only well-formed HH:MM values; partial input while
                // typing stays local to the entry.
                if parse_hhmm(&v) {
                    self.quiet_start = v.clone();
                    config_manager().update_config(|c| c.notifications.quiet_hours_start = v);
                }
            }
            NotificationSettingsInput::QuietEndChanged(v) => {
                if parse_hhmm(&v) {
                    self.quiet_end = v.clone();
                    config_manager().update_config(|c| c.notifications.quiet_hours_end = v);
                }
            }
            NotificationSettingsInput::ReplySoundEffect {
                inline_reply,
                show_progress,
                sound_enabled,
                sound_low,
                sound_normal,
                sound_critical,
                sound_from_client,
                quiet_enabled,
                quiet_start,
                quiet_end,
            } => {
                self.inline_reply = inline_reply;
                self.show_progress = show_progress;
                self.sound_enabled = sound_enabled;
                self.sound_low = sound_low;
                self.sound_normal = sound_normal;
                self.sound_critical = sound_critical;
                self.sound_from_client = sound_from_client;
                self.quiet_enabled = quiet_enabled;
                self.quiet_start = quiet_start;
                self.quiet_end = quiet_end;
            }
            NotificationSettingsInput::BlocklistAdd(name) => {
                let exists = self.blocklist.iter().any(|e| e.eq_ignore_ascii_case(&name));
                if !exists {
                    self.blocklist.push(name);
                    let list = self.blocklist.clone();
                    config_manager().update_config(move |config| {
                        config.notifications.blocklist = list;
                    });
                    rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
                }
            }
            NotificationSettingsInput::BlocklistRemove(name) => {
                self.blocklist.retain(|e| e != &name);
                let list = self.blocklist.clone();
                config_manager().update_config(move |config| {
                    config.notifications.blocklist = list;
                });
                rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
            }
            NotificationSettingsInput::BlocklistEffect(list) => {
                if list != self.blocklist {
                    self.blocklist = list;
                    rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Repaint the muted-apps list: one row per entry with a remove ✕.
fn rebuild_blocklist_rows(
    list: &gtk::Box,
    items: &[String],
    sender: &ComponentSender<NotificationSettingsModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for name in items {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("notification-mute-row");

        let label = gtk::Label::new(Some(name));
        label.add_css_class("label-medium");
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row.append(&label);

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        let name = name.clone();
        let sender = sender.clone();
        remove.connect_clicked(move |_| {
            sender.input(NotificationSettingsInput::BlocklistRemove(name.clone()));
        });
        row.append(&remove);

        list.append(&row);
    }
}

/// Whether `s` is a well-formed `HH:MM` (24-hour) clock string.
fn parse_hhmm(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else {
        return false;
    };
    let (Ok(h), Ok(m)) = (h.trim().parse::<u8>(), m.trim().parse::<u8>()) else {
        return false;
    };
    h < 24 && m < 60
}
