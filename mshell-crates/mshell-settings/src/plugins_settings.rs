//! Settings → Widgets → Plugins.
//!
//! The mplugins manager UI: manage *sources* (git repos), browse the plugins
//! they offer, install / enable / disable / uninstall them. Enabling a plugin
//! feeds its declarative widgets into the shell's custom-widget set (handled
//! by `mshell_config::plugin_bridge` on config reload), so they become
//! placeable in bars as `plugin:<key>:<widget>`.
//!
//! Git work (registry fetch, install) is blocking, so it runs on a tokio
//! blocking task and reports back to the GTK main loop via a oneshot.

use mshell_config::config_manager::config_manager;
use mshell_plugins::{InstalledPlugin, PluginStore, PluginsState, Registry, RegistryEntry, Source};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct PluginsSettingsModel {
    store: PluginStore,
    state: PluginsState,
    installed: Vec<InstalledPlugin>,
    available: Vec<AvailableRow>,
    busy: bool,
    status: String,
}

#[derive(Clone)]
struct AvailableRow {
    source_url: String,
    entry: RegistryEntry,
}

#[derive(Debug)]
pub(crate) enum PluginsSettingsInput {
    AddSource,
    RemoveSource(String),
    Refresh,
    Install(String, RegistryEntry),
    ToggleEnabled(String, bool),
    Uninstall(String),
    RegistriesFetched(Vec<(Source, Result<Registry, String>)>),
    Installed(Result<String, String>),
    /// Re-read local state + installed list and repaint.
    ReloadLocal,
}

#[derive(Debug)]
pub(crate) enum PluginsSettingsOutput {}

pub(crate) struct PluginsSettingsInit {}

#[derive(Debug)]
pub(crate) enum PluginsSettingsCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for PluginsSettingsModel {
    type CommandOutput = PluginsSettingsCommandOutput;
    type Input = PluginsSettingsInput;
    type Output = PluginsSettingsOutput;
    type Init = PluginsSettingsInit;

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
                set_hexpand: true,
                set_spacing: 16,

