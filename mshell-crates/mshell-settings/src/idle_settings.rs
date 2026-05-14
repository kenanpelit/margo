use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::gtk::glib;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct IdleSettingsModel {
    dim_enabled: bool,
    dim_timeout: u32,
    lock_enabled: bool,
    lock_timeout: u32,
    suspend_enabled: bool,
    suspend_timeout: u32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum IdleSettingsInput {
    DimEnabledChanged(bool),
    DimTimeoutChanged(u32),
    LockEnabledChanged(bool),
    LockTimeoutChanged(u32),
    SuspendEnabledChanged(bool),
    SuspendTimeoutChanged(u32),

    DimEnabledEffect(bool),
    DimTimeoutEffect(u32),
    LockEnabledEffect(bool),
    LockTimeoutEffect(u32),
    SuspendEnabledEffect(bool),
    SuspendTimeoutEffect(u32),
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

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Staged actions as the session sits idle. Each stage's timeout is measured from the last input — keep them ordered dim < lock < suspend. Any activity resets all stages.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // ── Dim ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Dim Screen",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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

                // ── Lock ────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Lock Screen",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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

                // ── Suspend ─────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Suspend",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
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
                    let value = config_manager().config().idle().$field().get();
                    sender_clone.input(IdleSettingsInput::$variant(value));
                });
            }};
        }
        push_effect!(dim_enabled, DimEnabledEffect);
        push_effect!(dim_timeout_minutes, DimTimeoutEffect);
        push_effect!(lock_enabled, LockEnabledEffect);
        push_effect!(lock_timeout_minutes, LockTimeoutEffect);
        push_effect!(suspend_enabled, SuspendEnabledEffect);
        push_effect!(suspend_timeout_minutes, SuspendTimeoutEffect);

        let model = IdleSettingsModel {
            dim_enabled: config_manager().config().idle().dim_enabled().get_untracked(),
            dim_timeout: config_manager()
                .config()
                .idle()
                .dim_timeout_minutes()
                .get_untracked(),
            lock_enabled: config_manager()
                .config()
                .idle()
                .lock_enabled()
                .get_untracked(),
            lock_timeout: config_manager()
                .config()
                .idle()
                .lock_timeout_minutes()
                .get_untracked(),
            suspend_enabled: config_manager()
                .config()
                .idle()
                .suspend_enabled()
                .get_untracked(),
            suspend_timeout: config_manager()
                .config()
                .idle()
                .suspend_timeout_minutes()
                .get_untracked(),
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
            IdleSettingsInput::DimEnabledChanged(v) => {
                config_manager().update_config(|c| c.idle.dim_enabled = v);
            }
            IdleSettingsInput::DimTimeoutChanged(v) => {
                config_manager().update_config(|c| c.idle.dim_timeout_minutes = v);
            }
            IdleSettingsInput::LockEnabledChanged(v) => {
                config_manager().update_config(|c| c.idle.lock_enabled = v);
            }
            IdleSettingsInput::LockTimeoutChanged(v) => {
                config_manager().update_config(|c| c.idle.lock_timeout_minutes = v);
            }
            IdleSettingsInput::SuspendEnabledChanged(v) => {
                config_manager().update_config(|c| c.idle.suspend_enabled = v);
            }
            IdleSettingsInput::SuspendTimeoutChanged(v) => {
                config_manager().update_config(|c| c.idle.suspend_timeout_minutes = v);
            }

            IdleSettingsInput::DimEnabledEffect(v) => self.dim_enabled = v,
            IdleSettingsInput::DimTimeoutEffect(v) => self.dim_timeout = v,
            IdleSettingsInput::LockEnabledEffect(v) => self.lock_enabled = v,
            IdleSettingsInput::LockTimeoutEffect(v) => self.lock_timeout = v,
            IdleSettingsInput::SuspendEnabledEffect(v) => self.suspend_enabled = v,
            IdleSettingsInput::SuspendTimeoutEffect(v) => self.suspend_timeout = v,
        }

        self.update_view(widgets, sender);
    }
}
