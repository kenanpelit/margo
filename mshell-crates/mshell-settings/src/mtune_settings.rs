//! Settings → Tune — the folder-first music player (`mtune`).
//!
//! Edits `~/.config/margo/mtune.toml` directly (mtune's own config world,
//! like `mpower.toml` — NOT `config_manager()`). Library-root changes are
//! also pushed to a running mtune over `org.margo.Tune::SetLibraryRoots`
//! so it rescans immediately; the other knobs take effect on mtune's next
//! launch.

use mshell_services::mtune::mtune_service;
use mshell_services::tokio_rt_spawn;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── mtune.toml mirror (only the fields this page edits are named; the
//    rest round-trip through `#[serde(flatten)]` so a hand-edited file
//    keeps its other keys). ────────────────────────────────────────────

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct MtuneToml {
    #[serde(default)]
    library: LibrarySection,
    #[serde(default)]
    playback: PlaybackSection,
    #[serde(default)]
    behaviour: BehaviourSection,
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
    #[serde(flatten)]
    rest: toml::Table,
}
impl Default for BehaviourSection {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            rest: toml::Table::new(),
        }
    }
}
fn default_true() -> bool {
    true
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("margo").join("mtune.toml")
}

fn load() -> MtuneToml {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(cfg: &MtuneToml) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match toml::to_string_pretty(cfg) {
        Ok(body) => {
            let tmp = path.with_extension("toml.tmp");
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        Err(e) => tracing::warn!(error = %e, "mtune-settings: serialize failed"),
    }
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
                        set_icon_name: Some("folder-music-symbolic"),
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

        rebuild_roots(&widgets.roots_list, &model.cfg.library.roots, &sender);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            MtuneSettingsInput::AddRoot => {
                let dialog = gtk::FileDialog::builder()
                    .title("Add a music folder")
                    .build();
                let parent: Option<gtk::Window> = root.root().and_downcast();
                let s = sender.clone();
                dialog.select_folder(parent.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
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