                // ── Hero ──
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("application-x-addon-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Plugins",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Install widgets from external git repositories. Plugins run shell commands with your privileges — review a plugin's commands before enabling it, and only add sources you trust.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Sources ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Sources",
                    set_halign: gtk::Align::Start,
                },
                #[name = "sources_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[name = "source_name_entry"]
                    gtk::Entry {
                        set_placeholder_text: Some("Name"),
                        set_width_chars: 12,
                    },
                    #[name = "source_url_entry"]
                    gtk::Entry {
                        set_placeholder_text: Some("https://github.com/user/repo"),
                        set_hexpand: true,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface"],
                        set_label: "Add",
                        connect_clicked[sender] => move |_| sender.input(PluginsSettingsInput::AddSource),
                    },
                },

                // ── Available ──
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_margin_top: 12,
                    gtk::Label {
                        add_css_class: "label-large-bold",
                        set_label: "Available",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    #[name = "status_label"]
                    gtk::Label {
                        add_css_class: "label-small",
                        add_css_class: "dim-label",
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface"],
                        set_label: "Refresh",
                        connect_clicked[sender] => move |_| sender.input(PluginsSettingsInput::Refresh),
                    },
                },
                #[name = "available_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },

                // ── Installed ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Installed",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                #[name = "installed_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let store = PluginStore::new();
        let state = store.load_state();
        let installed = store.installed();
        let model = PluginsSettingsModel {
            store,
            state,
            installed,
            available: Vec::new(),
            busy: false,
            status: String::new(),
        };
        let widgets = view_output!();

        rebuild_sources(&widgets.sources_list, &model.state.sources, &sender);
        rebuild_installed(&widgets.installed_list, &model, &sender);
        rebuild_available(&widgets.available_list, &model, &sender);

        // Re-read local state + installed list whenever the page is shown,
        // in case plugins.toml changed elsewhere (e.g. the CLI, later).
        {
            let s = sender.clone();
            root.connect_map(move |_| s.input(PluginsSettingsInput::ReloadLocal));
        }

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
            PluginsSettingsInput::AddSource => {
                let name = widgets.source_name_entry.text().trim().to_string();
                let url = widgets.source_url_entry.text().trim().to_string();
                if url.is_empty() {
                    self.status = "Enter a source URL.".into();
                } else {
                    let name = if name.is_empty() { url.clone() } else { name };
                    self.state.ensure_source(&name, &url);
                    let _ = self.store.save_state(&self.state);
                    widgets.source_name_entry.set_text("");
                    widgets.source_url_entry.set_text("");
                    self.status = format!("Added source “{name}”. Hit Refresh.");
                }
            }
            PluginsSettingsInput::RemoveSource(url) => {
                self.state.sources.retain(|s| s.url != url);
                self.available.retain(|a| a.source_url != url);
                let _ = self.store.save_state(&self.state);
            }
            PluginsSettingsInput::Refresh => {
                if !self.busy {
                    self.busy = true;
                    self.status = "Fetching registries…".into();
                    let store = self.store.clone();
                    let sources = self.state.sources.clone();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        let res = tokio::task::spawn_blocking(move || {
                            sources
                                .into_iter()
                                .map(|s| {
                                    let r = store.fetch_registry(&s.url).map_err(|e| e.to_string());
                                    (s, r)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await
                        .unwrap_or_default();
                        let _ = tx.send(res);
                    });
                    let s2 = sender.clone();
                    relm4::gtk::glib::spawn_future_local(async move {
                        if let Ok(res) = rx.await {
                            s2.input(PluginsSettingsInput::RegistriesFetched(res));
                        }
                    });
                }
            }
            PluginsSettingsInput::RegistriesFetched(results) => {
                self.busy = false;
                let mut rows = Vec::new();
                let mut errors = 0usize;
                for (source, res) in results {
                    match res {
                        Ok(reg) => {
                            for entry in reg.plugins {
                                rows.push(AvailableRow {
                                    source_url: source.url.clone(),
                                    entry,
                                });
                            }
                        }
                        Err(_) => errors += 1,
                    }
                }
                let n = rows.len();
                self.available = rows;
                self.status = if errors > 0 {
                    format!("{n} plugins · {errors} source(s) failed")
                } else {
                    format!("{n} plugins available")
                };
            }
            PluginsSettingsInput::Install(url, entry) => {
                if !self.busy {
                    self.busy = true;
                    self.status = format!("Installing {}…", entry.id);
                    let store = self.store.clone();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        let res = tokio::task::spawn_blocking(move || {
                            store.install(&url, &entry).map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                        let _ = tx.send(res);
                    });
                    let s2 = sender.clone();
                    relm4::gtk::glib::spawn_future_local(async move {
                        if let Ok(res) = rx.await {
                            s2.input(PluginsSettingsInput::Installed(res));
                        }
                    });
                }
            }
            PluginsSettingsInput::Installed(res) => {
                self.busy = false;
                match res {
                    Ok(key) => {
                        self.status = format!("Installed {key}. Enable it below.");
                        self.installed = self.store.installed();
                    }
                    Err(e) => self.status = format!("Install failed: {e}"),
                }
            }
            PluginsSettingsInput::ToggleEnabled(key, on) => {
                self.state.set_enabled(&key, on);
                let _ = self.store.save_state(&self.state);
                // Re-derive plugin widgets into the live config.
                config_manager().reload_config();
            }
            PluginsSettingsInput::Uninstall(key) => {
                let _ = self.store.uninstall(&key);
                self.state.set_enabled(&key, false);
                let _ = self.store.save_state(&self.state);
                self.installed = self.store.installed();
                config_manager().reload_config();
                self.status = format!("Removed {key}.");
            }
            PluginsSettingsInput::ReloadLocal => {
                self.state = self.store.load_state();
                self.installed = self.store.installed();
            }
        }

        // Repaint everything that could have changed + the status line.
        widgets.status_label.set_label(&self.status);
        rebuild_sources(&widgets.sources_list, &self.state.sources, &sender);
        rebuild_installed(&widgets.installed_list, self, &sender);
        rebuild_available(&widgets.available_list, self, &sender);
    }
}

// ── Row builders ────────────────────────────────────────────────────────────

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn rebuild_sources(list: &gtk::Box, sources: &[Source], sender: &ComponentSender<PluginsSettingsModel>) {
    clear(list);
    for s in sources {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("settings-row");
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.set_hexpand(true);
        let name = gtk::Label::new(Some(&s.name));
        name.add_css_class("label-medium-bold");
        name.set_halign(gtk::Align::Start);
        let url = gtk::Label::new(Some(&s.url));
        url.add_css_class("label-small");
        url.add_css_class("dim-label");
        url.set_halign(gtk::Align::Start);
        url.set_xalign(0.0);
        url.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        col.append(&name);
        col.append(&url);
        row.append(&col);

        let is_official = s.url == mshell_plugins::OFFICIAL_SOURCE;
        if !is_official {
            let btn = gtk::Button::from_icon_name("user-trash-symbolic");
            btn.add_css_class("panel-action-btn");
            btn.set_valign(gtk::Align::Center);
            btn.set_tooltip_text(Some("Remove source"));
            let s2 = sender.clone();
            let url = s.url.clone();
            btn.connect_clicked(move |_| s2.input(PluginsSettingsInput::RemoveSource(url.clone())));
            row.append(&btn);
        }
        list.append(&row);
    }
}

