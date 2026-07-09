//! Settings → OSD — geometry + chrome for the on-screen-display capsules
//! (volume / brightness / mic / network pulse). Position and distance anchor
//! the layer-shell windows (applied when the OSDs are next created — a shell
//! restart, like the screen-corner overlays); width, corner radius and border
//! thickness drive the live `--osd-*` CSS vars, so they update instantly.
//! Border colour follows the matugen `--outline` role — the same one the
//! compositor paints idle window borders with, so the OSD ring matches the
//! window border — and isn't a setting (DESIGN.md: surfaces over hardcoded
//! colours).
//!
//! All five knobs persist to `osd.*` on the shared `config_manager` store.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, OsdStoreFields};
use mshell_config::schema::position::OsdPosition;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct OsdSettingsModel {
    width: i32,
    position: OsdPosition,
    distance: i32,
    radius: i32,
    border_width: i32,
    _effects: EffectScope,
}

reactive_settings! {
    model: OsdSettingsModel,
    input: OsdSettingsInput,
    group: osd,
    fields {
        Position => position: OsdPosition => position,
    }
    // The SpinButton ranges are defensive-clamped again on write, matching the
    // hand-rolled arms; the store -> model (`Effect`) path is never clamped.
    clamped {
        Width => width: i32 => width in 80 ..= 1200,
        Distance => distance: i32 => distance in 0 ..= 400,
        Radius => radius: i32 => radius in 0 ..= 200,
        BorderWidth => border_width: i32 => border_width in 0 ..= 20,
    }
}

#[derive(Debug)]
pub(crate) enum OsdSettingsOutput {}

pub(crate) struct OsdSettingsInit {}

#[derive(Debug)]
pub(crate) enum OsdSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for OsdSettingsModel {
    type CommandOutput = OsdSettingsCommandOutput;
    type Input = OsdSettingsInput;
    type Output = OsdSettingsOutput;
    type Init = OsdSettingsInit;

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
                        set_icon_name: Some("audio-volume-high-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "OSD",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The volume, brightness, mic and network pulse capsule — where it sits and how it looks.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Placement ────────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Placement",
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
                                set_label: "Which screen edge the capsule docks against. Applies after restarting mshell (systemctl --user restart mshell).",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_width_request: 150,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&OsdPosition::display_names())),
                            #[watch]
                            #[block_signal(position_handler)]
                            set_selected: model.position.to_index(),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(OsdSettingsInput::PositionChanged(
                                    OsdPosition::from_index(dd.selected())
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
                                set_label: "Margin from the docked edge. Ignored when Position is Center. Applies after an mshell restart.",
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
                                sender.input(OsdSettingsInput::DistanceChanged(s.value() as i32));
                            } @distance_handler,
                        },
                    },
                },

                // ── Size & shape ─────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Size & shape",
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
                                set_label: "Width (px)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Capsule width (its minimum — height follows the content). Updates live.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (80.0, 1200.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(width_handler)]
                            set_value: model.width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OsdSettingsInput::WidthChanged(s.value() as i32));
                            } @width_handler,
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
                                set_label: "Corner radius (px)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Rounding of the capsule corners. A large value gives a full pill. Updates live.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 200.0),
                            set_increments: (1.0, 8.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(radius_handler)]
                            set_value: model.radius as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OsdSettingsInput::RadiusChanged(s.value() as i32));
                            } @radius_handler,
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
                                set_label: "Border thickness (px)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Outline ring around the capsule, in the matugen outline colour. 0 hides it. Updates live.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 20.0),
                            set_increments: (1.0, 2.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(border_handler)]
                            set_value: model.border_width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OsdSettingsInput::BorderWidthChanged(s.value() as i32));
                            } @border_handler,
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
        let model = Self::from_config_store(&sender);

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        self.apply_reactive(message);
    }
}
