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
    ConfigStoreFields, FileSearch, FileSearchScope, LauncherStoreFields, MenuStoreFields,
    MenusStoreFields, PassAutotypeSeparator, PassStoreFields,
};
use mshell_config::schema::position::Position;
use mshell_launcher::providers::file_search::scope::{FolderError, validate_new_folder};
use mshell_launcher::providers::websearch::parse_sites;
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
    /// Mirror of `config.launcher.web_sites` (`Name|url` lines).
    web_sites: Vec<String>,
    /// Mirror of `config.launcher.file_search`.
    file_search: FileSearch,
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
    /// Set `config.launcher.nvim_projects_dir` (blank = sessions only).
    SetNvimProjectsDir(String),
    /// Append a `Name|url` favourite (validated first).
    WebSiteAdd(String),
    /// Remove the favourite at this index.
    WebSiteRemove(usize),
    /// Config changed externally (hand-edited YAML).
    WebSitesEffect(Vec<String>),
    /// File search scope dropdown (Home only / Home + folders / Whole system).
    FsScopePicked(u32),
    FsUsbToggled(bool),
    /// Validate (off the GTK thread) and append an extra search folder.
    FsFolderAdd(String),
    /// Pick an extra search folder with the folder chooser.
    FsFolderBrowse,
    FsFolderRemove(usize),
    /// `launcher.file_search` changed (here, from the launcher, or YAML).
    FsEffect(FileSearch),
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
pub(crate) enum LauncherSettingsCommandOutput {
    /// An extra search folder after validation (stored form, or why not).
    FsFolderChecked(Result<String, FolderError>),
}

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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/menus/app_launcher_menu/position",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/menus/app_launcher_menu/minimum_width",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/menus/app_launcher_menu/maximum_height",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/show_preview",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/compact_rows",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/large_app_icons",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
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

                // Favourite websites ───────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Favourite websites",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Sites listed by the launcher's `web` prefix, most recently opened first. Add them as Name|https://url (a bare URL works too).",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/web_site_favicons",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Site icons",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Fetch the favicon of each site and new web app once (the domain is sent to Google's favicon service) and cache it.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: config_manager().config().launcher().web_site_favicons().get_untracked(),
                            connect_active_notify => move |sw| {
                                let v = sw.is_active();
                                config_manager().update_config(move |c| c.launcher.web_site_favicons = v);
                            },
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_widget_name: "setting:/launcher/web_sites",

                    #[name = "web_site_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("Name|https://example.com"),
                    },

                    #[name = "web_site_add"]
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_label: "Add",
                    },
                },

                #[name = "web_sites_list"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },

                // File search (`fs`) ────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "File search",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Where the launcher's `fs` prefix looks for files and folders. Each choice keeps its own index, so switching back is instant. An empty `fs` query offers the same switches and Rebuild search index.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/file_search/scope",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Search in",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Home only, your home plus the folders below, or the whole system (without /proc, /sys, /dev, /run, /tmp, network, FUSE and snapshot mounts).",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_width_request: 180,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&["Home only", "Home + folders", "Whole system"])),
                            #[watch]
                            #[block_signal(fs_scope_handler)]
                            set_selected: scope_index(model.file_search.scope),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(LauncherSettingsInput::FsScopePicked(dd.selected()));
                            } @fs_scope_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/launcher/file_search/include_usb",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Include USB disks",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Also search mounted USB and removable disks. They are walked only when you run Rebuild search index from `fs` — never on plug-in; until then the launcher says the index is out of date.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(fs_usb_handler)]
                            set_active: model.file_search.include_usb,
                            connect_active_notify[sender] => move |sw| {
                                sender.input(LauncherSettingsInput::FsUsbToggled(sw.is_active()));
                            } @fs_usb_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Extra folders, searched in Home + folders mode — a second disk or a hidden folder such as ~/.config. The folder must exist; ~ works. In the launcher, `fs +<path>` adds one too.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_margin_top: 4,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_widget_name: "setting:/launcher/file_search/folders",

                    #[name = "fs_folder_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("e.g. /media/data or ~/.config/app"),
                        connect_activate[sender] => move |e| {
                            let text = e.text().trim().to_string();
                            if !text.is_empty() {
                                sender.input(LauncherSettingsInput::FsFolderAdd(text));
                            }
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_valign: gtk::Align::Center,
                        set_label: "Browse…",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::FsFolderBrowse);
                        },
                    },
                    #[name = "fs_folder_add"]
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_label: "Add",
                    },
                },

                #[name = "fs_folders_list"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
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

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "In the launcher, `pass <entry>:` lists an entry's fields plus Autotype login (types username, separator, password, OTP) and Edit entry (`pass edit` in your terminal).",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_margin_top: 4,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/pass/clip_timeout_secs",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Clear clipboard after",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Seconds a copied password or field stays on the clipboard (if nothing else was copied since).",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (5.0, 600.0),
                            set_increments: (5.0, 30.0),
                            set_digits: 0,
                            set_value: config_manager().config().pass().clip_timeout_secs().get_untracked() as f64,
                            connect_value_changed => move |s| {
                                let v = s.value() as u32;
                                config_manager().update_config(move |c| c.pass.clip_timeout_secs = v);
                            },
                        },
                    },
                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/pass/type_delay_ms",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Typing delay",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Milliseconds to wait after the launcher closes before typing, so focus is back in your field.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 5000.0),
                            set_increments: (50.0, 250.0),
                            set_digits: 0,
                            set_value: config_manager().config().pass().type_delay_ms().get_untracked() as f64,
                            connect_value_changed => move |s| {
                                let v = s.value() as u32;
                                config_manager().update_config(move |c| c.pass.type_delay_ms = v);
                            },
                        },
                    },
                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/pass/key_delay_ms",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Keystroke delay",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Milliseconds between typed keys. Raise it if an app drops characters; 0 types as fast as possible.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 500.0),
                            set_increments: (5.0, 25.0),
                            set_digits: 0,
                            set_value: config_manager().config().pass().key_delay_ms().get_untracked() as f64,
                            connect_value_changed => move |s| {
                                let v = s.value() as u32;
                                config_manager().update_config(move |c| c.pass.key_delay_ms = v);
                            },
                        },
                    },
                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/pass/autotype_separator",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Autotype separator",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Key pressed between the username and the password. Enter suits two-step logins that ask for the password on a second page.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::DropDown {
                            set_width_request: 140,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&["Tab", "Enter"])),
                            set_selected: match config_manager().config().pass().autotype_separator().get_untracked() {
                                PassAutotypeSeparator::Tab => 0,
                                PassAutotypeSeparator::Enter => 1,
                            },
                            connect_selected_notify => move |dd| {
                                let v = if dd.selected() == 1 {
                                    PassAutotypeSeparator::Enter
                                } else {
                                    PassAutotypeSeparator::Tab
                                };
                                config_manager().update_config(move |c| c.pass.autotype_separator = v);
                            },
                        },
                    },
                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/pass/autotype_submit",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Autotype submits",
                                set_halign: gtk::Align::Start,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Press Enter after the last value (the password, or the OTP code when the entry has one).",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: config_manager().config().pass().autotype_submit().get_untracked(),
                            connect_active_notify => move |sw| {
                                let v = sw.is_active();
                                config_manager().update_config(move |c| c.pass.autotype_submit = v);
                            },
                        },
                    },
                },

                // Neovim projects ──────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Neovim projects",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "The `nv` launcher prefix lists your saved Neovim sessions (~/.local/state/nvim/sessions). Set a folder to also list each of its sub-directories as a project. Blank = sessions only. Applies on the next launcher open.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
                #[name = "nvim_projects_entry"]
                gtk::Entry {
                    set_widget_name: "setting:/launcher/nvim_projects_dir",
                    set_placeholder_text: Some("e.g. ~/Projects   (blank = sessions only)"),
                    set_hexpand: true,
                    connect_changed[sender] => move |e| {
                        sender.input(LauncherSettingsInput::SetNvimProjectsDir(e.text().to_string()));
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
            let list = config_manager().config().launcher().web_sites().get();
            sender_clone.input(LauncherSettingsInput::WebSitesEffect(list));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let fs = config_manager().config().launcher().file_search().get();
            sender_clone.input(LauncherSettingsInput::FsEffect(fs));
        });

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
            web_sites: config_manager()
                .config()
                .launcher()
                .web_sites()
                .get_untracked(),
            file_search: config_manager()
                .config()
                .launcher()
                .file_search()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

        // Favourite websites: add entry + button, initial rows.
        {
            let submit = |entry: &gtk::Entry, sender: &ComponentSender<LauncherSettingsModel>| {
                let line = entry.text().trim().to_string();
                if !line.is_empty() {
                    sender.input(LauncherSettingsInput::WebSiteAdd(line));
                    entry.set_text("");
                }
            };
            let entry = widgets.web_site_entry.clone();
            let s = sender.clone();
            widgets
                .web_site_add
                .connect_clicked(move |_| submit(&entry, &s));
            let s = sender.clone();
            widgets
                .web_site_entry
                .connect_activate(move |e| submit(e, &s));
        }
        rebuild_web_site_rows(&widgets.web_sites_list, &model.web_sites, &sender);

        // Extra search folders: Add button (Enter in the entry is wired in
        // the view), initial rows.
        {
            let entry = widgets.fs_folder_entry.clone();
            let s = sender.clone();
            widgets.fs_folder_add.connect_clicked(move |_| {
                let text = entry.text().trim().to_string();
                if !text.is_empty() {
                    s.input(LauncherSettingsInput::FsFolderAdd(text));
                }
            });
        }
        rebuild_folder_rows(
            &widgets.fs_folders_list,
            &model.file_search.folders,
            &sender,
        );

        widgets.pass_store_entry.set_text(
            &config_manager()
                .config()
                .pass()
                .store_path()
                .get_untracked(),
        );

        widgets.nvim_projects_entry.set_text(
            &config_manager()
                .config()
                .launcher()
                .nvim_projects_dir()
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
            LauncherSettingsInput::SetNvimProjectsDir(path) => {
                let path = path.trim().to_string();
                config_manager().update_config(move |c| c.launcher.nvim_projects_dir = path);
            }
            LauncherSettingsInput::WebSiteAdd(line) => {
                if parse_sites(std::slice::from_ref(&line)).is_empty() {
                    mshell_launcher::notify::toast(
                        "Not a website",
                        format!("“{line}” needs an http(s) URL, e.g. Name|https://example.com"),
                    );
                } else if !self.web_sites.contains(&line) {
                    self.web_sites.push(line);
                    let list = self.web_sites.clone();
                    config_manager().update_config(move |c| c.launcher.web_sites = list);
                    rebuild_web_site_rows(&widgets.web_sites_list, &self.web_sites, &sender);
                }
            }
            LauncherSettingsInput::WebSiteRemove(idx) => {
                if idx < self.web_sites.len() {
                    self.web_sites.remove(idx);
                    let list = self.web_sites.clone();
                    config_manager().update_config(move |c| c.launcher.web_sites = list);
                    rebuild_web_site_rows(&widgets.web_sites_list, &self.web_sites, &sender);
                }
            }
            LauncherSettingsInput::WebSitesEffect(list) => {
                if list != self.web_sites {
                    self.web_sites = list;
                    rebuild_web_site_rows(&widgets.web_sites_list, &self.web_sites, &sender);
                }
            }
            LauncherSettingsInput::FsScopePicked(idx) => {
                let scope = scope_from_index(idx);
                if self.file_search.scope != scope {
                    self.file_search.scope = scope;
                    config_manager().update_config(move |c| c.launcher.file_search.scope = scope);
                }
            }
            LauncherSettingsInput::FsUsbToggled(on) => {
                if self.file_search.include_usb != on {
                    self.file_search.include_usb = on;
                    config_manager()
                        .update_config(move |c| c.launcher.file_search.include_usb = on);
                }
            }
            LauncherSettingsInput::FsFolderAdd(raw) => {
                // Stat off the GTK thread: the path may be on a slow or
                // network mount.
                let existing = self.file_search.folders.clone();
                let home = margo_fs::xdg::home()
                    .map(|h| h.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "/".into());
                sender.oneshot_command(async move {
                    let checked = tokio::task::spawn_blocking(move || {
                        validate_new_folder(&raw, &home, &existing, |p| {
                            std::fs::metadata(p).ok().map(|m| m.is_dir())
                        })
                    })
                    .await
                    .unwrap_or(Err(FolderError::Empty));
                    LauncherSettingsCommandOutput::FsFolderChecked(checked)
                });
            }
            LauncherSettingsInput::FsFolderBrowse => {
                let dialog = gtk::FileDialog::builder()
                    .title("Add a search folder")
                    .modal(true)
                    .build();
                let s = sender.clone();
                dialog.select_folder(
                    gtk::Window::NONE,
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result
                            && let Some(path) = file.path()
                        {
                            s.input(LauncherSettingsInput::FsFolderAdd(
                                path.to_string_lossy().into_owned(),
                            ));
                        }
                    },
                );
            }
            LauncherSettingsInput::FsFolderRemove(idx) => {
                if idx < self.file_search.folders.len() {
                    self.file_search.folders.remove(idx);
                    let list = self.file_search.folders.clone();
                    config_manager().update_config(move |c| c.launcher.file_search.folders = list);
                    rebuild_folder_rows(
                        &widgets.fs_folders_list,
                        &self.file_search.folders,
                        &sender,
                    );
                }
            }
            LauncherSettingsInput::FsEffect(fs) => {
                if fs.folders != self.file_search.folders {
                    rebuild_folder_rows(&widgets.fs_folders_list, &fs.folders, &sender);
                }
                self.file_search = fs;
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

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            LauncherSettingsCommandOutput::FsFolderChecked(Ok(stored)) => {
                // Re-checked against the list as it is now (it may have
                // changed while the stat ran).
                if !self.file_search.folders.contains(&stored) {
                    self.file_search.folders.push(stored);
                    let list = self.file_search.folders.clone();
                    config_manager().update_config(move |c| c.launcher.file_search.folders = list);
                    rebuild_folder_rows(
                        &widgets.fs_folders_list,
                        &self.file_search.folders,
                        &sender,
                    );
                }
                widgets.fs_folder_entry.set_text("");
            }
            LauncherSettingsCommandOutput::FsFolderChecked(Err(err)) => {
                mshell_launcher::notify::toast("Can't add search folder", err.to_string());
            }
        }
        self.update_view(widgets, sender);
    }
}

