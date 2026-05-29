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
use mshell_plugins::{
    InstalledPlugin, PanelLayout, PluginStore, PluginsState, Registry, RegistryEntry, Source,
};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct PluginsSettingsModel {
    store: PluginStore,
    state: PluginsState,
    installed: Vec<InstalledPlugin>,
    available: Vec<AvailableRow>,
    busy: bool,
    status: String,
    /// Composite key of the installed plugin whose settings form is open.
    expanded_settings: Option<String>,
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
    /// Open / close a plugin's inline settings form.
    ToggleSettings(String),
    /// Persist one setting value (and re-template the live widget).
    SetSetting {
        plugin: String,
        key: String,
        value: String,
    },
    /// Persist a plugin's panel size (its own preference, not the global Menus
    /// page) and re-derive the live widget.
    SetPanelSize {
        plugin: String,
        min_width: i32,
        max_height: i32,
    },
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
            expanded_settings: None,
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
                        self.installed = self.store.installed();
                        // On an update of an already-enabled plugin, re-derive
                        // its widgets so the new version takes effect live.
                        if self.state.is_enabled(&key) {
                            config_manager().reload_config();
                        }
                        self.status = format!("{key} installed.");
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
                self.state.forget(&key);
                if self.expanded_settings.as_deref() == Some(key.as_str()) {
                    self.expanded_settings = None;
                }
                let _ = self.store.save_state(&self.state);
                self.installed = self.store.installed();
                config_manager().reload_config();
                self.status = format!("Removed {key}.");
            }
            PluginsSettingsInput::ToggleSettings(key) => {
                self.expanded_settings = if self.expanded_settings.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
            }
            PluginsSettingsInput::SetSetting { plugin, key, value } => {
                self.state.set_setting(&plugin, &key, &value);
                let _ = self.store.save_state(&self.state);
                // Re-template the live widget if the plugin is enabled.
                if self.state.is_enabled(&plugin) {
                    config_manager().reload_config();
                }
                // Don't rebuild the list here — that would tear down the open
                // form mid-edit. The controls already hold the new value.
                return;
            }
            PluginsSettingsInput::SetPanelSize {
                plugin,
                min_width,
                max_height,
            } => {
                let mut layout = self.state.panel(&plugin);
                layout.min_width = min_width;
                layout.max_height = max_height;
                self.state.set_panel(&plugin, layout);
                let _ = self.store.save_state(&self.state);
                if self.state.is_enabled(&plugin) {
                    config_manager().reload_config();
                }
                return;
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

const FALLBACK_ICON: &str = "application-x-addon-symbolic";

/// A card row: leading icon + a hexpanding text column. Caller appends
/// trailing controls and the row to the list.
fn card_row(icon: &str) -> (gtk::Box, gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("plugins-row");
    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class("plugins-row-icon");
    img.set_valign(gtk::Align::Center);
    row.append(&img);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    (row, col)
}

/// Title line: bold name + a small version badge.
fn title_line(name: &str, version: &str) -> gtk::Box {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(name));
    label.add_css_class("label-medium-bold");
    label.set_halign(gtk::Align::Start);
    head.append(&label);
    if !version.trim().is_empty() {
        let badge = gtk::Label::new(Some(&format!("v{version}")));
        badge.add_css_class("plugins-version");
        badge.set_valign(gtk::Align::Center);
        head.append(&badge);
    }
    head
}

fn dim_line(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-small");
    l.add_css_class("dim-label");
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    l
}

fn empty_hint(list: &gtk::Box, text: &str) {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-small");
    l.add_css_class("dim-label");
    l.add_css_class("plugins-empty");
    l.set_halign(gtk::Align::Start);
    list.append(&l);
}

/// The newer registry entry for an installed plugin, if any source offers one.
fn update_for<'a>(
    model: &'a PluginsSettingsModel,
    p: &InstalledPlugin,
) -> Option<&'a AvailableRow> {
    model.available.iter().find(|a| {
        model.store.key_for(&a.entry.id, &a.source_url) == p.key
            && mshell_plugins::is_newer(&a.entry.version, &p.manifest.version)
    })
}

