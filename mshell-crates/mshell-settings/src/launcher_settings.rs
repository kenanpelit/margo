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

use mshell_launcher::providers::ScriptsProvider;
use mshell_launcher::{frecency, history};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct LauncherSettingsModel {
    /// Snapshot of the currently-indexed `>start` scripts. Built
    /// once on init via a throwaway `ScriptsProvider` (the real
    /// one lives inside the running launcher widget — we just
    /// re-do the PATH scan here, which is cheap).
    indexed_scripts: Vec<String>,
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
    /// Re-scan PATH to refresh the indexed-scripts display.
    RefreshScripts,
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

                // Scripts ──────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Scripts",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_label: &format!(
                        "Type `>start` in the launcher to run any \
                         executable on $PATH whose name starts with \
                         `{prefix}`. Indexed: {count} script(s).",
                        prefix = ScriptsProvider::DEFAULT_PREFIX,
                        count = model.indexed_scripts.len(),
                    ),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[name = "scripts_scroll"]
                gtk::ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_height_request: 180,
                    set_hexpand: true,

                    #[name = "scripts_box"]
                    gtk::Box {
                        add_css_class: "settings-boxed-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Refresh",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::RefreshScripts);
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
                                top; history clears the >cmd MRU list; \
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
        let model = LauncherSettingsModel {
            indexed_scripts: rebuild_indexed_scripts(),
        };

        let widgets = view_output!();

        rebuild_scripts_box(&widgets.scripts_box, &model.indexed_scripts);

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
                        ">cmd recent list emptied.",
                    );
                }
            }
            LauncherSettingsInput::ClearClipboard => {
                mshell_clipboard::clipboard_service().clear_history();
                mshell_launcher::notify::toast(
                    "Clipboard cleared",
                    "All entries removed.",
                );
            }
            LauncherSettingsInput::RefreshScripts => {
                self.indexed_scripts = rebuild_indexed_scripts();
                rebuild_scripts_box(&widgets.scripts_box, &self.indexed_scripts);
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Build a fresh `ScriptsProvider`, take its index snapshot,
/// drop the provider. Cheap (~tens of milliseconds for ~500
/// PATH entries) — running it on every refresh button click
/// keeps the displayed list authoritative without holding a
/// long-lived provider instance on the Settings page.
fn rebuild_indexed_scripts() -> Vec<String> {
    let provider = ScriptsProvider::new();
    provider.indexed_names()
}

/// Replace every child in `scripts_box` with one row per script.
/// Cleaner than mounting a full DynamicBox here — the list is
/// short, updates are rare, and a plain GtkBox + labels keeps the
/// Settings page free of factory boilerplate.
fn rebuild_scripts_box(scripts_box: &gtk::Box, scripts: &[String]) {
    // Remove existing children.
    while let Some(child) = scripts_box.first_child() {
        scripts_box.remove(&child);
    }
    if scripts.is_empty() {
        let empty = gtk::Label::builder()
            .label("No scripts found. Make sure your scripts are on $PATH \
                   and start with the configured prefix.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        scripts_box.append(&empty);
        return;
    }
    for name in scripts {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        let icon = gtk::Image::builder()
            .icon_name("utilities-terminal-symbolic")
            .build();
        let label = gtk::Label::builder()
            .label(name)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        label.add_css_class("label-medium");
        row.append(&icon);
        row.append(&label);
        scripts_box.append(&row);
    }
    let _ = glib::MainContext::default();
}
