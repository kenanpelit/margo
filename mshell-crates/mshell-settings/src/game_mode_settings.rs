//! Settings → Game Mode. A live master toggle plus the per-effect switches
//! that pick *what* the mode affects. Shell-owned config (`config.game_mode`)
//! through the reactive store; flipping `active` is reconciled by the
//! reconcile effect in mshell-core (which writes the compositor effects
//! fragment, toggles DND, and holds the idle inhibitor). Page shape copied
//! from `toast_settings.rs` (DESIGN.md §8b).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GameModeStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct GameModeSettingsModel {
    active: bool,
    disable_animations: bool,
    disable_blur: bool,
    disable_shadows: bool,
    inhibit_idle: bool,
    enable_dnd: bool,
    allow_tearing: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum GameModeSettingsInput {
    ActiveChanged(bool),
    AnimationsChanged(bool),
    BlurChanged(bool),
    ShadowsChanged(bool),
    IdleChanged(bool),
    DndChanged(bool),
    TearingChanged(bool),

    ActiveEffect(bool),
    AnimationsEffect(bool),
    BlurEffect(bool),
    ShadowsEffect(bool),
    IdleEffect(bool),
    DndEffect(bool),
    TearingEffect(bool),
}

#[derive(Debug)]
pub(crate) enum GameModeSettingsOutput {}

pub(crate) struct GameModeSettingsInit {}

#[derive(Debug)]
pub(crate) enum GameModeSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for GameModeSettingsModel {
    type CommandOutput = GameModeSettingsCommandOutput;
    type Input = GameModeSettingsInput;
    type Output = GameModeSettingsOutput;
    type Init = GameModeSettingsInit;

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
                        set_icon_name: Some("input-gaming-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Game Mode",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "One toggle to drop compositor effects, silence notifications, and keep the session awake while gaming. Also `mshellctl gamemode on|off|toggle`.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
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
                                set_label: "Game Mode active",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Engage now. Applies live and survives a shell restart.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(active_handler)]
                            set_active: model.active,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::ActiveChanged(v));
                                glib::Propagation::Proceed
                            } @active_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "What it affects",
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
                                set_label: "Disable animations",
                                set_hexpand: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(animations_handler)]
                            set_active: model.disable_animations,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::AnimationsChanged(v));
                                glib::Propagation::Proceed
                            } @animations_handler,
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
                                set_label: "Disable blur",
                                set_hexpand: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(blur_handler)]
                            set_active: model.disable_blur,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::BlurChanged(v));
                                glib::Propagation::Proceed
                            } @blur_handler,
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
                                set_label: "Disable shadows",
                                set_hexpand: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(shadows_handler)]
                            set_active: model.disable_shadows,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::ShadowsChanged(v));
                                glib::Propagation::Proceed
                            } @shadows_handler,
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
                                set_label: "Hold the idle inhibitor — no dim, lock, or suspend.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(idle_handler)]
                            set_active: model.inhibit_idle,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::IdleChanged(v));
                                glib::Propagation::Proceed
                            } @idle_handler,
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
                                set_label: "Suppress notification pop-ups while active.",
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
                            set_active: model.enable_dnd,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::DndChanged(v));
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
                                set_label: "Allow screen tearing",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Lower latency for fullscreen games. Off by default.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(tearing_handler)]
                            set_active: model.allow_tearing,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GameModeSettingsInput::TearingChanged(v));
                                glib::Propagation::Proceed
                            } @tearing_handler,
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
                    let value = config_manager().config().game_mode().$field().get();
                    sender_clone.input(GameModeSettingsInput::$variant(value));
                });
            }};
        }
        push_effect!(active, ActiveEffect);
        push_effect!(disable_animations, AnimationsEffect);
        push_effect!(disable_blur, BlurEffect);
        push_effect!(disable_shadows, ShadowsEffect);
        push_effect!(inhibit_idle, IdleEffect);
        push_effect!(enable_dnd, DndEffect);
        push_effect!(allow_tearing, TearingEffect);

        let model = GameModeSettingsModel {
            active: config_manager()
                .config()
                .game_mode()
                .active()
                .get_untracked(),
            disable_animations: config_manager()
                .config()
                .game_mode()
                .disable_animations()
                .get_untracked(),
            disable_blur: config_manager()
                .config()
                .game_mode()
                .disable_blur()
                .get_untracked(),
            disable_shadows: config_manager()
                .config()
                .game_mode()
                .disable_shadows()
                .get_untracked(),
            inhibit_idle: config_manager()
                .config()
                .game_mode()
                .inhibit_idle()
                .get_untracked(),
            enable_dnd: config_manager()
                .config()
                .game_mode()
                .enable_dnd()
                .get_untracked(),
            allow_tearing: config_manager()
                .config()
                .game_mode()
                .allow_tearing()
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
            GameModeSettingsInput::ActiveChanged(v) => {
                config_manager().update_config(|c| c.game_mode.active = v);
            }
            GameModeSettingsInput::AnimationsChanged(v) => {
                config_manager().update_config(|c| c.game_mode.disable_animations = v);
            }
            GameModeSettingsInput::BlurChanged(v) => {
                config_manager().update_config(|c| c.game_mode.disable_blur = v);
            }
            GameModeSettingsInput::ShadowsChanged(v) => {
                config_manager().update_config(|c| c.game_mode.disable_shadows = v);
            }
            GameModeSettingsInput::IdleChanged(v) => {
                config_manager().update_config(|c| c.game_mode.inhibit_idle = v);
            }
            GameModeSettingsInput::DndChanged(v) => {
                config_manager().update_config(|c| c.game_mode.enable_dnd = v);
            }
            GameModeSettingsInput::TearingChanged(v) => {
                config_manager().update_config(|c| c.game_mode.allow_tearing = v);
            }

            GameModeSettingsInput::ActiveEffect(v) => self.active = v,
            GameModeSettingsInput::AnimationsEffect(v) => self.disable_animations = v,
            GameModeSettingsInput::BlurEffect(v) => self.disable_blur = v,
            GameModeSettingsInput::ShadowsEffect(v) => self.disable_shadows = v,
            GameModeSettingsInput::IdleEffect(v) => self.inhibit_idle = v,
            GameModeSettingsInput::DndEffect(v) => self.enable_dnd = v,
            GameModeSettingsInput::TearingEffect(v) => self.allow_tearing = v,
        }

        self.update_view(widgets, sender);
    }
}
