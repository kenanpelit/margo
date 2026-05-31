//! Cross-cutting `Menus` settings page.
//!
//! Renders a top-of-page Menu Expansion section (left/right
//! always-expanded toggles) followed by one `MenuConfigPanel`
//! per menu listed in `MenuKind::all()`, then a small
//! Screenshare section at the bottom (Screenshare carries only
//! `position`, so it doesn't fit the MenuConfigPanel shape).
//!
//! All per-menu boilerplate — position / min_width / max_height
//! form fields + widget-list editor — lives in `MenuConfigPanel`
//! now. Adding a new menu = one entry in `MenuKind::all()` plus
//! the existing MenuKind dispatch arms. The old per-menu
//! ~250-line copy-paste blocks (this file used to be 4041
//! lines!) are gone.

use crate::menu_settings::menu_config_panel::{MenuConfigPanelInit, MenuConfigPanelModel};
use crate::widget_menu_settings::MenuKind;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, MenusStoreFields, ScreenshareMenuStoreFields, VerticalMenuExpansion,
};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

#[derive(Debug)]
pub(crate) struct MenuSettingsModel {
    panels: Vec<Controller<MenuConfigPanelModel>>,
    screenshare_position: Position,
    left_menu_expansion_type: VerticalMenuExpansion,
    right_menu_expansion_type: VerticalMenuExpansion,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MenuSettingsInput {
    ScreensharePositionChanged(Position),
    LeftMenuExpansionChanged(VerticalMenuExpansion),
    RightMenuExpansionChanged(VerticalMenuExpansion),
    ScreensharePositionEffect(Position),
    LeftMenuExpansionEffect(VerticalMenuExpansion),
    RightMenuExpansionEffect(VerticalMenuExpansion),
}

#[derive(Debug)]
pub(crate) enum MenuSettingsOutput {}

pub(crate) struct MenuSettingsInit {}

#[derive(Debug)]
pub(crate) enum MenuSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for MenuSettingsModel {
    type CommandOutput = MenuSettingsCommandOutput;
    type Input = MenuSettingsInput;
    type Output = MenuSettingsOutput;
    type Init = MenuSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            #[name = "page_box"]
            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                // ── Hero header ──────────────────────────────
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
                            set_label: "Menus",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-menu surface — expansion, geometry, contents. Each menu (clock, dashboard, notifications, etc.) gets its own page in the sub-sidebar.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Menu Expansion ───────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Menu Expansion",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Left Menu Expansion",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How left-anchored menus expand vertically — always full-height or only as tall as their contents.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&VerticalMenuExpansion::display_names())),
                        #[watch]
                        #[block_signal(left_exp_handler)]
                        set_selected: model.left_menu_expansion_type.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(MenuSettingsInput::LeftMenuExpansionChanged(
                                VerticalMenuExpansion::from_index(dd.selected())
                            ));
                        } @left_exp_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Right Menu Expansion",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How right-anchored menus expand vertically.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&VerticalMenuExpansion::display_names())),
                        #[watch]
                        #[block_signal(right_exp_handler)]
                        set_selected: model.right_menu_expansion_type.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(MenuSettingsInput::RightMenuExpansionChanged(
                                VerticalMenuExpansion::from_index(dd.selected())
                            ));
                        } @right_exp_handler,
                    },
                },

                gtk::Separator {},

                // Per-menu MenuConfigPanel widgets are appended
                // to `page_box` from `init` after `view_output!`
                // — relm4's declarative view! macro can't loop,
                // so the dynamic append happens immediately
                // below, before the Screenshare footer is
                // attached (which is also done in init).
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        // Reactive effects for the three top-of-page knobs.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .menus()
                .left_menu_expansion_type()
                .get();
            sender_clone.input(MenuSettingsInput::LeftMenuExpansionEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .menus()
                .right_menu_expansion_type()
                .get();
            sender_clone.input(MenuSettingsInput::RightMenuExpansionEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .menus()
                .screenshare_menu()
                .position()
                .get();
            sender_clone.input(MenuSettingsInput::ScreensharePositionEffect(value));
        });

        // One MenuConfigPanel per known menu kind.
        let panels: Vec<Controller<MenuConfigPanelModel>> = MenuKind::all()
            .iter()
            .map(|kind| {
                MenuConfigPanelModel::builder()
                    .launch(MenuConfigPanelInit { kind: *kind })
                    .detach()
            })
            .collect();

        let model = MenuSettingsModel {
            panels,
            screenshare_position: config_manager()
                .config()
                .menus()
                .screenshare_menu()
                .position()
                .get_untracked(),
            left_menu_expansion_type: config_manager()
                .config()
                .menus()
                .left_menu_expansion_type()
                .get_untracked(),
            right_menu_expansion_type: config_manager()
                .config()
                .menus()
                .right_menu_expansion_type()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

        // Append the panels to the page_box in order. The
        // Screenshare footer is appended last so it sits below
        // every per-menu panel.
        for panel in &model.panels {
            widgets.page_box.append(panel.widget());
        }

        // ── Screenshare footer ───────────────────────────────
        let screenshare_section = build_screenshare_section(&model, &sender);
        widgets.page_box.append(&screenshare_section);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            MenuSettingsInput::ScreensharePositionChanged(position) => {
                self.screenshare_position = position.clone();
                config_manager().update_config(|config| {
                    config.menus.screenshare_menu.position = position;
                });
            }
            MenuSettingsInput::LeftMenuExpansionChanged(expansion_type) => {
                self.left_menu_expansion_type = expansion_type.clone();
                config_manager().update_config(|config| {
                    config.menus.left_menu_expansion_type = expansion_type;
                });
            }
            MenuSettingsInput::RightMenuExpansionChanged(expansion_type) => {
                self.right_menu_expansion_type = expansion_type.clone();
                config_manager().update_config(|config| {
                    config.menus.right_menu_expansion_type = expansion_type;
                });
            }
            MenuSettingsInput::ScreensharePositionEffect(position) => {
                self.screenshare_position = position;
            }
            MenuSettingsInput::LeftMenuExpansionEffect(expansion_type) => {
                self.left_menu_expansion_type = expansion_type;
            }
            MenuSettingsInput::RightMenuExpansionEffect(expansion_type) => {
                self.right_menu_expansion_type = expansion_type;
            }
        }
    }
}