fn rebuild_installed(
    list: &gtk::Box,
    model: &PluginsSettingsModel,
    sender: &ComponentSender<PluginsSettingsModel>,
) {
    clear(list);
    if model.installed.is_empty() {
        let empty = gtk::Label::new(Some("No plugins installed yet."));
        empty.add_css_class("label-small");
        empty.add_css_class("dim-label");
        empty.set_halign(gtk::Align::Start);
        list.append(&empty);
        return;
    }
    for p in &model.installed {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("settings-row");

        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_hexpand(true);
        let title = gtk::Label::new(Some(&format!(
            "{}  ·  v{}",
            if p.manifest.name.is_empty() { &p.manifest.id } else { &p.manifest.name },
            p.manifest.version
        )));
        title.add_css_class("label-medium-bold");
        title.set_halign(gtk::Align::Start);
        col.append(&title);

        // Show the commands the plugin would run, so the user can review
        // them before enabling.
        let cmds = command_summary(p);
        if !cmds.is_empty() {
            let cmd = gtk::Label::new(Some(&cmds));
            cmd.add_css_class("label-small");
            cmd.add_css_class("dim-label");
            cmd.set_halign(gtk::Align::Start);
            cmd.set_xalign(0.0);
            // Commands can be long; keep the row compact with one ellipsized
            // line and the full text on hover (still selectable for review).
            cmd.set_ellipsize(gtk::pango::EllipsizeMode::End);
            cmd.set_max_width_chars(48);
            cmd.set_tooltip_text(Some(&cmds));
            cmd.set_selectable(true);
            col.append(&cmd);
        }
        row.append(&col);

        let enabled = model.state.is_enabled(&p.key);
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        sw.set_active(enabled);
        sw.set_tooltip_text(Some("Enable / disable"));
        {
            let s2 = sender.clone();
            let key = p.key.clone();
            sw.connect_active_notify(move |s| {
                s2.input(PluginsSettingsInput::ToggleEnabled(key.clone(), s.is_active()))
            });
        }
        row.append(&sw);

        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("panel-action-btn");
        del.set_valign(gtk::Align::Center);
        del.set_tooltip_text(Some("Uninstall"));
        {
            let s2 = sender.clone();
            let key = p.key.clone();
            del.connect_clicked(move |_| s2.input(PluginsSettingsInput::Uninstall(key.clone())));
        }
        row.append(&del);

        list.append(&row);
    }
}

fn rebuild_available(
    list: &gtk::Box,
    model: &PluginsSettingsModel,
    sender: &ComponentSender<PluginsSettingsModel>,
) {
    clear(list);
    let installed_keys: Vec<&str> = model.installed.iter().map(|p| p.key.as_str()).collect();
    let mut shown = 0;
    for row_data in &model.available {
        let key = model.store.key_for(&row_data.entry.id, &row_data.source_url);
        if installed_keys.contains(&key.as_str()) {
            continue; // already installed
        }
        shown += 1;
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("settings-row");

        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_hexpand(true);
        let e = &row_data.entry;
        let title = gtk::Label::new(Some(&format!(
            "{}  ·  v{}",
            if e.name.is_empty() { &e.id } else { &e.name },
            e.version
        )));
        title.add_css_class("label-medium-bold");
        title.set_halign(gtk::Align::Start);
        col.append(&title);
        if !e.description.is_empty() {
            let desc = gtk::Label::new(Some(&e.description));
            desc.add_css_class("label-small");
            desc.add_css_class("dim-label");
            desc.set_halign(gtk::Align::Start);
            desc.set_xalign(0.0);
            desc.set_wrap(true);
            col.append(&desc);
        }
        row.append(&col);

        let btn = gtk::Button::with_label("Install");
        btn.add_css_class("ok-button-surface");
        btn.set_valign(gtk::Align::Center);
        btn.set_sensitive(!model.busy);
        {
            let s2 = sender.clone();
            let url = row_data.source_url.clone();
            let entry = row_data.entry.clone();
            btn.connect_clicked(move |_| {
                s2.input(PluginsSettingsInput::Install(url.clone(), entry.clone()))
            });
        }
        row.append(&btn);
        list.append(&row);
    }
    if shown == 0 {
        let msg = if model.available.is_empty() {
            "Hit Refresh to fetch plugins from your sources."
        } else {
            "All available plugins are installed."
        };
        let empty = gtk::Label::new(Some(msg));
        empty.add_css_class("label-small");
        empty.add_css_class("dim-label");
        empty.set_halign(gtk::Align::Start);
        list.append(&empty);
    }
}

/// One-line summary of the shell commands a plugin's widgets declare.
fn command_summary(p: &InstalledPlugin) -> String {
    let mut cmds = Vec::new();
    for w in &p.manifest.widgets {
        for c in [&w.exec, &w.on_click, &w.on_click_right] {
            let c = c.trim();
            if !c.is_empty() {
                cmds.push(c.to_string());
            }
        }
    }
    if cmds.is_empty() {
        String::new()
    } else {
        format!("runs: {}", cmds.join(" · "))
    }
}