fn installed_icon(p: &InstalledPlugin) -> String {
    p.manifest
        .widgets
        .iter()
        .map(|w| w.icon.trim())
        .find(|i| !i.is_empty())
        .unwrap_or(FALLBACK_ICON)
        .to_string()
}

fn rebuild_sources(list: &gtk::Box, sources: &[Source], sender: &ComponentSender<PluginsSettingsModel>) {
    clear(list);
    for s in sources {
        let (row, col) = card_row("network-server-symbolic");
        let name = gtk::Label::new(Some(&s.name));
        name.add_css_class("label-medium-bold");
        name.set_halign(gtk::Align::Start);
        let url = dim_line(&s.url);
        url.set_wrap(false);
        url.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        col.append(&name);
        col.append(&url);
        row.append(&col);

        if s.url != mshell_plugins::OFFICIAL_SOURCE {
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
        empty_hint(list, "No plugins installed yet.");
        return;
    }
    for p in &model.installed {
        let (row, col) = card_row(&installed_icon(p));
        let name = if p.manifest.name.is_empty() { &p.manifest.id } else { &p.manifest.name };
        col.append(&title_line(name, &p.manifest.version));

        if !p.manifest.description.trim().is_empty() {
            col.append(&dim_line(p.manifest.description.trim()));
        }

        // Trust gate: a quiet hint that it runs commands, full text in tooltip.
        let cmds = command_summary(p);
        if !cmds.is_empty() {
            let hint = dim_line("runs shell commands — hover to review");
            hint.set_tooltip_text(Some(&cmds));
            col.append(&hint);
        }
        row.append(&col);

        // Update (when a source offers a newer version).
        if let Some(av) = update_for(model, p) {
            let btn = gtk::Button::with_label("Update");
            btn.add_css_class("ok-button-surface");
            btn.add_css_class("plugins-update-btn");
            btn.set_valign(gtk::Align::Center);
            btn.set_sensitive(!model.busy);
            btn.set_tooltip_text(Some(&format!("Update to v{}", av.entry.version)));
            let s2 = sender.clone();
            let url = av.source_url.clone();
            let entry = av.entry.clone();
            btn.connect_clicked(move |_| {
                s2.input(PluginsSettingsInput::Install(url.clone(), entry.clone()))
            });
            row.append(&btn);
        }

        // Settings gear — when the plugin declares settings, or ships a
        // panel/menu whose size lives here (under the plugin, not Menus).
        if !p.manifest.settings.is_empty() || plugin_has_panel(&p.manifest) {
            let gear = gtk::Button::from_icon_name("emblem-system-symbolic");
            gear.add_css_class("panel-action-btn");
            gear.set_valign(gtk::Align::Center);
            gear.set_tooltip_text(Some("Settings"));
            let s2 = sender.clone();
            let key = p.key.clone();
            gear.connect_clicked(move |_| {
                s2.input(PluginsSettingsInput::ToggleSettings(key.clone()))
            });
            row.append(&gear);
        }

        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        sw.set_active(model.state.is_enabled(&p.key));
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

        // Inline settings form, when this plugin's gear is toggled open.
        if model.expanded_settings.as_deref() == Some(p.key.as_str()) {
            list.append(&build_settings_form(p, &model.state, sender));
        }
    }
}

/// The inline settings card for a plugin: one control per declared setting,
/// pre-filled from the stored value (or the manifest default).
fn build_settings_form(
    p: &InstalledPlugin,
    state: &PluginsState,
    sender: &ComponentSender<PluginsSettingsModel>,
) -> gtk::Box {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.add_css_class("plugins-settings-form");

    for s in &p.manifest.settings {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("plugins-setting-row");

        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.set_hexpand(true);
        col.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(Some(if s.label.is_empty() { &s.key } else { &s.label }));
        label.add_css_class("label-medium-bold");
        label.set_halign(gtk::Align::Start);
        col.append(&label);
        if !s.description.trim().is_empty() {
            col.append(&dim_line(s.description.trim()));
        }
        row.append(&col);

        let current = state
            .setting(&p.key, &s.key)
            .cloned()
            .unwrap_or_else(|| s.default.clone());
        let plugin_key = p.key.clone();
        let setting_key = s.key.clone();
        let control = setting_control(&s.kind, &s.choices, &current, sender, plugin_key, setting_key);
        row.append(&control);

        form.append(&row);
    }

    // A plugin that ships a panel/menu carries its own surface size here — its
    // settings, not the global Menus page (keeps a plugin self-contained).
    if plugin_has_panel(&p.manifest) {
        form.append(&panel_size_section(&p.key, &state.panel(&p.key), sender));
    }
    form
}

/// `true` if the plugin contributes an in-shell surface (a WASM panel or a
/// declarative `[[widget.menu]]`) whose size is worth configuring.
fn plugin_has_panel(m: &mshell_plugins::Manifest) -> bool {
    m.has_wasm_entry() || m.widgets.iter().any(|w| w.opens_panel || !w.menu.is_empty())
}

/// The "Panel Size" controls (min width + max height) for a plugin's surface,
/// pre-filled from its stored layout and emitting `SetPanelSize` on change.
fn panel_size_section(
    plugin: &str,
    layout: &PanelLayout,
    sender: &ComponentSender<PluginsSettingsModel>,
) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
    section.add_css_class("plugins-setting-row");
    let title = gtk::Label::new(Some("Panel Size"));
    title.add_css_class("label-medium-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);
    section.append(&dim_line("Size of this plugin's in-shell surface."));

    let min_spin = gtk::SpinButton::with_range(200.0, 1200.0, 10.0);
    min_spin.set_value(layout.min_width.max(200) as f64);
    let max_spin = gtk::SpinButton::with_range(0.0, 1400.0, 10.0);
    max_spin.set_value(layout.max_height.max(0) as f64);

    // Wire AFTER set_value so the initial fill doesn't emit. Each control sends
    // both current values (SetPanelSize carries the pair).
    {
        let sender = sender.clone();
        let plugin = plugin.to_string();
        let max_w = max_spin.clone();
        min_spin.connect_value_changed(move |w| {
            sender.input(PluginsSettingsInput::SetPanelSize {
                plugin: plugin.clone(),
                min_width: w.value() as i32,
                max_height: max_w.value() as i32,
            });
        });
    }
    {
        let sender = sender.clone();
        let plugin = plugin.to_string();
        let min_w = min_spin.clone();
        max_spin.connect_value_changed(move |w| {
            sender.input(PluginsSettingsInput::SetPanelSize {
                plugin: plugin.clone(),
                min_width: min_w.value() as i32,
                max_height: w.value() as i32,
            });
        });
    }

    section.append(&spin_row("Min width", &min_spin));
    section.append(&spin_row("Max height (0 = no cap)", &max_spin));
    section
}

/// A label + a trailing spin control, on one row.
fn spin_row(label: &str, spin: &gtk::SpinButton) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let lbl = gtk::Label::new(Some(label));
    lbl.set_halign(gtk::Align::Start);
    lbl.set_hexpand(true);
    row.append(&lbl);
    spin.set_valign(gtk::Align::Center);
    row.append(spin);
    row
}