fn scope_index(scope: FileSearchScope) -> u32 {
    match scope {
        FileSearchScope::Home => 0,
        FileSearchScope::HomeAndFolders => 1,
        FileSearchScope::System => 2,
    }
}

fn scope_from_index(idx: u32) -> FileSearchScope {
    match idx {
        1 => FileSearchScope::HomeAndFolders,
        2 => FileSearchScope::System,
        _ => FileSearchScope::Home,
    }
}

/// Repaint the extra search folder rows: one action-row per folder with
/// a trailing remove button (like the favourite websites).
fn rebuild_folder_rows(
    list: &gtk::Box,
    folders: &[String],
    sender: &ComponentSender<LauncherSettingsModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    list.set_visible(!folders.is_empty());
    for (idx, folder) in folders.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("action-row");

        let icon = gtk::Image::from_icon_name("folder-symbolic");
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        let name = gtk::Label::new(Some(folder));
        name.add_css_class("label-medium-bold");
        name.set_halign(gtk::Align::Start);
        name.set_hexpand(true);
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        row.append(&name);

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        mshell_common::a11y::name(&remove, &format!("Remove {folder}"));
        let sender = sender.clone();
        remove.connect_clicked(move |_| {
            sender.input(LauncherSettingsInput::FsFolderRemove(idx));
        });
        row.append(&remove);

        list.append(&row);
    }
}

