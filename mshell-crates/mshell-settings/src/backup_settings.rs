//! Settings → Backup. Profiles + whole-config export / import / reset.
//!
//! - **Profiles** — switch / create / delete the shell-config profiles the
//!   `config_manager` already maintains (`~/.config/margo/mshell/profiles/`).
//! - **Export** — bundle the live config into a portable `.tar.gz`: the shell
//!   profiles **and** the compositor's hand-edited `config.conf` + `binds.conf`
//!   (machine-generated fragments — colours, plugin binds — regenerate
//!   themselves and are left out).
//! - **Import** — restore a bundle: files are copied **through** any dotfiles
//!   symlink (so a symlinked `config.conf` keeps its link), then the shell
//!   reloads and `mctl reload` re-reads the compositor side.
//! - **Reset** — return the shell config to `Config::default()` (the compositor
//!   `config.conf` is left untouched — re-import or edit it directly).

use std::path::{Path, PathBuf};
use std::process::Command;

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::Config;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::gio;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// `~/.config/margo` (honours `XDG_CONFIG_HOME`), the tar root.
fn margo_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("margo")
}

/// The hand-edited members worth bundling, relative to `margo_dir()`. Skips
/// machine-generated fragments (`conf.d/colors.conf`, `binds.d/`) — they
/// regenerate. `-h` dereferences the frequently-symlinked `config.conf`.
const BUNDLE_MEMBERS: &[&str] = &["config.conf", "binds.conf", "mshell/profiles"];

/// Tar the existing bundle members into `dest` (`.tar.gz`).
fn export_bundle(dest: &Path) -> std::io::Result<()> {
    let base = margo_dir();
    let members: Vec<&str> = BUNDLE_MEMBERS
        .iter()
        .copied()
        .filter(|m| base.join(m).exists())
        .collect();
    if members.is_empty() {
        return Err(std::io::Error::other("nothing to export"));
    }
    let ok = Command::new("tar")
        .arg("czhf") // h: dereference the config.conf dotfiles symlink
        .arg(dest)
        .arg("-C")
        .arg(&base)
        .args(&members)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::other("tar export failed"))
    }
}

