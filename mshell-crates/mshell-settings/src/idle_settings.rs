use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct IdleSettingsModel {
    dim_enabled: bool,
    dim_timeout: u32,
    lock_enabled: bool,
    lock_timeout: u32,
    suspend_enabled: bool,
    suspend_timeout: u32,
    inhibit_while_media: bool,
    _effects: EffectScope,
}

reactive_settings! {
    model: IdleSettingsModel,
    input: IdleSettingsInput,
    group: idle,
    fields {
        DimEnabled => dim_enabled: bool => dim_enabled,
        DimTimeout => dim_timeout: u32 => dim_timeout_minutes,
        LockEnabled => lock_enabled: bool => lock_enabled,
        LockTimeout => lock_timeout: u32 => lock_timeout_minutes,
        SuspendEnabled => suspend_enabled: bool => suspend_enabled,
        SuspendTimeout => suspend_timeout: u32 => suspend_timeout_minutes,
        InhibitWhileMedia => inhibit_while_media: bool => inhibit_while_media,
    }
}

#[derive(Debug)]
pub(crate) enum IdleSettingsOutput {}

pub(crate) struct IdleSettingsInit {}

#[derive(Debug)]
pub(crate) enum IdleSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for IdleSettingsModel {
    type CommandOutput = IdleSettingsCommandOutput;
    type Input = IdleSettingsInput;
    type Output = IdleSettingsOutput;
    type Init = IdleSettingsInit;

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
                        // `system-suspend-symbolic` doesn't ship in
                        // MargoMaterial OR Adwaita. `coffee-symbolic`
                        // is in MargoMaterial and matches the sidebar
                        // entry's icon — consistent left/right framing.
                        set_icon_name: Some("coffee-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Idle",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Staged actions as the session sits idle — dim < lock < suspend, measured from the last input.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Dim ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Dim Screen",
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
                                set_label: "Enabled",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Dim the screen with a translucent overlay.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(dim_enabled_handler)]
                            set_active: model.dim_enabled,
                            connect_state_set[sender] => move |_, enabled| {
                                sender.input(IdleSettingsInput::DimEnabledChanged(enabled));
                                glib::Propagation::Proceed
                            } @dim_enabled_handler,
                        },
                    },

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
                                set_label: "Timeout (minutes)",
                                set_hexpand: true,
                            },
                        },

                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 1440.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(dim_timeout_handler)]
                            set_value: model.dim_timeout as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(IdleSettingsInput::DimTimeoutChanged(s.value() as u32));
                            } @dim_timeout_handler,
                        },
                    },
                },

                // ── Lock ────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Lock Screen",
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
                                set_label: "Enabled",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Activate the lock screen.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(lock_enabled_handler)]
                            set_active: model.lock_enabled,
                            connect_state_set[sender] => move |_, enabled| {
                                sender.input(IdleSettingsInput::LockEnabledChanged(enabled));
                                glib::Propagation::Proceed
                            } @lock_enabled_handler,
                        },
                    },

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
                                set_label: "Timeout (minutes)",
                                set_hexpand: true,
                            },
                        },

                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 1440.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(lock_timeout_handler)]
                            set_value: model.lock_timeout as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(IdleSettingsInput::LockTimeoutChanged(s.value() as u32));
                            } @lock_timeout_handler,
                        },
                    },
                },

                // ── Suspend ─────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Suspend",
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
                                set_label: "Enabled",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Suspend the system (systemctl suspend).",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(suspend_enabled_handler)]
                            set_active: model.suspend_enabled,
                            connect_state_set[sender] => move |_, enabled| {
                                sender.input(IdleSettingsInput::SuspendEnabledChanged(enabled));
                                glib::Propagation::Proceed
                            } @suspend_enabled_handler,
                        },
                    },

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
                                set_label: "Timeout (minutes)",
                                set_hexpand: true,
                            },
                        },

                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 1440.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(suspend_timeout_handler)]
                            set_value: model.suspend_timeout as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(IdleSettingsInput::SuspendTimeoutChanged(s.value() as u32));
                            } @suspend_timeout_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Media",
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
                                set_label: "Keep awake while media plays",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Hold the idle inhibitor while any media player is playing, and restore the previous state when it stops. Overrides the idle / lock / suspend timers above during playback.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(inhibit_media_handler)]
                            set_active: model.inhibit_while_media,
                            connect_state_set[sender] => move |_, enabled| {
                                sender.input(IdleSettingsInput::InhibitWhileMediaChanged(enabled));
                                glib::Propagation::Proceed
                            } @inhibit_media_handler,
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
        let model = Self::from_config_store(&sender);
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        self.apply_reactive(message);
    }
}
