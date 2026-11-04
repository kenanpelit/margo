//! Settings → Tune — the folder-first music player (`mtune`).
//!
//! Edits `~/.config/margo/mtune.toml` directly (mtune's own config world,
//! like `mpower.toml` — NOT `config_manager()`). Library-root changes are
//! also pushed to a running mtune over `org.margo.Tune::SetLibraryRoots`
//! so it rescans immediately; the other knobs take effect on mtune's next
//! launch.

use margo_fs::side_file::{self, SaveError, SideLoad};
use mshell_services::mtune::mtune_service;
use mshell_services::tokio_rt_spawn;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── mtune.toml mirror (only the fields this page edits are named; the
//    rest round-trip through `#[serde(flatten)]`, at the top level and in
//    each section, so a hand-edited file keeps its other keys). ─────────

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct MtuneToml {
    #[serde(default)]
    library: LibrarySection,
    #[serde(default)]
    playback: PlaybackSection,
    #[serde(default)]
    behaviour: BehaviourSection,
    /// Every other top-level key/section mtune has (or adds later), kept
    /// as is so a save from this page never drops it.
    #[serde(flatten)]
    rest: toml::Table,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct LibrarySection {
    #[serde(default)]
    roots: Vec<String>,
    #[serde(flatten)]
    rest: toml::Table,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PlaybackSection {
    #[serde(default = "default_on_start")]
    on_start: String,
    #[serde(flatten)]
    rest: toml::Table,
}
impl Default for PlaybackSection {
    fn default() -> Self {
        Self {
            on_start: default_on_start(),
            rest: toml::Table::new(),
        }
    }
}
fn default_on_start() -> String {
    "resume".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BehaviourSection {
    #[serde(default = "default_true")]
    close_to_tray: bool,
    #[serde(default)]
    start_hidden: bool,
    #[serde(flatten)]
    rest: toml::Table,
}
impl Default for BehaviourSection {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            start_hidden: false,
            rest: toml::Table::new(),
        }
    }
}
fn default_true() -> bool {
    true
}

fn config_path() -> PathBuf {
    let base = margo_fs::xdg::config_home();
    base.join("margo").join("mtune.toml")
}

/// Load mtune.toml. A file that does not parse shows defaults (with a
/// warning and a `mtune.toml.bak` copy); [`save`] then refuses to write.
fn load() -> MtuneToml {
    load_from(&config_path())
}

fn load_from(path: &Path) -> MtuneToml {
    let loaded = side_file::load(path, toml::from_str::<MtuneToml>);
    if let SideLoad::Broken { error, backup } = &loaded {
        tracing::warn!(
            path = %path.display(),
            backup = ?backup,
            error,
            "mtune-settings: mtune.toml does not parse; showing defaults, not saving over it"
        );
    }
    loaded.or_default()
}

/// Save mtune.toml (atomic). Never over a file that does not parse:
/// mtune owns it, and `library.roots` would be reset to `[]`.
fn save(cfg: &MtuneToml) {
    let path = config_path();
    mshell_config::toast::report_side_save(&path, save_to(&path, cfg));
}

fn save_to(path: &Path, cfg: &MtuneToml) -> Result<(), SaveError> {
    let body = toml::to_string_pretty(cfg).map_err(|e| SaveError::Io {
        path: path.to_path_buf(),
        error: std::io::Error::other(e),
    })?;
    side_file::save(path, body.as_bytes(), |raw| {
        toml::from_str::<MtuneToml>(raw).map(|_| ())
    })
}

const ON_START: &[(&str, &str)] = &[
    ("resume", "Resume the last track"),
    ("library", "Select the top of the library"),
    ("nothing", "Do nothing"),
];

// ── Component ─────────────────────────────────────────────────────────

pub(crate) struct MtuneSettingsModel {
    cfg: MtuneToml,
}

#[derive(Debug)]
pub(crate) enum MtuneSettingsInput {
    AddRoot,
    RemoveRoot(usize),
    OnStartChanged(u32),
    CloseToTrayToggled(bool),
    StartHiddenToggled(bool),
    FolderPicked(String),
}

#[derive(Debug)]
pub(crate) enum MtuneSettingsOutput {}

pub(crate) struct MtuneSettingsInit {}

#[derive(Debug)]
pub(crate) enum MtuneSettingsCmd {}

#[relm4::component(pub)]
impl Component for MtuneSettingsModel {
    type CommandOutput = MtuneSettingsCmd;
    type Input = MtuneSettingsInput;
    type Output = MtuneSettingsOutput;
    type Init = MtuneSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 16,
                set_hexpand: true,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("org.margo.Tune-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Tune",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The folder-first music player. Point it at your music folders; it scans them recursively and keeps the queue in sync.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Library folders ────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Library folders",
                    set_halign: gtk::Align::Start,
                },
                #[name = "roots_list"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_halign: gtk::Align::Start,
                    set_label: "Add folder…",
                    connect_clicked => MtuneSettingsInput::AddRoot,
                },

                // ── Playback ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Playback",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_spacing: 20,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_hexpand: true,
                            set_halign: gtk::Align::Start,
                            set_label: "On launch",
                        },
                        #[name = "on_start_drop"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                        },
                    },
                },

                // ── Behaviour ──────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Behaviour",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Keep playing when the window is closed",
                            },
                            gtk::Label {
                                add_css_class: "label-small-dim",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Closing the window hides Tune to the tray instead of quitting.",
                            },
                        },
                        #[name = "tray_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Start hidden in the system tray",
                            },
                            gtk::Label {
                                add_css_class: "label-small-dim",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Launch with no window — just the tray icon. Click it (or bind `mshellctl mtune toggle-window`) to open the player.",
                            },
                        },
                        #[name = "start_hidden_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = MtuneSettingsModel { cfg: load() };
        let widgets = view_output!();

        // on_start dropdown
        let labels: Vec<&str> = ON_START.iter().map(|(_, l)| *l).collect();
        widgets
            .on_start_drop
            .set_model(Some(&gtk::StringList::new(&labels)));
        let sel = ON_START
            .iter()
            .position(|(k, _)| *k == model.cfg.playback.on_start)
            .unwrap_or(0) as u32;
        widgets.on_start_drop.set_selected(sel);
        let s = sender.clone();
        widgets.on_start_drop.connect_selected_notify(move |d| {
            s.input(MtuneSettingsInput::OnStartChanged(d.selected()))
        });

        // tray switch
        widgets
            .tray_switch
            .set_active(model.cfg.behaviour.close_to_tray);
        let s = sender.clone();
        widgets.tray_switch.connect_state_set(move |_, on| {
            s.input(MtuneSettingsInput::CloseToTrayToggled(on));
            glib::Propagation::Proceed
        });

        // start-hidden switch
        widgets
            .start_hidden_switch
            .set_active(model.cfg.behaviour.start_hidden);
        let s = sender.clone();
        widgets.start_hidden_switch.connect_state_set(move |_, on| {
            s.input(MtuneSettingsInput::StartHiddenToggled(on));
            glib::Propagation::Proceed
        });

        rebuild_roots(&widgets.roots_list, &model.cfg.library.roots, &sender);

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
            MtuneSettingsInput::AddRoot => {
                // Parent must be `None`: Settings is a layer-shell surface
                // with no xdg_toplevel, so handing it to the file-chooser
                // as a parent aborts GTK (crashing the shell).
                let dialog = gtk::FileDialog::builder()
                    .title("Add a music folder")
                    .modal(true)
                    .build();
                let s = sender.clone();
                dialog.select_folder(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(folder) = res
                        && let Some(p) = folder.path()
                    {
                        s.input(MtuneSettingsInput::FolderPicked(
                            p.to_string_lossy().into_owned(),
                        ));
                    }
                });
            }
            MtuneSettingsInput::FolderPicked(path) => {
                if !self.cfg.library.roots.contains(&path) {
                    self.cfg.library.roots.push(path);
                    self.persist_roots();
                    rebuild_roots(&widgets.roots_list, &self.cfg.library.roots, &sender);
                }
            }
            MtuneSettingsInput::RemoveRoot(i) => {
                if i < self.cfg.library.roots.len() {
                    self.cfg.library.roots.remove(i);
                    self.persist_roots();
                    rebuild_roots(&widgets.roots_list, &self.cfg.library.roots, &sender);
                }
            }
            MtuneSettingsInput::OnStartChanged(idx) => {
                if let Some((k, _)) = ON_START.get(idx as usize) {
                    self.cfg.playback.on_start = (*k).to_string();
                    save(&self.cfg);
                }
            }
            MtuneSettingsInput::CloseToTrayToggled(on) => {
                self.cfg.behaviour.close_to_tray = on;
                save(&self.cfg);
            }
            MtuneSettingsInput::StartHiddenToggled(on) => {
                self.cfg.behaviour.start_hidden = on;
                save(&self.cfg);
            }
        }
    }
}

