//! Settings → Widgets → Discord Voice — what a pill click does while
//! Discord isn't running, whether the pill hides outside a call, and which
//! program starts Discord. Composed with the generic per-menu geometry
//! page.

use crate::github_settings::row_text;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DiscordConfig};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct DiscordSettingsModel {
    cfg: DiscordConfig,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum DiscordSettingsInput {
    LaunchChanged(bool),
    HideChanged(bool),
    /// The program entry was activated or lost focus.
    BinaryCommitted(String),
    Effect(DiscordConfig),
}

#[derive(Debug)]
pub(crate) enum DiscordSettingsOutput {}

pub(crate) struct DiscordSettingsInit {}

#[relm4::component(pub)]
impl Component for DiscordSettingsModel {
    type CommandOutput = ();
    type Input = DiscordSettingsInput;
    type Output = DiscordSettingsOutput;
    type Init = DiscordSettingsInit;

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
                    add_css_class: "label-large-bold",
                    set_label: "Bar pill",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Launch Discord on click",
                            "While Discord isn't running, clicking the pill starts it instead of opening the menu (the menu always has a Launch Discord button).",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(launch_handler)]
                            set_active: model.cfg.launch_on_click,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(DiscordSettingsInput::LaunchChanged(v));
                                glib::Propagation::Proceed
                            } @launch_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Hide when not in a call",
                            "Hide the pill while you're not connected to a voice channel.",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(hide_handler)]
                            set_active: model.cfg.hide_when_disconnected,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(DiscordSettingsInput::HideChanged(v));
                                glib::Propagation::Proceed
                            } @hide_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Discord program",
                            "A name on $PATH or a full path, started without a shell. Blank = the first installed of discord, discord-canary, discord-ptb.",
                        ),
                        #[name = "binary_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("discord"),
                            connect_activate[sender] => move |e| {
                                sender.input(DiscordSettingsInput::BinaryCommitted(e.text().to_string()));
                            },
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
        let s = sender.clone();
        effects.push(move |_| {
            s.input(DiscordSettingsInput::Effect(
                config_manager().config().discord().get(),
            ));
        });
        let model = DiscordSettingsModel {
            cfg: config_manager().config().discord().get_untracked(),
            _effects: effects,
        };
        let widgets = view_output!();
        widgets.binary_entry.set_text(&model.cfg.binary);
        mshell_common::a11y::name(&widgets.binary_entry, "Discord program");
        let focus = gtk::EventControllerFocus::new();
        let (s, entry) = (sender.clone(), widgets.binary_entry.clone());
        focus.connect_leave(move |_| {
            s.input(DiscordSettingsInput::BinaryCommitted(
                entry.text().to_string(),
            ));
        });
        widgets.binary_entry.add_controller(focus);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            DiscordSettingsInput::LaunchChanged(v) => {
                if v != self.cfg.launch_on_click {
                    config_manager().update_config(|c| c.discord.launch_on_click = v);
                }
            }
            DiscordSettingsInput::HideChanged(v) => {
                if v != self.cfg.hide_when_disconnected {
                    config_manager().update_config(|c| c.discord.hide_when_disconnected = v);
                }
            }
            DiscordSettingsInput::BinaryCommitted(text) => {
                let text = text.trim().to_string();
                if text != self.cfg.binary {
                    config_manager().update_config(|c| c.discord.binary = text);
                }
            }
            DiscordSettingsInput::Effect(cfg) => {
                // Don't clobber text being edited with the same value.
                if widgets.binary_entry.text().trim() != cfg.binary {
                    widgets.binary_entry.set_text(&cfg.binary);
                }
                self.cfg = cfg;
            }
        }
        self.update_view(widgets, sender);
        let _ = root;
    }
}
