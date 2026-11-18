//! Settings → Widgets → Global Menu (`config.global_menu`). The registrar /
//! focused-menu task in `mshell-frame/src/global_menu` starts and stops live.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GlobalMenuConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct GlobalMenuSettingsModel {
    enabled: bool,
    show_app_name: bool,
    max_menus: u32,
    _effects: EffectScope,
}

reactive_settings! {
    model: GlobalMenuSettingsModel,
    input: GlobalMenuSettingsInput,
    group: global_menu,
    fields {
        Enabled => enabled: bool => enabled,
        ShowAppName => show_app_name: bool => show_app_name,
    }
    clamped {
        MaxMenus => max_menus: u32 => max_menus in 0 ..= 30,
    }
}

#[derive(Debug)]
pub(crate) enum GlobalMenuSettingsOutput {}

pub(crate) struct GlobalMenuSettingsInit {}

#[relm4::component(pub)]
impl Component for GlobalMenuSettingsModel {
    type CommandOutput = ();
    type Input = GlobalMenuSettingsInput;
    type Output = GlobalMenuSettingsOutput;
    type Init = GlobalMenuSettingsInit;

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
                        set_icon_name: Some("open-menu-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Global Menu",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The focused window's application menu in the bar (add the Global Menu widget to a top bar). Apps must export their menu over the AppMenu registrar: GTK 3 apps with appmenu-gtk-module (GTK_MODULES=appmenu-gtk-module) and Qt apps with the KDE platform theme (QT_QPA_PLATFORMTHEME=kde), both as XWayland clients. Native Wayland Qt and GTK 4 menus are not supported yet.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Global menu",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/global_menu/enabled",
                        #[template_child] title { set_label: "Serve application menus" },
                        #[template_child] desc { set_label: "Off by default. Owns com.canonical.AppMenu.Registrar on the session bus while on (or reads the one a KDE session already runs)." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(enabled_handler)]
                            set_active: model.enabled,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GlobalMenuSettingsInput::EnabledChanged(v));
                                glib::Propagation::Proceed
                            } @enabled_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/global_menu/show_app_name",
                        #[template_child] title { set_label: "Show the app name" },
                        #[template_child] desc { set_label: "The focused app's name in front of its menus." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_app_name_handler)]
                            set_active: model.show_app_name,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GlobalMenuSettingsInput::ShowAppNameChanged(v));
                                glib::Propagation::Proceed
                            } @show_app_name_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/global_menu/max_menus",
                        #[template_child] title { set_label: "Menus in the bar" },
                        #[template_child] desc { set_label: "At most this many top-level menus; the rest go under ». 0 shows all." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_digits: 0,
                            set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 30.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(max_menus_handler)]
                            set_value: model.max_menus as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(GlobalMenuSettingsInput::MaxMenusChanged(sb.value() as u32));
                            } @max_menus_handler,
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
