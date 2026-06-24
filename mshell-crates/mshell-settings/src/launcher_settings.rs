//! Settings → Launcher page.
//!
//! Surfaces the launcher's cache/index state and gives the user
//! one-click controls to clear each store. The launcher itself
//! (and its providers) own the actual data — this page just
//! reaches into the public store helpers exposed by
//! `mshell-launcher` to read paths and remove files.
//!
//! Layout follows the Apple-style hero + section-heading pattern
//! the rest of Settings already uses (idle / theme / wallpaper).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, LauncherStoreFields, MenuStoreFields, MenusStoreFields, PassStoreFields,
};
use mshell_config::schema::position::Position;
use mshell_launcher::{frecency, history};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct LauncherSettingsModel {
    menu_position: Position,
    menu_min_width: i32,
    menu_max_height: i32,
    position_model: gtk::StringList,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum LauncherSettingsInput {
    /// Clear `~/.cache/margo/launcher_usage.json`. Next launcher
    /// open re-creates an empty store.
    ClearFrecency,
    /// Clear `~/.cache/margo/launcher_command_history.json`.
    ClearCommandHistory,
    /// Clear the in-memory clipboard history kept by
    /// `mshell_clipboard::clipboard_service()`. Effect is
    /// immediate — no file to remove.
    ClearClipboard,
    /// Set the GNU pass store directory (`config.pass.store_path`).
    /// Empty falls back to $PASSWORD_STORE_DIR / ~/.password-store.
    SetPassStorePath(String),
    PositionPicked(u32),
    MinWidthChanged(i32),
    MaxHeightChanged(i32),
    PositionEffect(Position),
    MinWidthEffect(i32),
    MaxHeightEffect(i32),
}

#[derive(Debug)]
pub(crate) enum LauncherSettingsOutput {}

pub(crate) struct LauncherSettingsInit {}

#[derive(Debug)]
pub(crate) enum LauncherSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for LauncherSettingsModel {
    type CommandOutput = LauncherSettingsCommandOutput;
    type Input = LauncherSettingsInput;
    type Output = LauncherSettingsOutput;
    type Init = LauncherSettingsInit;

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

                // Hero ─────────────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("system-search-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Launcher",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Provider-based app launcher — \
                                        Apps, Windows, Calculator, \
                                        Clipboard, Scripts (>start), \
                                        Sessions, Settings, Margo \
                                        actions, Shell commands.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // Panel layout ────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Panel layout",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Position",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Screen edge where the launcher panel opens.",
                                set_halign: gtk::Align::Start,
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
                            set_selected: model.menu_position.to_index(),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(LauncherSettingsInput::PositionPicked(dd.selected()));
                            } @position_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Minimum width",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Panel width in pixels.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (300.0, 2000.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(min_width_handler)]
                            set_value: model.menu_min_width as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(LauncherSettingsInput::MinWidthChanged(s.value() as i32));
                            } @min_width_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Height",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Visible panel height in pixels. 0 lets GTK choose.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 2000.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[watch]
                            #[block_signal(max_height_handler)]
                            set_value: model.menu_max_height as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(LauncherSettingsInput::MaxHeightChanged(s.value() as i32));
                            } @max_height_handler,
                        },
                    },
                },

                // Appearance ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Appearance",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Preview pane",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Show a detail pane beside the results for the selected item.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: config_manager().config().launcher().show_preview().get_untracked(),
                            connect_active_notify => move |sw| {
                                let v = sw.is_active();
                                config_manager().update_config(move |c| c.launcher.show_preview = v);
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Compact rows",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Tighter row spacing to fit more results on screen.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: config_manager().config().launcher().compact_rows().get_untracked(),
                            connect_active_notify => move |sw| {
                                let v = sw.is_active();
                                config_manager().update_config(move |c| c.launcher.compact_rows = v);
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "settings-row",
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Large app icons",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Lead app and window rows with a bigger icon.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: config_manager().config().launcher().large_app_icons().get_untracked(),
                            connect_active_notify => move |sw| {
                                let v = sw.is_active();
                                config_manager().update_config(move |c| c.launcher.large_app_icons = v);
                            },
                        },
                    },
                },

                // Cache ────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Cache",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Reset the launcher's persistent state. \
                                Frecency clears the usage counts that \
                                push frequently-launched apps to the \
                                top; history clears the >run MRU list; \
                                clipboard clears the running clipboard \
                                ring.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear frecency",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::ClearFrecency);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear command history",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::ClearCommandHistory);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear clipboard",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::ClearClipboard);
                        },
                    },
                },

                // GNU pass store ───────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Password store (pass)",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Directory the `pass` launcher provider scans (type `pass …` in the launcher). Blank = $PASSWORD_STORE_DIR, else ~/.password-store. Applies on the next launcher open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
                #[name = "pass_store_entry"]
                gtk::Entry {
                    set_placeholder_text: Some("e.g. ~/.pass   (blank = $PASSWORD_STORE_DIR)"),
                    set_hexpand: true,
                    connect_changed[sender] => move |e| {
                        sender.input(LauncherSettingsInput::SetPassStorePath(e.text().to_string()));
                    },
                },

                // Paths (debug) ────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Storage paths",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: &format!(
                        "Frecency: {}\nCommand history: {}",
                        frecency::store_path().display(),
                        history::store_path().display(),
                    ),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_selectable: true,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
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

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let p = config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .position()
                .get();
            sender_clone.input(LauncherSettingsInput::PositionEffect(p));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let w = config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .minimum_width()
                .get();
            sender_clone.input(LauncherSettingsInput::MinWidthEffect(w));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let h = config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .maximum_height()
                .get();
            sender_clone.input(LauncherSettingsInput::MaxHeightEffect(h));
        });

        let model = LauncherSettingsModel {
            menu_position: config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .position()
                .get_untracked(),
            menu_min_width: config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .minimum_width()
                .get_untracked(),
            menu_max_height: config_manager()
                .config()
                .menus()
                .app_launcher_menu()
                .maximum_height()
                .get_untracked(),
            position_model,
            _effects: effects,
        };

        let widgets = view_output!();

        widgets.pass_store_entry.set_text(
            &config_manager()
                .config()
                .pass()
                .store_path()
                .get_untracked(),
        );

        let _ = root;
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
            LauncherSettingsInput::ClearFrecency => {
                if let Err(err) = frecency::clear_disk() {
                    tracing::warn!(?err, "settings: clear frecency failed");
                } else {
                    mshell_launcher::notify::toast(
                        "Frecency cleared",
                        "Usage counts reset to zero.",
                    );
                }
            }
            LauncherSettingsInput::ClearCommandHistory => {
                if let Err(err) = history::clear_disk() {
                    tracing::warn!(?err, "settings: clear command history failed");
                } else {
                    mshell_launcher::notify::toast(
                        "Command history cleared",
                        ">run recent list emptied.",
                    );
                }
            }
            LauncherSettingsInput::ClearClipboard => {
                mshell_clipboard::clipboard_service().clear_history();
                mshell_launcher::notify::toast("Clipboard cleared", "All entries removed.");
            }
            LauncherSettingsInput::SetPassStorePath(path) => {
                let path = path.trim().to_string();
                config_manager().update_config(move |config| {
                    config.pass.store_path = path;
                });
            }
            LauncherSettingsInput::PositionPicked(idx) => {
                let p = Position::from_index(idx);
                if self.menu_position != p {
                    self.menu_position = p.clone();
                    config_manager().update_config(move |config| {
                        config.menus.app_launcher_menu.position = p;
                    });
                }
            }
            LauncherSettingsInput::MinWidthChanged(w) => {
                if self.menu_min_width != w {
                    self.menu_min_width = w;
                    config_manager().update_config(move |config| {
                        config.menus.app_launcher_menu.minimum_width = w;
                    });
                }
            }
            LauncherSettingsInput::MaxHeightChanged(h) => {
                if self.menu_max_height != h {
                    self.menu_max_height = h;
                    config_manager().update_config(move |config| {
                        config.menus.app_launcher_menu.maximum_height = h;
                    });
                }
            }
            LauncherSettingsInput::PositionEffect(p) => self.menu_position = p,
            LauncherSettingsInput::MinWidthEffect(w) => self.menu_min_width = w,
            LauncherSettingsInput::MaxHeightEffect(h) => self.menu_max_height = h,
        }
        self.update_view(widgets, sender);
    }
}
