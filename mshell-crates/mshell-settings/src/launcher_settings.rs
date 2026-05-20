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

use mshell_config::config_manager::config_manager;
use mshell_config::schema::clipboard::{ClipboardClearPolicy, ClipboardPersist};
use mshell_launcher::providers::ScriptsProvider;
use mshell_launcher::{frecency, history};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Read the current clipboard config and push it to the running
/// watcher (live max-entries / persist / clear-policy / sensitive /
/// image-history change). Called after each settings mutation.
fn apply_clipboard_config() {
    use mshell_clipboard::{ClearPolicy, ClipboardSettings, PersistMode};
    let c = config_manager().config().get_untracked().clipboard;
    mshell_clipboard::clipboard_service().apply_settings(ClipboardSettings {
        max_entries: c.max_entries.max(1),
        persist: match c.persist {
            ClipboardPersist::None => PersistMode::None,
            ClipboardPersist::FavoritesOnly => PersistMode::FavoritesOnly,
            ClipboardPersist::All => PersistMode::All,
        },
        clear_policy: match c.clear_policy {
            ClipboardClearPolicy::Never => ClearPolicy::Never,
            ClipboardClearPolicy::AfterHours => ClearPolicy::AfterHours,
            ClipboardClearPolicy::OnLogout => ClearPolicy::OnLogout,
        },
        clear_after_hours: c.clear_after_hours,
        skip_sensitive: c.skip_sensitive,
        image_history: c.image_history,
    });
}

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
    /// Clear everything except pinned (favourite) clipboard entries.
    ClearClipboardUnpinned,
    /// Re-scan PATH to refresh the indexed-scripts display.
    RefreshScripts,
    // ── Clipboard behaviour knobs (write config + live-apply) ──
    SetClipboardMaxEntries(i32),
    SetClipboardPersist(u32),
    SetClipboardClearPolicy(u32),
    SetClipboardClearHours(i32),
    SetClipboardSkipSensitive(bool),
    SetClipboardImageHistory(bool),
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
                    set_label: "Clipboard history",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Tracks every copy from any app via the \
                                Wayland clipboard. Pinned (★) entries are \
                                exempt from eviction + auto-clear and \
                                persist regardless of the persist mode.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "History size", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_max"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(100.0, 5.0, 10000.0, 5.0, 100.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(LauncherSettingsInput::SetClipboardMaxEntries(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Persist to disk", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_persist"]
                    gtk::DropDown {
                        connect_selected_notify[sender] => move |d| {
                            sender.input(LauncherSettingsInput::SetClipboardPersist(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Auto-clear", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_clear"]
                    gtk::DropDown {
                        connect_selected_notify[sender] => move |d| {
                            sender.input(LauncherSettingsInput::SetClipboardClearPolicy(d.selected()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Clear after (hours)", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_hours"]
                    gtk::SpinButton {
                        set_adjustment: &gtk::Adjustment::new(24.0, 1.0, 720.0, 1.0, 6.0, 0.0),
                        set_digits: 0,
                        connect_value_changed[sender] => move |s| {
                            sender.input(LauncherSettingsInput::SetClipboardClearHours(s.value() as i32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Skip password-manager copies", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_sensitive"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(LauncherSettingsInput::SetClipboardSkipSensitive(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Label { set_label: "Keep image copies", set_hexpand: true, set_halign: gtk::Align::Start },
                    #[name = "cb_images"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_state_set[sender] => move |_, state| {
                            sender.input(LauncherSettingsInput::SetClipboardImageHistory(state));
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Clear unpinned",
                        connect_clicked[sender] => move |_| {
                            sender.input(LauncherSettingsInput::ClearClipboardUnpinned);
                        },
                    },
                },

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

        // Prime the clipboard controls from config.
        let cb = config_manager().config().get_untracked().clipboard;
        widgets.cb_persist.set_model(Some(&gtk::StringList::new(
            &ClipboardPersist::display_names(),
        )));
        widgets.cb_persist.set_selected(cb.persist.to_index());
        widgets.cb_clear.set_model(Some(&gtk::StringList::new(
            &ClipboardClearPolicy::display_names(),
        )));
        widgets.cb_clear.set_selected(cb.clear_policy.to_index());
        widgets.cb_max.set_value(cb.max_entries as f64);
        widgets.cb_hours.set_value(cb.clear_after_hours as f64);
        widgets.cb_sensitive.set_active(cb.skip_sensitive);
        widgets.cb_images.set_active(cb.image_history);

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
            LauncherSettingsInput::ClearClipboardUnpinned => {
                mshell_clipboard::clipboard_service().clear_unpinned();
                mshell_launcher::notify::toast(
                    "Clipboard cleared",
                    "Unpinned entries removed; favorites kept.",
                );
            }
            LauncherSettingsInput::SetClipboardMaxEntries(v) => {
                config_manager().update_config(|c| c.clipboard.max_entries = v.max(1) as usize);
                apply_clipboard_config();
            }
            LauncherSettingsInput::SetClipboardPersist(i) => {
                config_manager()
                    .update_config(|c| c.clipboard.persist = ClipboardPersist::from_index(i));
                apply_clipboard_config();
            }
            LauncherSettingsInput::SetClipboardClearPolicy(i) => {
                config_manager().update_config(|c| {
                    c.clipboard.clear_policy = ClipboardClearPolicy::from_index(i)
                });
                apply_clipboard_config();
            }
            LauncherSettingsInput::SetClipboardClearHours(v) => {
                config_manager().update_config(|c| c.clipboard.clear_after_hours = v.max(1) as u32);
                apply_clipboard_config();
            }
            LauncherSettingsInput::SetClipboardSkipSensitive(on) => {
                config_manager().update_config(|c| c.clipboard.skip_sensitive = on);
                apply_clipboard_config();
            }
            LauncherSettingsInput::SetClipboardImageHistory(on) => {
                config_manager().update_config(|c| c.clipboard.image_history = on);
                apply_clipboard_config();
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