/// Extract `src` to a temp dir, then copy each member to its real path —
/// `fs::copy` follows a symlinked destination, so a dotfiles-linked
/// `config.conf` is written through rather than clobbered. Reloads after.
fn import_bundle(src: &Path) -> std::io::Result<()> {
    let base = margo_dir();
    let tmp = std::env::temp_dir().join(format!("margo-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;

    let ok = Command::new("tar")
        .arg("xzf")
        .arg(src)
        .arg("-C")
        .arg(&tmp)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(std::io::Error::other(
            "tar import failed (not a margo bundle?)",
        ));
    }

    // Single files: write through the (possibly symlinked) destination.
    for f in ["config.conf", "binds.conf"] {
        let from = tmp.join(f);
        if from.is_file() {
            let _ = std::fs::copy(&from, base.join(f));
        }
    }
    // Profiles: copy every *.yaml into the profiles dir.
    let prof_src = tmp.join("mshell/profiles");
    let prof_dst = base.join("mshell/profiles");
    if prof_src.is_dir() {
        let _ = std::fs::create_dir_all(&prof_dst);
        if let Ok(rd) = std::fs::read_dir(&prof_src) {
            for entry in rd.flatten() {
                if entry.path().extension().is_some_and(|e| e == "yaml") {
                    let _ = std::fs::copy(entry.path(), prof_dst.join(entry.file_name()));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);

    // Apply: shell reloads its config; the compositor re-reads config.conf.
    config_manager().reload_config();
    let _ = Command::new("mctl").arg("reload").spawn();
    Ok(())
}

#[derive(Debug)]
pub(crate) enum BackupSettingsInput {
    SelectProfile(u32),
    SetNewName(String),
    NewProfile,
    DeleteProfile,
    ExportClicked,
    ImportClicked,
    DoImport(PathBuf),
    ResetClicked,
    DoReset,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum BackupSettingsOutput {}
#[derive(Debug)]
pub(crate) enum BackupSettingsCommandOutput {}
pub(crate) struct BackupSettingsInit {}

pub(crate) struct BackupSettingsModel {
    profiles: Vec<String>,
    active: Option<String>,
    profile_model: gtk::StringList,
    profile_dd: gtk::DropDown,
    f_new: String,
}

fn read_profiles() -> (Vec<String>, Option<String>) {
    (
        config_manager().available_profiles().get_untracked(),
        config_manager().active_profile().get_untracked(),
    )
}

impl BackupSettingsModel {
    /// Repaint the profile dropdown from the live profile list + active name.
    fn refresh(&mut self) {
        let (profiles, active) = read_profiles();
        self.profiles = profiles;
        self.active = active;
        // Splice the StringList in place (avoid set_model churn).
        let n = self.profile_model.n_items();
        self.profile_model.splice(
            0,
            n,
            &self.profiles.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        if let Some(active) = &self.active
            && let Some(idx) = self.profiles.iter().position(|p| p == active)
        {
            self.profile_dd.set_selected(idx as u32);
        }
    }
}

#[relm4::component(pub)]
impl Component for BackupSettingsModel {
    type CommandOutput = BackupSettingsCommandOutput;
    type Input = BackupSettingsInput;
    type Output = BackupSettingsOutput;
    type Init = BackupSettingsInit;

    view! {
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("document-save-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Backup", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Switch config profiles, export the whole setup to a portable .tar.gz, import one back, or reset the shell to defaults.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ════════ Profiles ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Profiles", set_halign: gtk::Align::Start },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "Named snapshots of your shell config. Switching applies instantly.",
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[local_ref]
                    profile_dd -> gtk::DropDown {
                        set_hexpand: true,
                        connect_selected_notify[sender] => move |d| sender.input(BackupSettingsInput::SelectProfile(d.selected())),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Delete",
                        connect_clicked[sender] => move |_| sender.input(BackupSettingsInput::DeleteProfile),
                    },
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[name = "new_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("new profile name (snapshots the current config)"),
                        connect_changed[sender] => move |e| sender.input(BackupSettingsInput::SetNewName(e.text().to_string())),
                        connect_activate[sender] => move |_| sender.input(BackupSettingsInput::NewProfile),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Save as",
                        connect_clicked[sender] => move |_| sender.input(BackupSettingsInput::NewProfile),
                    },
                },

                // ════════ Backup ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Backup", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "Export bundles the shell profiles + the compositor's config.conf and binds.conf into one .tar.gz. Import restores a bundle and reloads.",
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Export…",
                        connect_clicked[sender] => move |_| sender.input(BackupSettingsInput::ExportClicked),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Import…",
                        connect_clicked[sender] => move |_| sender.input(BackupSettingsInput::ImportClicked),
                    },
                },

                // ════════ Reset ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Reset", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "Return the shell config to factory defaults. The compositor config.conf is left untouched.",
                },
                gtk::Button {
                    add_css_class: "destructive-action",
                    set_halign: gtk::Align::Start,
                    set_label: "Reset shell to defaults…",
                    connect_clicked[sender] => move |_| sender.input(BackupSettingsInput::ResetClicked),
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (profiles, active) = read_profiles();
        let profile_model =
            gtk::StringList::new(&profiles.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let profile_dd = gtk::DropDown::builder().model(&profile_model).build();
        if let Some(active) = &active
            && let Some(idx) = profiles.iter().position(|p| p == active)
        {
            profile_dd.set_selected(idx as u32);
        }

        let model = BackupSettingsModel {
            profiles,
            active,
            profile_model,
            profile_dd: profile_dd.clone(),
            f_new: String::new(),
        };
        let profile_dd = model.profile_dd.clone();
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            BackupSettingsInput::SelectProfile(idx) => {
                if let Some(name) = self.profiles.get(idx as usize)
                    && self.active.as_deref() != Some(name.as_str())
                {
                    config_manager().set_active_profile(Some(name.clone()));
                    self.active = Some(name.clone());
                }
            }
            BackupSettingsInput::SetNewName(v) => self.f_new = v,
            BackupSettingsInput::NewProfile => {
                let name = self.f_new.trim().to_string();
                if name.is_empty() {
                    return;
                }
                // Snapshot the live config under the new name + make it active.
                let _ = config_manager().snapshot_active_as(&name);
                self.f_new.clear();
                self.refresh();
                mshell_launcher::notify::toast("Profile saved", &name);
            }
            BackupSettingsInput::DeleteProfile => {
                if let Some(name) = self.active.clone()
                    && self.profiles.len() > 1
                {
                    let _ = config_manager().delete_profile(&name);
                    self.refresh();
                    mshell_launcher::notify::toast("Profile deleted", &name);
                }
            }
            BackupSettingsInput::ExportClicked => {
                let sender = sender.clone();
                let dialog = gtk::FileDialog::builder()
                    .title("Export margo config")
                    .modal(true)
                    .initial_name("margo-config.tar.gz")
                    .build();
                dialog.save(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        match export_bundle(&path) {
                            Ok(()) => mshell_launcher::notify::toast(
                                "Config exported",
                                path.display().to_string(),
                            ),
                            Err(e) => {
                                mshell_launcher::notify::toast("Export failed", e.to_string())
                            }
                        }
                    }
                    let _ = &sender; // keep the closure's sender alive
                });
            }
            BackupSettingsInput::ImportClicked => {
                let sender = sender.clone();
                let dialog = gtk::FileDialog::builder()
                    .title("Import margo config")
                    .modal(true)
                    .build();
                let filter = gtk::FileFilter::new();
                filter.add_pattern("*.tar.gz");
                filter.add_pattern("*.tgz");
                filter.set_name(Some("margo bundle (.tar.gz)"));
                dialog.set_default_filter(Some(&filter));
                dialog.open(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        sender.input(BackupSettingsInput::DoImport(path));
                    }
                });
            }
            BackupSettingsInput::DoImport(path) => {
                // Confirm — import overwrites the live config.
                let sender = sender.clone();
                let dialog = gtk::AlertDialog::builder()
                    .modal(true)
                    .message("Import this config bundle?")
                    .detail("It overwrites your current shell + compositor config. Export a backup first if unsure.")
                    .buttons(["Cancel", "Import"])
                    .cancel_button(0)
                    .default_button(1)
                    .build();
                dialog.choose(gtk::Window::NONE, gio::Cancellable::NONE, move |res| {
                    if res == Ok(1) {
                        match import_bundle(&path) {
                            Ok(()) => {
                                mshell_launcher::notify::toast("Config imported", "Reloaded.");
                                sender.input(BackupSettingsInput::Refresh);
                            }
                            Err(e) => {
                                mshell_launcher::notify::toast("Import failed", e.to_string())
                            }
                        }
                    }
                });
            }
            BackupSettingsInput::ResetClicked => {
                let sender = sender.clone();
                let dialog = gtk::AlertDialog::builder()
                    .modal(true)
                    .message("Reset shell settings to defaults?")
                    .detail("Your shell config returns to factory defaults. The compositor config.conf is left as-is.")
                    .buttons(["Cancel", "Reset"])
                    .cancel_button(0)
                    .default_button(1)
                    .build();
                dialog.choose(gtk::Window::NONE, gio::Cancellable::NONE, move |res| {
                    if res == Ok(1) {
                        sender.input(BackupSettingsInput::DoReset);
                    }
                });
            }
            BackupSettingsInput::DoReset => {
                config_manager().update_config(|c| *c = Config::default());
                self.refresh();
                mshell_launcher::notify::toast("Shell reset", "Defaults restored.");
            }
            BackupSettingsInput::Refresh => self.refresh(),
        }
    }
}
