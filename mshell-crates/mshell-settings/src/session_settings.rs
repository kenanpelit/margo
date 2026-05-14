//! Session settings — per-action command overrides for the
//! power menu. Each entry is the command run for that action;
//! leaving it empty falls back to the built-in default
//! (`systemctl …` / the in-process session lock). Non-empty
//! commands run via `sh -c`.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, SessionStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct SessionSettingsModel {
    lock_command: String,
    logout_command: String,
    suspend_command: String,
    reboot_command: String,
    shutdown_command: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum SessionSettingsInput {
    LockChanged(String),
    LogoutChanged(String),
    SuspendChanged(String),
    RebootChanged(String),
    ShutdownChanged(String),

    LockEffect(String),
    LogoutEffect(String),
    SuspendEffect(String),
    RebootEffect(String),
    ShutdownEffect(String),
}

#[derive(Debug)]
pub(crate) enum SessionSettingsOutput {}

pub(crate) struct SessionSettingsInit {}

#[derive(Debug)]
pub(crate) enum SessionSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for SessionSettingsModel {
    type CommandOutput = SessionSettingsCommandOutput;
    type Input = SessionSettingsInput;
    type Output = SessionSettingsOutput;
    type Init = SessionSettingsInit;

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
                    set_label: "Commands run by the session menu and `mshellctl menu session …`. Leave a field empty to use the built-in default. Non-empty commands run via `sh -c`.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // ── Lock ────────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Lock",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = built-in session lock.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("built-in"),
                        #[watch]
                        #[block_signal(lock_handler)]
                        set_text: &model.lock_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::LockChanged(e.text().to_string()));
                        } @lock_handler,
                    },
                },

                // ── Logout ──────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Logout",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl --user exit.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl --user exit"),
                        #[watch]
                        #[block_signal(logout_handler)]
                        set_text: &model.logout_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::LogoutChanged(e.text().to_string()));
                        } @logout_handler,
                    },
                },

                // ── Suspend ─────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Suspend",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl suspend.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl suspend"),
                        #[watch]
                        #[block_signal(suspend_handler)]
                        set_text: &model.suspend_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::SuspendChanged(e.text().to_string()));
                        } @suspend_handler,
                    },
                },

                // ── Reboot ──────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Reboot",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl reboot. e.g. osc-safe-reboot",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl reboot"),
                        #[watch]
                        #[block_signal(reboot_handler)]
                        set_text: &model.reboot_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::RebootChanged(e.text().to_string()));
                        } @reboot_handler,
                    },
                },

                // ── Shutdown ────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Shutdown",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_label: "Empty = systemctl poweroff.",
                        },
                    },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_placeholder_text: Some("systemctl poweroff"),
                        #[watch]
                        #[block_signal(shutdown_handler)]
                        set_text: &model.shutdown_command,
                        connect_changed[sender] => move |e| {
                            sender.input(SessionSettingsInput::ShutdownChanged(
                                e.text().to_string(),
                            ));
                        } @shutdown_handler,
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
                    let value = config_manager().config().session().$field().get();
                    sender_clone.input(SessionSettingsInput::$variant(value));
                });
            }};
        }
        push_effect!(lock_command, LockEffect);
        push_effect!(logout_command, LogoutEffect);
        push_effect!(suspend_command, SuspendEffect);
        push_effect!(reboot_command, RebootEffect);
        push_effect!(shutdown_command, ShutdownEffect);

        // The `SessionStoreFields` accessors consume `self`, so the
        // `config().session()` chain has to be re-walked per field.
        let model = SessionSettingsModel {
            lock_command: config_manager()
                .config()
                .session()
                .lock_command()
                .get_untracked(),
            logout_command: config_manager()
                .config()
                .session()
                .logout_command()
                .get_untracked(),
            suspend_command: config_manager()
                .config()
                .session()
                .suspend_command()
                .get_untracked(),
            reboot_command: config_manager()
                .config()
                .session()
                .reboot_command()
                .get_untracked(),
            shutdown_command: config_manager()
                .config()
                .session()
                .shutdown_command()
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
            SessionSettingsInput::LockChanged(v) => {
                config_manager().update_config(|c| c.session.lock_command = v);
            }
            SessionSettingsInput::LogoutChanged(v) => {
                config_manager().update_config(|c| c.session.logout_command = v);
            }
            SessionSettingsInput::SuspendChanged(v) => {
                config_manager().update_config(|c| c.session.suspend_command = v);
            }
            SessionSettingsInput::RebootChanged(v) => {
                config_manager().update_config(|c| c.session.reboot_command = v);
            }
            SessionSettingsInput::ShutdownChanged(v) => {
                config_manager().update_config(|c| c.session.shutdown_command = v);
            }

            SessionSettingsInput::LockEffect(v) => self.lock_command = v,
            SessionSettingsInput::LogoutEffect(v) => self.logout_command = v,
            SessionSettingsInput::SuspendEffect(v) => self.suspend_command = v,
            SessionSettingsInput::RebootEffect(v) => self.reboot_command = v,
            SessionSettingsInput::ShutdownEffect(v) => self.shutdown_command = v,
        }

        self.update_view(widgets, sender);
    }
}