impl MtuneSettingsModel {
    /// Write the file and push the roots to a running mtune for a live rescan.
    fn persist_roots(&self) {
        save(&self.cfg);
        let roots = self.cfg.library.roots.clone();
        tokio_rt_spawn(async move {
            mtune_service().player.set_library_roots(roots).await;
        });
    }
}

fn rebuild_roots(list: &gtk::Box, roots: &[String], sender: &ComponentSender<MtuneSettingsModel>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if roots.is_empty() {
        let row = gtk::Label::builder()
            .label("No folders yet — add one below.")
            .css_classes(["label-small-dim"])
            .halign(gtk::Align::Start)
            .build();
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        list.append(&row);
        return;
    }
    for (i, path) in roots.iter().enumerate() {
        let row = gtk::Box::builder()
            .css_classes(["action-row"])
            .spacing(12)
            .build();
        let label = gtk::Label::builder()
            .label(path)
            .hexpand(true)
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .ellipsize(relm4::gtk::pango::EllipsizeMode::Middle)
            .build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["ok-button-flat", "circular"])
            .valign(gtk::Align::Center)
            .tooltip_text("Remove this folder")
            .build();
        let s = sender.clone();
        remove.connect_clicked(move |_| s.input(MtuneSettingsInput::RemoveRoot(i)));
        row.append(&label);
        row.append(&remove);
        list.append(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One typo in mtune.toml must not cost the user their library roots.
    #[test]
    fn unparseable_mtune_toml_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mtune.toml");
        let broken = "[library]\nroots = [\"/music\"\n[behaviour]\nclose_to_tray = false\n";
        std::fs::write(&path, broken).unwrap();
        let mut cfg = load_from(&path);
        cfg.behaviour.close_to_tray = true;
        assert!(save_to(&path, &cfg).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        assert!(dir.path().join("mtune.toml.bak").exists());
    }

    /// Sections and keys this page doesn't know (mtune adds them) survive.
    #[test]
    fn unknown_sections_and_keys_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mtune.toml");
        let body = "volume = 0.5\n\n[library]\nroots = [\"/music\"]\nscan = \"auto\"\n\n[ui]\ntheme = \"dark\"\n";
        std::fs::write(&path, body).unwrap();
        let mut cfg = load_from(&path);
        cfg.behaviour.start_hidden = true;
        save_to(&path, &cfg).unwrap();
        let back: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back["volume"].as_float(), Some(0.5));
        assert_eq!(back["ui"]["theme"].as_str(), Some("dark"));
        assert_eq!(back["library"]["scan"].as_str(), Some("auto"));
        assert_eq!(back["library"]["roots"][0].as_str(), Some("/music"));
        assert_eq!(back["behaviour"]["start_hidden"].as_bool(), Some(true));
    }
}
