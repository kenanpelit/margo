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
use mshell_config::schema::config::{
    ConfigStoreFields, LauncherStoreFields, PassStoreFields, ScriptAutostart,
};
use mshell_launcher::providers::ScriptsProvider;
use mshell_launcher::{frecency, history};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct LauncherSettingsModel {
    /// User-managed startup-script list — the source of truth.
    /// Add by name (text entry below), delete per row; each entry
    /// carries its run-at-startup toggle + post-startup delay.
    /// Mirrors `config.launcher.autostart_scripts`, refreshed on
    /// every add / remove.
    scripts: Vec<ScriptAutostart>,
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
    /// Add a script to the startup list by name (text entry).
    /// No-op on empty / duplicate names.
    AddScript(String),
    /// Remove a script from the startup list (delete button).
    RemoveScript(String),
    /// Toggle a script's run-at-startup flag (by short name).
    SetAutostart(String, bool),
    /// Set how many seconds after startup a script runs (by name).
    SetDelay(String, u32),
    /// Set the GNU pass store directory (`config.pass.store_path`).
    /// Empty falls back to $PASSWORD_STORE_DIR / ~/.password-store.
    SetPassStorePath(String),
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
                        "Type a script name and click Add to put it on the \
                         startup list. Tick a row to run it at shell startup, \
                         and set how many seconds after startup it should \
                         launch. Names match executables on $PATH (e.g. \
                         `{prefix}foo`), which also run via `>start` in the \
                         launcher. {count} script(s) listed.",
                        prefix = ScriptsProvider::DEFAULT_PREFIX,
                        count = model.scripts.len(),
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
                    set_height_request: 240,
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
                    set_spacing: 8,

                    #[name = "name_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("script name, e.g. start-foo"),
                        // Enter in the entry adds too.
                        connect_activate[sender] => move |e| {
                            sender.input(LauncherSettingsInput::AddScript(
                                e.text().to_string(),
                            ));
                            e.set_text("");
                        },
                    },

                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Add",
                        connect_clicked[sender, name_entry] => move |_| {
                            sender.input(LauncherSettingsInput::AddScript(
                                name_entry.text().to_string(),
                            ));
                            name_entry.set_text("");
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
        let model = LauncherSettingsModel {
            scripts: read_autostart_scripts(),
        };

        let widgets = view_output!();

        rebuild_scripts_box(&widgets.scripts_box, &model.scripts, &sender);
        widgets.pass_store_entry.set_text(
            &config_manager().config().pass().store_path().get_untracked(),
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
            LauncherSettingsInput::AddScript(name) => {
                let name = name.trim().to_string();
                if name.is_empty() || self.scripts.iter().any(|e| e.name == name) {
                    // Empty or already listed — nothing to do.
                    return;
                }
                // New entries default to enabled so "type + Add" is
                // enough to autostart; the toggle turns it back off.
                upsert_autostart(&name, |e| e.enabled = true);
                self.scripts = read_autostart_scripts();
                rebuild_scripts_box(&widgets.scripts_box, &self.scripts, &sender);
            }
            LauncherSettingsInput::RemoveScript(name) => {
                config_manager().update_config(|config| {
                    config.launcher.autostart_scripts.retain(|e| e.name != name);
                });
                self.scripts = read_autostart_scripts();
                rebuild_scripts_box(&widgets.scripts_box, &self.scripts, &sender);
            }
            LauncherSettingsInput::SetAutostart(name, enabled) => {
                upsert_autostart(&name, |e| e.enabled = enabled);
            }
            LauncherSettingsInput::SetDelay(name, secs) => {
                upsert_autostart(&name, |e| e.delay_secs = secs);
            }
            LauncherSettingsInput::SetPassStorePath(path) => {
                let path = path.trim().to_string();
                config_manager().update_config(move |config| {
                    config.pass.store_path = path;
                });
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Snapshot the user's startup-script list from config.
fn read_autostart_scripts() -> Vec<ScriptAutostart> {
    config_manager()
        .config()
        .launcher()
        .autostart_scripts()
        .get_untracked()
}

/// Repaint the startup-scripts list: one row per configured entry —
/// name, "after N s" delay spin, run-at-startup toggle, and a delete
/// button. Toggle / spin persist through `upsert_autostart`; delete
/// routes back through `RemoveScript`.
fn rebuild_scripts_box(
    scripts_box: &gtk::Box,
    scripts: &[ScriptAutostart],
    sender: &ComponentSender<LauncherSettingsModel>,
) {
    while let Some(child) = scripts_box.first_child() {
        scripts_box.remove(&child);
    }
    if scripts.is_empty() {
        let empty = gtk::Label::builder()
            .label("No startup scripts yet. Type a script name above and \
                   click Add.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        scripts_box.append(&empty);
        return;
    }

    for entry in scripts {
        let name = entry.name.clone();

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        row.add_css_class("launcher-script-row");

        let icon = gtk::Image::from_icon_name("utilities-terminal-symbolic");
        let label = gtk::Label::builder()
            .label(&name)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        label.add_css_class("label-medium");
        row.append(&icon);
        row.append(&label);

        // Delay: "after N s" — relevant once autostart is on.
        let after = gtk::Label::new(Some("after"));
        after.add_css_class("label-small");
        let delay_spin = gtk::SpinButton::with_range(0.0, 3600.0, 1.0);
        delay_spin.set_digits(0);
        delay_spin.set_valign(gtk::Align::Center);
        delay_spin.set_tooltip_text(Some("Seconds after startup before this runs"));
        delay_spin.set_value(entry.delay_secs as f64); // before connect → no spurious input
        let secs = gtk::Label::new(Some("s"));
        secs.add_css_class("label-small");
        {
            let name = name.clone();
            let sender = sender.clone();
            delay_spin.connect_value_changed(move |s| {
                sender.input(LauncherSettingsInput::SetDelay(
                    name.clone(),
                    s.value().max(0.0) as u32,
                ));
            });
        }
        row.append(&after);
        row.append(&delay_spin);
        row.append(&secs);

        // Autostart toggle.
        let toggle = gtk::Switch::new();
        toggle.set_valign(gtk::Align::Center);
        toggle.set_tooltip_text(Some("Run at shell startup"));
        toggle.set_active(entry.enabled); // before connect → no spurious input
        {
            let name = name.clone();
            let sender = sender.clone();
            toggle.connect_active_notify(move |sw| {
                sender.input(LauncherSettingsInput::SetAutostart(name.clone(), sw.is_active()));
            });
        }
        row.append(&toggle);

        // Delete — drop the entry from the startup list.
        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.add_css_class("ok-button-flat");
        delete.set_valign(gtk::Align::Center);
        delete.set_tooltip_text(Some("Remove from startup list"));
        {
            let name = name.clone();
            let sender = sender.clone();
            delete.connect_clicked(move |_| {
                sender.input(LauncherSettingsInput::RemoveScript(name.clone()));
            });
        }
        row.append(&delete);

        scripts_box.append(&row);
    }
}

/// Find-or-insert a script's autostart entry by name and mutate it,
/// persisting to config (the startup runner + Settings both read it).
fn upsert_autostart(name: &str, mutate: impl FnOnce(&mut ScriptAutostart)) {
    config_manager().update_config(|config| {
        if let Some(entry) = config
            .launcher
            .autostart_scripts
            .iter_mut()
            .find(|e| e.name == name)
        {
            mutate(entry);
        } else {
            let mut entry = ScriptAutostart {
                name: name.to_string(),
                enabled: false,
                delay_secs: 0,
            };
            mutate(&mut entry);
            config.launcher.autostart_scripts.push(entry);
        }
    });
}
