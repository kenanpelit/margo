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
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug)]
pub(crate) struct LauncherSettingsModel {
    /// Snapshot of the currently-indexed `>start` scripts as
    /// `(name, absolute path)`. Re-scanned on init and after every
    /// add / delete / refresh; the path is what edit + delete act on.
    indexed_scripts: Vec<(String, PathBuf)>,
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
    /// Open a script file in the system default editor (xdg-open).
    EditScript(PathBuf),
    /// Move a script file to the trash (recoverable), then re-scan.
    DeleteScript(PathBuf),
    /// Create a new `start-<name>` script (shebang + chmod +x) in a
    /// writable scripts dir, open it for editing, then re-scan.
    NewScript(String),
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

                // New script: type a name → create start-<name> + open.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    #[name = "new_script_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("New script name (e.g. brave-ai → start-brave-ai)"),
                    },

                    #[name = "new_script_add"]
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_label: "New script",
                    },
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

        // Wire the "New script" entry + button (Enter or click).
        let entry = widgets.new_script_entry.clone();
        let submit = {
            let sender = sender.clone();
            move |entry: &gtk::Entry| {
                let name = entry.text().trim().to_string();
                if !name.is_empty() {
                    sender.input(LauncherSettingsInput::NewScript(name));
                    entry.set_text("");
                }
            }
        };
        {
            let entry = entry.clone();
            let submit = submit.clone();
            widgets
                .new_script_add
                .connect_clicked(move |_| submit(&entry));
        }
        widgets
            .new_script_entry
            .connect_activate(move |e| submit(e));

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
            LauncherSettingsInput::EditScript(path) => {
                open_in_editor(&path);
            }
            LauncherSettingsInput::DeleteScript(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if trash_file(&path) {
                    mshell_launcher::notify::toast(
                        "Script moved to trash",
                        &format!("{name} — recover from your trash if needed."),
                    );
                    self.indexed_scripts = rebuild_indexed_scripts();
                    rebuild_scripts_box(&widgets.scripts_box, &self.indexed_scripts, &sender);
                } else {
                    mshell_launcher::notify::toast(
                        "Delete failed",
                        &format!("Could not move {name} to trash."),
                    );
                }
            }
            LauncherSettingsInput::NewScript(name) => {
                match create_script(&self.indexed_scripts, &name) {
                    Ok(path) => {
                        open_in_editor(&path);
                        mshell_launcher::notify::toast(
                            "Script created",
                            &format!(
                                "{} — opened for editing.",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                        );
                        self.indexed_scripts = rebuild_indexed_scripts();
                        rebuild_scripts_box(&widgets.scripts_box, &self.indexed_scripts, &sender);
                    }
                    Err(err) => {
                        mshell_launcher::notify::toast("Could not create script", &err);
                    }
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Re-scan PATH for `start-*` executables → `(name, path)` pairs.
/// Cheap (~tens of ms for ~500 PATH entries) so it's fine to run on
/// every add / delete / refresh.
fn rebuild_indexed_scripts() -> Vec<(String, PathBuf)> {
    ScriptsProvider::new().indexed()
}

/// Replace every child in `scripts_box` with one editable row:
/// terminal icon + name + Edit (open in editor) + Delete (trash).
fn rebuild_scripts_box(
    scripts_box: &gtk::Box,
    scripts: &[(String, PathBuf)],
    sender: &ComponentSender<LauncherSettingsModel>,
) {
    while let Some(child) = scripts_box.first_child() {
        scripts_box.remove(&child);
    }
    if scripts.is_empty() {
        let empty = gtk::Label::builder()
            .label("No scripts yet. Use \u{201c}New script\u{201d} above, or drop a \
                   start-* executable onto $PATH.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        scripts_box.append(&empty);
        return;
    }
    for (name, path) in scripts {
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
        label.set_tooltip_text(Some(&path.display().to_string()));
        row.append(&icon);
        row.append(&label);

        let edit = gtk::Button::new();
        edit.add_css_class("ok-button-surface");
        edit.set_valign(gtk::Align::Center);
        edit.set_tooltip_text(Some("Edit script"));
        edit.set_child(Some(&gtk::Image::from_icon_name("document-edit-symbolic")));
        {
            let path = path.clone();
            let sender = sender.clone();
            edit.connect_clicked(move |_| {
                sender.input(LauncherSettingsInput::EditScript(path.clone()));
            });
        }
        row.append(&edit);

        let del = gtk::Button::new();
        del.add_css_class("ok-button-surface");
        del.set_valign(gtk::Align::Center);
        del.set_tooltip_text(Some("Delete (move to trash)"));
        del.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        {
            let path = path.clone();
            let sender = sender.clone();
            del.connect_clicked(move |_| {
                sender.input(LauncherSettingsInput::DeleteScript(path.clone()));
            });
        }
        row.append(&del);

        scripts_box.append(&row);
    }
    let _ = glib::MainContext::default();
}

/// Open a file in the system default app (a text editor for scripts).
fn open_in_editor(path: &PathBuf) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

/// Move a file to the trash via `gio trash` (recoverable, not `rm`).
fn trash_file(path: &PathBuf) -> bool {
    Command::new("gio")
        .arg("trash")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Create a new `start-<name>` script (shebang + executable bit) and
/// return its path, or a user-facing error string.
fn create_script(existing: &[(String, PathBuf)], raw_name: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let cleaned: String = raw_name
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    let file_name = if cleaned.starts_with(ScriptsProvider::DEFAULT_PREFIX) {
        cleaned
    } else {
        format!("{}{}", ScriptsProvider::DEFAULT_PREFIX, cleaned)
    };
    if file_name == ScriptsProvider::DEFAULT_PREFIX {
        return Err("Empty script name.".into());
    }

    let dir = scripts_target_dir(existing);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir: {e}"))?;
    let path = dir.join(&file_name);
    if path.exists() {
        return Err(format!("{file_name} already exists in {}.", dir.display()));
    }

    std::fs::write(
        &path,
        "#!/usr/bin/env bash\n# Created from margo launcher settings.\n\n",
    )
    .map_err(|e| format!("write: {e}"))?;
    if let Ok(meta) = std::fs::metadata(&path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(&path, perms);
    }
    Ok(path)
}

/// Where new scripts land: the first writable parent dir among the
/// user's existing scripts, else `~/.local/bin`.
fn scripts_target_dir(existing: &[(String, PathBuf)]) -> PathBuf {
    for (_, p) in existing {
        if let Some(parent) = p.parent() {
            if is_writable_dir(parent) {
                return parent.to_path_buf();
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".local/bin")
}

fn is_writable_dir(dir: &std::path::Path) -> bool {
    let probe = dir.join(".margo-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
