//! Per-widget menu settings — one component, parameterised by
//! `MenuKind`, that surfaces a given menu's `position` and
//! `minimum_width`. Used inside the `Widgets` sub-sidebar so each
//! menu gets its own focused settings page.
//!
//! The widgets-list editor (which BarWidget pills live inside a
//! menu) stays in the existing `menu_settings::Layout` page —
//! that's a cross-cutting view of every menu at once. These
//! per-menu pages are the "I just want to tweak THIS one"
//! shortcut.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Which menu this settings page targets. The enum carries
/// everything we need to read / write through `config_manager`
/// (descriptive label + reactive-field accessor dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    AppLauncher,
    Clipboard,
    Clock,
    Ndns,
    Nip,
    Nnotes,
    Npodman,
    Npower,
    QuickSettings,
    Screenshot,
    Nufw,
}

impl MenuKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::AppLauncher => "App Launcher",
            Self::Clipboard => "Clipboard",
            Self::Clock => "Clock",
            Self::Ndns => "DNS / VPN",
            Self::Nip => "Public IP",
            Self::Nnotes => "Notes Hub",
            Self::Npodman => "Podman",
            Self::Npower => "Power Profile",
            Self::QuickSettings => "Quick Settings",
            Self::Screenshot => "Screenshot",
            Self::Nufw => "UFW Firewall",
        }
    }

    /// Snapshot the menu's current position. `_untracked` so the
    /// initial model load doesn't subscribe; the `EffectScope`
    /// below subscribes explicitly.
    fn read_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get_untracked(),
            Self::Clipboard => m.clipboard_menu().position().get_untracked(),
            Self::Clock => m.clock_menu().position().get_untracked(),
            Self::Ndns => m.ndns_menu().position().get_untracked(),
            Self::Nip => m.nip_menu().position().get_untracked(),
            Self::Nnotes => m.nnotes_menu().position().get_untracked(),
            Self::Npodman => m.npodman_menu().position().get_untracked(),
            Self::Npower => m.npower_menu().position().get_untracked(),
            Self::QuickSettings => m.quick_settings_menu().position().get_untracked(),
            Self::Screenshot => m.screenshot_menu().position().get_untracked(),
            Self::Nufw => m.nufw_menu().position().get_untracked(),
        }
    }

    fn read_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get_untracked(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get_untracked(),
            Self::Clock => m.clock_menu().minimum_width().get_untracked(),
            Self::Ndns => m.ndns_menu().minimum_width().get_untracked(),
            Self::Nip => m.nip_menu().minimum_width().get_untracked(),
            Self::Nnotes => m.nnotes_menu().minimum_width().get_untracked(),
            Self::Npodman => m.npodman_menu().minimum_width().get_untracked(),
            Self::Npower => m.npower_menu().minimum_width().get_untracked(),
            Self::QuickSettings => m.quick_settings_menu().minimum_width().get_untracked(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get_untracked(),
            Self::Nufw => m.nufw_menu().minimum_width().get_untracked(),
        }
    }

    fn tracked_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get(),
            Self::Clipboard => m.clipboard_menu().position().get(),
            Self::Clock => m.clock_menu().position().get(),
            Self::Ndns => m.ndns_menu().position().get(),
            Self::Nip => m.nip_menu().position().get(),
            Self::Nnotes => m.nnotes_menu().position().get(),
            Self::Npodman => m.npodman_menu().position().get(),
            Self::Npower => m.npower_menu().position().get(),
            Self::QuickSettings => m.quick_settings_menu().position().get(),
            Self::Screenshot => m.screenshot_menu().position().get(),
            Self::Nufw => m.nufw_menu().position().get(),
        }
    }

    fn tracked_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get(),
            Self::Clock => m.clock_menu().minimum_width().get(),
            Self::Ndns => m.ndns_menu().minimum_width().get(),
            Self::Nip => m.nip_menu().minimum_width().get(),
            Self::Nnotes => m.nnotes_menu().minimum_width().get(),
            Self::Npodman => m.npodman_menu().minimum_width().get(),
            Self::Npower => m.npower_menu().minimum_width().get(),
            Self::QuickSettings => m.quick_settings_menu().minimum_width().get(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get(),
            Self::Nufw => m.nufw_menu().minimum_width().get(),
        }
    }

    fn write_position(self, p: Position) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.position = p,
            Self::Clipboard => c.menus.clipboard_menu.position = p,
            Self::Clock => c.menus.clock_menu.position = p,
            Self::Ndns => c.menus.ndns_menu.position = p,
            Self::Nip => c.menus.nip_menu.position = p,
            Self::Nnotes => c.menus.nnotes_menu.position = p,
            Self::Npodman => c.menus.npodman_menu.position = p,
            Self::Npower => c.menus.npower_menu.position = p,
            Self::QuickSettings => c.menus.quick_settings_menu.position = p,
            Self::Screenshot => c.menus.screenshot_menu.position = p,
            Self::Nufw => c.menus.nufw_menu.position = p,
        });
    }

    fn write_min_width(self, w: i32) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.minimum_width = w,
            Self::Clipboard => c.menus.clipboard_menu.minimum_width = w,
            Self::Clock => c.menus.clock_menu.minimum_width = w,
            Self::Ndns => c.menus.ndns_menu.minimum_width = w,
            Self::Nip => c.menus.nip_menu.minimum_width = w,
            Self::Nnotes => c.menus.nnotes_menu.minimum_width = w,
            Self::Npodman => c.menus.npodman_menu.minimum_width = w,
            Self::Npower => c.menus.npower_menu.minimum_width = w,
            Self::QuickSettings => c.menus.quick_settings_menu.minimum_width = w,
            Self::Screenshot => c.menus.screenshot_menu.minimum_width = w,
            Self::Nufw => c.menus.nufw_menu.minimum_width = w,
        });
    }
}

