//! Settings → Widgets → Margo Dock.
//!
//! Tunables for the dock (the pinned/running app strip). These live in
//! the shell config (`config.dock`), so reads/writes go through
//! `config_manager` like the other shell pages; the dock widget watches
//! the same store and re-applies live (icon size, tooltips, show-running).

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DockStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct DockSettingsModel {
    icon_size: i32,
    show_tooltips: bool,
    show_running: bool,
}

#[derive(Debug)]
pub(crate) enum DockSettingsInput {
    SetIconSize(i32),
    SetShowTooltips(bool),
    SetShowRunning(bool),
}

#[derive(Debug)]
pub(crate) enum DockSettingsOutput {}

pub(crate) struct DockSettingsInit {}

#[derive(Debug)]
pub(crate) enum DockSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for DockSettingsModel {
    type CommandOutput = DockSettingsCommandOutput;
    type Input = DockSettingsInput;
    type Output = DockSettingsOutput;
    type Init = DockSettingsInit;

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
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Margo Dock",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The pinned / running app strip. Click an icon to jump to that app's tag; hover to see its windows.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Appearance",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Icon size",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "App-icon pixel size in the dock.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "icon_size_spin"]
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (16.0, 96.0),
                        set_increments: (2.0, 8.0),
                        set_digits: 0,
                        #[block_signal(icon_size_handler)]
                        set_value: model.icon_size as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(DockSettingsInput::SetIconSize(s.value() as i32));
                        } @icon_size_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Behaviour",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Show running apps",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Include running apps that aren't pinned. Off = a pinned-only launcher dock.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "show_running_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(show_running_handler)]
                        set_active: model.show_running,
                        connect_active_notify[sender] => move |s| {
                            sender.input(DockSettingsInput::SetShowRunning(s.is_active()));
                        } @show_running_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Window tooltips",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hover an icon to list the app's open window titles — handy when one icon hides several windows.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "show_tooltips_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(show_tooltips_handler)]
                        set_active: model.show_tooltips,
                        connect_active_notify[sender] => move |s| {
                            sender.input(DockSettingsInput::SetShowTooltips(s.is_active()));
                        } @show_tooltips_handler,
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
        let model = DockSettingsModel {
            icon_size: config_manager()
                .config()
                .dock()
                .icon_size()
                .get_untracked() as i32,
            show_tooltips: config_manager()
                .config()
                .dock()
                .show_tooltips()
                .get_untracked(),
            show_running: config_manager()
                .config()
                .dock()
                .show_running()
                .get_untracked(),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DockSettingsInput::SetIconSize(v) => {
                let v = v.clamp(8, 256);
                self.icon_size = v;
                config_manager().update_config(move |c| c.dock.icon_size = v as u32);
            }
            DockSettingsInput::SetShowTooltips(v) => {
                self.show_tooltips = v;
                config_manager().update_config(move |c| c.dock.show_tooltips = v);
            }
            DockSettingsInput::SetShowRunning(v) => {
                self.show_running = v;
                config_manager().update_config(move |c| c.dock.show_running = v);
            }
        }
    }
}