/// Repaint the favourite-website rows: one action-row per entry (name +
/// URL) with a trailing remove button. Lines that don't parse are still
/// listed (by their raw text) so they can be removed.
fn rebuild_web_site_rows(
    list: &gtk::Box,
    lines: &[String],
    sender: &ComponentSender<LauncherSettingsModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    list.set_visible(!lines.is_empty());
    for (idx, line) in lines.iter().enumerate() {
        let (title, subtitle) = match parse_sites(std::slice::from_ref(line)).into_iter().next() {
            Some(site) => (site.name, site.url),
            None => (line.clone(), "Invalid — not an http(s) URL".to_string()),
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("action-row");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let name = gtk::Label::new(Some(&title));
        name.add_css_class("label-medium-bold");
        name.set_halign(gtk::Align::Start);
        name.set_xalign(0.0);
        let url = gtk::Label::new(Some(&subtitle));
        url.add_css_class("label-small");
        url.set_halign(gtk::Align::Start);
        url.set_xalign(0.0);
        url.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.append(&name);
        text.append(&url);
        row.append(&text);

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        mshell_common::a11y::name(&remove, &format!("Remove {title}"));
        let sender = sender.clone();
        remove.connect_clicked(move |_| {
            sender.input(LauncherSettingsInput::WebSiteRemove(idx));
        });
        row.append(&remove);

        list.append(&row);
    }
}
