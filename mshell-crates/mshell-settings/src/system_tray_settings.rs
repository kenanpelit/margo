//! Settings → Widgets → System Tray.
//!
//! Tunables for the system tray bar widget (the StatusNotifierItem icon
//! strip). These live in the shell config (`config.system_tray`), so
//! reads/writes go through `config_manager` like the other shell pages;
//! the tray widget watches the same store and re-applies the reveal state
//! live.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, SystemTrayStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct SystemTraySettingsModel {
    default_expanded: bool,
}

#[derive(Debug)]
pub(crate) enum SystemTraySettingsInput {
    SetDefaultExpanded(bool),
}

#[derive(Debug)]
pub(crate) enum SystemTraySettingsOutput {}

pub(crate) struct SystemTraySettingsInit {}

#[derive(Debug)]
pub(crate) enum SystemTraySettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for SystemTraySettingsModel {
    type CommandOutput = SystemTraySettingsCommandOutput;
    type Input = SystemTraySettingsInput;
    type Output = SystemTraySettingsOutput;
    type Init = SystemTraySettingsInit;

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
                        set_icon_name: Some("view-list-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "System Tray",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The StatusNotifierItem icon strip. The tray button toggles the icons open or shut; the icons only appear while at least one app exposes a tray item.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Behaviour",
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
                            set_label: "Expanded by default",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "On = the tray comes up with its icons revealed. Off = the icons start collapsed behind the tray button (click to reveal). You can still toggle them at runtime either way.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "default_expanded_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(default_expanded_handler)]
                        set_active: model.default_expanded,
                        connect_active_notify[sender] => move |s| {
                            sender.input(SystemTraySettingsInput::SetDefaultExpanded(s.is_active()));
                        } @default_expanded_handler,
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
        let model = SystemTraySettingsModel {
            default_expanded: config_manager()
                .config()
                .system_tray()
                .default_expanded()
                .get_untracked(),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SystemTraySettingsInput::SetDefaultExpanded(v) => {
                self.default_expanded = v;
                config_manager().update_config(move |c| c.system_tray.default_expanded = v);
            }
        }
    }
}
