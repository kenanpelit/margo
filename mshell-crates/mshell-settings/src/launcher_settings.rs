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
use mshell_config::schema::config::{ConfigStoreFields, LauncherStoreFields, ScriptAutostart};
use mshell_launcher::providers::ScriptsProvider;
use mshell_launcher::{frecency, history};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct LauncherSettingsModel {
    /// Short names of the currently-indexed `>start` scripts,
    /// re-scanned on init / refresh. Per-script autostart state
    /// (enabled + delay) lives in config, looked up by name.
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
    /// Toggle a script's run-at-startup flag (by short name).
    SetAutostart(String, bool),
    /// Set how many seconds after startup a script runs (by name).
    SetDelay(String, u32),
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
                        "Run any `{prefix}*` executable on $PATH via `>start` \
                         in the launcher. Tick a script to run it at shell \
                         startup, and set how many seconds after startup it \
                         should launch. Indexed: {count} script(s).",
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

        rebuild_scripts_box(&widgets.scripts_box, &model.indexed_scripts, &sender);

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
                rebuild_scripts_box(&widgets.scripts_box, &self.indexed_scripts, &sender);
            }
            LauncherSettingsInput::SetAutostart(name, enabled) => {
                upsert_autostart(&name, |e| e.enabled = enabled);
            }
            LauncherSettingsInput::SetDelay(name, secs) => {
                upsert_autostart(&name, |e| e.delay_secs = secs);
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Re-scan PATH for `start-*` executables → short names.
fn rebuild_indexed_scripts() -> Vec<String> {
    ScriptsProvider::new().indexed_names()
}

/// Repaint the scripts list: one row per discovered script with an
/// autostart toggle + a "after N s" delay spin. Initial states are
/// read from `config.launcher.autostart_scripts` (keyed by name);
/// toggling / spinning persists back through `upsert_autostart`.
fn rebuild_scripts_box(
    scripts_box: &gtk::Box,
    scripts: &[String],
    sender: &ComponentSender<LauncherSettingsModel>,
) {
    while let Some(child) = scripts_box.first_child() {
        scripts_box.remove(&child);
    }
    if scripts.is_empty() {
        let empty = gtk::Label::builder()
            .label("No scripts found. Drop a start-* executable onto $PATH, \
                   then hit Refresh.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        scripts_box.append(&empty);
        return;
    }

    let autostart = config_manager()
        .config()
        .launcher()
        .autostart_scripts()
        .get_untracked();

    for name in scripts {
        let cfg = autostart.iter().find(|e| &e.name == name);
        let enabled = cfg.map(|c| c.enabled).unwrap_or(false);
        let delay = cfg.map(|c| c.delay_secs).unwrap_or(0);

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        row.add_css_class("launcher-script-row");

        let icon = gtk::Image::from_icon_name("utilities-terminal-symbolic");
        let label = gtk::Label::builder()
            .label(name)
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
        delay_spin.set_value(delay as f64); // before connect → no spurious input
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
        toggle.set_active(enabled); // before connect → no spurious input
        {
            let name = name.clone();
            let sender = sender.clone();
            toggle.connect_active_notify(move |sw| {
                sender.input(LauncherSettingsInput::SetAutostart(name.clone(), sw.is_active()));
            });
        }
        row.append(&toggle);

        scripts_box.append(&row);
    }
    let _ = glib::MainContext::default();
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