pub(crate) struct WidgetMenuSettingsModel {
    kind: MenuKind,
    position: Position,
    minimum_width: i32,
    position_model: gtk::StringList,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsInput {
    PositionPicked(u32),
    MinWidthChanged(i32),
    PositionEffect(Position),
    MinWidthEffect(i32),
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsOutput {}

pub(crate) struct WidgetMenuSettingsInit {
    pub(crate) kind: MenuKind,
}

#[relm4::component(pub(crate))]
impl Component for WidgetMenuSettingsModel {
    type CommandOutput = ();
    type Input = WidgetMenuSettingsInput;
    type Output = WidgetMenuSettingsOutput;
    type Init = WidgetMenuSettingsInit;

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
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Per-menu layout. The widgets that appear inside this menu are configured under Widgets → Layout.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Position",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Which screen edge this menu anchors to.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 180,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.position_model),
                        #[watch]
                        #[block_signal(position_handler)]
                        set_selected: model.position.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WidgetMenuSettingsInput::PositionPicked(dd.selected()));
                        } @position_handler,
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
                            set_label: "Minimum Width",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Width floor in pixels. The menu may grow past this for long content.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (200.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(min_width_handler)]
                        set_value: model.minimum_width as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MinWidthChanged(s.value() as i32));
                        } @min_width_handler,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let position_refs: Vec<&str> =
            Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let mut effects = EffectScope::new();

        let kind = params.kind;
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let p = kind.tracked_position();
            sender_clone.input(WidgetMenuSettingsInput::PositionEffect(p));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let w = kind.tracked_min_width();
            sender_clone.input(WidgetMenuSettingsInput::MinWidthEffect(w));
        });

        let model = WidgetMenuSettingsModel {
            kind,
            position: kind.read_position(),
            minimum_width: kind.read_min_width(),
            position_model,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WidgetMenuSettingsInput::PositionPicked(idx) => {
                let p = Position::from_index(idx);
                if self.position != p {
                    self.position = p.clone();
                    self.kind.write_position(p);
                }
            }
            WidgetMenuSettingsInput::MinWidthChanged(w) => {
                if self.minimum_width != w {
                    self.minimum_width = w;
                    self.kind.write_min_width(w);
                }
            }
            WidgetMenuSettingsInput::PositionEffect(p) => self.position = p,
            WidgetMenuSettingsInput::MinWidthEffect(w) => self.minimum_width = w,
        }
    }
}
