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

reactive_settings! {
    model: GameModeSettingsModel,
    input: GameModeSettingsInput,
    group: game_mode,
    // Variant bases are the page's short names, not the field names: `Animations`
    // over `disable_animations`, `Idle` over `inhibit_idle`, `Dnd` over `enable_dnd`.
    fields {
        Active => active: bool => active,
        Animations => disable_animations: bool => disable_animations,
        Blur => disable_blur: bool => disable_blur,
        Shadows => disable_shadows: bool => disable_shadows,
        Idle => inhibit_idle: bool => inhibit_idle,
        Dnd => enable_dnd: bool => enable_dnd,
        Tearing => allow_tearing: bool => allow_tearing,
    }
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
        let model = Self::from_config_store(&sender);

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        self.apply_reactive(message);
    }
}