/// Build the right-hand control for one setting, wired to emit `SetSetting`.
fn setting_control(
    kind: &str,
    choices: &[String],
    current: &str,
    sender: &ComponentSender<PluginsSettingsModel>,
    plugin: String,
    key: String,
) -> gtk::Widget {
    match kind {
        "bool" => {
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_active(current == "true");
            let s2 = sender.clone();
            sw.connect_active_notify(move |w| {
                s2.input(PluginsSettingsInput::SetSetting {
                    plugin: plugin.clone(),
                    key: key.clone(),
                    value: if w.is_active() { "true" } else { "false" }.into(),
                });
            });
            sw.upcast()
        }
        "choice" => {
            let strs: Vec<&str> = choices.iter().map(|c| c.as_str()).collect();
            let dd = gtk::DropDown::from_strings(&strs);
            dd.set_valign(gtk::Align::Center);
            if let Some(i) = choices.iter().position(|c| c == current) {
                dd.set_selected(i as u32);
            }
            let s2 = sender.clone();
            let choices = choices.to_vec();
            dd.connect_selected_notify(move |w| {
                if let Some(v) = choices.get(w.selected() as usize) {
                    s2.input(PluginsSettingsInput::SetSetting {
                        plugin: plugin.clone(),
                        key: key.clone(),
                        value: v.clone(),
                    });
                }
            });
            dd.upcast()
        }
        other => {
            let entry = gtk::Entry::new();
            entry.set_valign(gtk::Align::Center);
            entry.set_hexpand(false);
            entry.set_width_chars(22);
            entry.set_text(current);
            if other == "secret" {
                entry.set_visibility(false);
                entry.set_input_purpose(gtk::InputPurpose::Password);
                entry.set_placeholder_text(Some("•••••"));
            } else if other == "number" {
                entry.set_input_purpose(gtk::InputPurpose::Number);
            }
            entry.set_tooltip_text(Some("Press Enter (or click away) to apply"));
            // Apply on Enter and on focus-leave so a typed value isn't lost.
            let emit = {
                let s2 = sender.clone();
                move |text: String| {
                    s2.input(PluginsSettingsInput::SetSetting {
                        plugin: plugin.clone(),
                        key: key.clone(),
                        value: text,
                    });
                }
            };
            let emit = std::rc::Rc::new(emit);
            {
                let emit = emit.clone();
                entry.connect_activate(move |e| emit(e.text().to_string()));
            }
            {
                let emit = emit.clone();
                let focus = gtk::EventControllerFocus::new();
                let entry_weak = entry.downgrade();
                focus.connect_leave(move |_| {
                    if let Some(e) = entry_weak.upgrade() {
                        emit(e.text().to_string());
                    }
                });
                entry.add_controller(focus);
            }
            entry.upcast()
        }
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
            continue; // already installed (updates live in the Installed list)
        }
        shown += 1;
        let e = &row_data.entry;
        let (row, col) = card_row(FALLBACK_ICON);
        let name = if e.name.is_empty() { &e.id } else { &e.name };
        col.append(&title_line(name, &e.version));
        if !e.description.is_empty() {
            col.append(&dim_line(&e.description));
        }
        row.append(&col);

        if mshell_plugins::compatible(&e.min_mshell) {
            let btn = gtk::Button::with_label("Install");
            btn.add_css_class("ok-button-surface");
            btn.set_valign(gtk::Align::Center);
            btn.set_sensitive(!model.busy);
            let s2 = sender.clone();
            let url = row_data.source_url.clone();
            let entry = row_data.entry.clone();
            btn.connect_clicked(move |_| {
                s2.input(PluginsSettingsInput::Install(url.clone(), entry.clone()))
            });
            row.append(&btn);
        } else {
            let note = dim_line(&format!("needs mshell ≥ {}", e.min_mshell));
            note.set_wrap(false);
            note.set_valign(gtk::Align::Center);
            note.set_tooltip_text(Some(&format!("You have mshell {}", mshell_plugins::MSHELL_VERSION)));
            row.append(&note);
        }
        list.append(&row);
    }
    if shown == 0 {
        let msg = if model.available.is_empty() {
            "Hit Refresh to fetch plugins from your sources."
        } else {
            "All available plugins are installed."
        };
        empty_hint(list, msg);
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
    cmds.join("\n")
}
