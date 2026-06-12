//! Display → Screen — physical-screen appearance that lives on mshell's side
//! (the shell paints the corner mask), as opposed to the compositor-owned
//! Twilight/gamma controls next to it. Today: the rounded-screen-corner mask
//! and its radius. Both persist to `general.*` on the shared `config_manager`
//! store and apply on the next mshell restart / monitor reconnect.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ScreenSettingsModel {
    show_screen_corners: bool,
    screen_corner_radius: i32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ScreenSettingsInput {
    ShowScreenCornersToggled(bool),
    ShowScreenCornersEffect(bool),
    ScreenCornerRadiusChanged(i32),
    ScreenCornerRadiusEffect(i32),
}

#[derive(Debug)]
pub(crate) enum ScreenSettingsOutput {}

pub(crate) struct ScreenSettingsInit {}

#[derive(Debug)]
pub(crate) enum ScreenSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for ScreenSettingsModel {
    type CommandOutput = ScreenSettingsCommandOutput;
    type Input = ScreenSettingsInput;
    type Output = ScreenSettingsOutput;
    type Init = ScreenSettingsInit;

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
                        set_icon_name: Some("video-display-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Screen",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Physical-screen appearance painted by the shell — rounded monitor corners and their radius.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Rounded corners",
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
                                set_label: "Rounded screen corners",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Mask each monitor's four corners so the screen reads as having rounded edges. Click-through. Off by default — the bar already paints its own rounded corners at the CSS frame-border-radius (24 px). Enable only when you also want the area *outside* the bar curved (e.g. bezel-less monitor), and set the radius below to match the frame border-radius so the two arcs line up.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(screen_corners_handler)]
                        set_active: model.show_screen_corners,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(ScreenSettingsInput::ShowScreenCornersToggled(v));
                            glib::Propagation::Proceed
                        } @screen_corners_handler,
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
                                set_label: "Corner radius (px)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Radius (px) of the black overlay mask that rounds the physical SCREEN corners — only when 'Rounded screen corners' above is on. This does NOT change widget, button, card or menu corners (those follow the fixed design scale, not a setting). Applies after restarting mshell (systemctl --user restart mshell) or reconnecting the monitor.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 64.0),
                        set_increments: (1.0, 4.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(corner_radius_handler)]
                        set_value: model.screen_corner_radius as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(ScreenSettingsInput::ScreenCornerRadiusChanged(s.value() as i32));
                        } @corner_radius_handler,
                    },
                },
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
            let v = config_manager()
                .config()
                .general()
                .show_screen_corners()
                .get();
            sender_clone.input(ScreenSettingsInput::ShowScreenCornersEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .screen_corner_radius()
                .get();
            sender_clone.input(ScreenSettingsInput::ScreenCornerRadiusEffect(v as i32));
        });

        let model = ScreenSettingsModel {
            show_screen_corners: config_manager()
                .config()
                .general()
                .show_screen_corners()
                .get_untracked(),
            screen_corner_radius: config_manager()
                .config()
                .general()
                .screen_corner_radius()
                .get_untracked() as i32,
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ScreenSettingsInput::ShowScreenCornersToggled(v) => {
                config_manager().update_config(|c| {
                    c.general.show_screen_corners = v;
                });
            }
            ScreenSettingsInput::ShowScreenCornersEffect(v) => {
                self.show_screen_corners = v;
            }
            ScreenSettingsInput::ScreenCornerRadiusChanged(r) => {
                let clamped = r.clamp(0, 64) as u32;
                config_manager().update_config(|c| {
                    c.general.screen_corner_radius = clamped;
                });
            }
            ScreenSettingsInput::ScreenCornerRadiusEffect(r) => {
                self.screen_corner_radius = r;
            }
        }
    }
}