/// The Screenshare section is a single Position dropdown — too
/// small (and too special, no widget list / min_width /
/// max_height) to deserve a MenuConfigPanel instance. Built
/// inline here as a plain `gtk::Box` and appended to the page.
fn build_screenshare_section(
    model: &MenuSettingsModel,
    sender: &ComponentSender<MenuSettingsModel>,
) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();

    let title = gtk::Label::builder()
        .css_classes(["label-large-bold"])
        .label("Screen Share Menu")
        .halign(gtk::Align::Start)
        .build();
    section.append(&title);

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(20)
        .build();
    section.append(&row);

    let labels = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    row.append(&labels);

    let label_title = gtk::Label::builder()
        .css_classes(["label-medium-bold"])
        .halign(gtk::Align::Start)
        .label("Position")
        .hexpand(true)
        .build();
    labels.append(&label_title);

    let label_sub = gtk::Label::builder()
        .css_classes(["label-small"])
        .halign(gtk::Align::Start)
        .label("Where this menu should be positioned.")
        .hexpand(true)
        .xalign(0.0)
        .wrap(true)
        .natural_wrap_mode(gtk::NaturalWrapMode::None)
        .build();
    labels.append(&label_sub);

    let dd = gtk::DropDown::builder()
        .width_request(150)
        .valign(gtk::Align::Center)
        .model(&gtk::StringList::new(&Position::display_names()))
        .selected(model.screenshare_position.to_index())
        .build();
    let sender_clone = sender.clone();
    dd.connect_selected_notify(move |dd| {
        sender_clone.input(MenuSettingsInput::ScreensharePositionChanged(
            Position::from_index(dd.selected()),
        ));
    });
    row.append(&dd);

    section
}
