//! Settings → Widgets → System Updates.
//!
//! The system-update widget's own page: menu size/position on top,
//! then the behaviour knobs — how often to re-check (in hours) and the
//! per-source toggles (official repo / AUR / Flatpak). Writes flow to
//! `config.menus.system_update_menu` (size) and
//! `config.bars.widgets.system_update` (cadence + sources). The
//! check interval is stored in minutes; the spin here edits whole
//! hours (the widget only re-probes once per interval, persisting the
//! last check across restarts).

use mshell_config::config_manager::config_manager;
use mshell_config::schema::position::Position;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct SystemUpdateSettingsModel {
    position_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateSettingsInput {
    // Menu surface size / position (config.menus.system_update_menu).
    SetPosition(u32),
    SetMinWidth(i32),
    SetMaxHeight(i32),
    // Behaviour (config.bars.widgets.system_update).
    SetIntervalHours(i32),
    SetCheckRepo(bool),
    SetCheckAur(bool),
    SetCheckFlatpak(bool),
}

#[derive(Debug)]
pub(crate) enum SystemUpdateSettingsOutput {}

pub(crate) struct SystemUpdateSettingsInit {}

#[derive(Debug)]
pub(crate) enum SystemUpdateSettingsCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for SystemUpdateSettingsModel {
    type CommandOutput = SystemUpdateSettingsCommandOutput;
    type Input = SystemUpdateSettingsInput;
    type Output = SystemUpdateSettingsOutput;
    type Init = SystemUpdateSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                // Hero ─────────────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("software-update-available-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "System Updates",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Counts pending updates across the official \
                                        repos, AUR, and Flatpak. The check runs once \
                                        per interval and survives restarts — \
                                        right-click the pill to refresh immediately.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // Menu size / position ─────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Menu size & position",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Position", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "pos_dd"]
                    gtk::DropDown {
                        set_model: Some(&model.position_model),
                        connect_selected_notify[sender] => move |d| {
                            sender.input(SystemUpdateSettingsInput::SetPosition(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Width (px)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "width_spin"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(420.0, 280.0, 1200.0, 10.0, 50.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(SystemUpdateSettingsInput::SetMinWidth(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Max height (px, 0 = no cap)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "height_spin"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(600.0, 0.0, 2000.0, 20.0, 100.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(SystemUpdateSettingsInput::SetMaxHeight(s.value() as i32));
                        },
                    },
                },

                // Update checks ────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Update checks",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Check every (hours)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "interval_spin"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(3.0, 1.0, 48.0, 1.0, 6.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(SystemUpdateSettingsInput::SetIntervalHours(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Official repo updates", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "su_repo"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(SystemUpdateSettingsInput::SetCheckRepo(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "AUR updates (paru / yay)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "su_aur"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(SystemUpdateSettingsInput::SetCheckAur(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Flatpak updates", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "su_flatpak"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(SystemUpdateSettingsInput::SetCheckFlatpak(state));
                            glib::Propagation::Proceed
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
        let position_refs: Vec<&str> = Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let model = SystemUpdateSettingsModel { position_model };
        let widgets = view_output!();

        // Prime all controls from config.
        let cfg = config_manager().config().get_untracked();
        let m = &cfg.menus.system_update_menu;
        let pos_idx = Position::all()
            .iter()
            .position(|p| *p == m.position)
            .unwrap_or(0) as u32;
        widgets.pos_dd.set_selected(pos_idx);
        widgets.width_spin.set_value(m.minimum_width as f64);
        widgets.height_spin.set_value(m.maximum_height as f64);

        let su = &cfg.bars.widgets.system_update;
        // Minutes → whole hours (floor, min 1) for the spin.
        widgets
            .interval_spin
            .set_value((su.check_interval_minutes / 60).max(1) as f64);
        widgets.su_repo.set_active(su.check_repo);
        widgets.su_aur.set_active(su.check_aur);
        widgets.su_flatpak.set_active(su.check_flatpak);

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SystemUpdateSettingsInput::SetPosition(i) => {
                config_manager().update_config(|c| {
                    c.menus.system_update_menu.position = Position::from_index(i)
                });
            }
            SystemUpdateSettingsInput::SetMinWidth(v) => {
                config_manager()
                    .update_config(|c| c.menus.system_update_menu.minimum_width = v.max(1));
            }
            SystemUpdateSettingsInput::SetMaxHeight(v) => {
                config_manager()
                    .update_config(|c| c.menus.system_update_menu.maximum_height = v.max(0));
            }
            SystemUpdateSettingsInput::SetIntervalHours(h) => {
                let minutes = (h.max(1) as u32).saturating_mul(60);
                config_manager().update_config(|c| {
                    c.bars.widgets.system_update.check_interval_minutes = minutes
                });
            }
            SystemUpdateSettingsInput::SetCheckRepo(on) => {
                config_manager().update_config(|c| c.bars.widgets.system_update.check_repo = on);
            }
            SystemUpdateSettingsInput::SetCheckAur(on) => {
                config_manager().update_config(|c| c.bars.widgets.system_update.check_aur = on);
            }
            SystemUpdateSettingsInput::SetCheckFlatpak(on) => {
                config_manager().update_config(|c| c.bars.widgets.system_update.check_flatpak = on);
            }
        }
    }
}
