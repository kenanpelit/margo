//! Plugin input handling for `Frame`: WASM plugin panels, declarative plugin
//! menus, resolving a plugin by key, hot reload and plugin keybinds. Called
//! from `update_with_view`; the panel builders live in `build`.

use super::*;

impl Frame {
    /// Open (building once and caching) a WASM plugin panel in the plugin-panel menu.
    pub(super) fn toggle_wasm_plugin_panel(
        &mut self,
        name: String,
        entry: String,
        settings: String,
        capabilities: String,
        widgets: &mut FrameWidgets,
        root: &<Self as Component>::Root,
    ) {
        self.apply_plugin_layout(&name, widgets);
        #[cfg(feature = "wasm-plugins")]
        {
            use std::collections::hash_map::Entry;
            // Build the panel once per plugin key; reuse it (and its
            // chat state) on later opens.
            if let Entry::Vacant(slot) = self.plugin_panels.entry(name.clone()) {
                let parsed: std::collections::HashMap<String, String> =
                    serde_json::from_str(settings.trim()).unwrap_or_default();
                match mshell_plugin_ui::PluginPanel::new(
                    &self.plugin_panel_runtime,
                    &name,
                    std::path::Path::new(entry.trim()),
                    parsed,
                    &capabilities,
                ) {
                    Ok(panel) => {
                        slot.insert(panel);
                    }
                    Err(e) => tracing::warn!("plugin panel `{name}`: load failed: {e}"),
                }
            }
            if let Some(panel) = self.plugin_panels.get(&name) {
                let content: Widget = panel.widget().clone().upcast();
                self.plugin_panel_menu
                    .sender()
                    .send(MenuInput::SetExternalContent(content))
                    .ok();
            }
            self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
            self.sync_keyboard_mode(root);
        }
        #[cfg(not(feature = "wasm-plugins"))]
        {
            let _ = (name, entry, settings, capabilities, root);
        }
    }

    /// Show a declarative plugin menu (`rows`) in the plugin-panel menu.
    pub(super) fn toggle_plugin_menu(
        &mut self,
        name: String,
        rows: Vec<CustomMenuRow>,
        widgets: &mut FrameWidgets,
        sender: &ComponentSender<Self>,
        root: &<Self as Component>::Root,
    ) {
        self.apply_plugin_layout(&name, widgets);
        let content = Self::build_plugin_menu_content(&rows, sender);
        self.plugin_panel_menu
            .sender()
            .send(MenuInput::SetExternalContent(content))
            .ok();
        self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
        self.sync_keyboard_mode(root);
    }

    pub(super) fn toggle_plugin_by_key(key: String, sender: &ComponentSender<Self>) {
        // Generic `mshellctl menu plugin <key>`: resolve the key to an
        // enabled plugin's derived widget (matching its composite key,
        // widget key, or full name) and dispatch to the panel or menu
        // path. No per-plugin code — any installed plugin works.
        let found = mshell_config::config_manager::config_manager()
            .config()
            .bars()
            .widgets()
            .custom_widgets()
            .get_untracked()
            .into_iter()
            .find(|c| {
                let Some(rest) = c.name.strip_prefix("plugin:") else {
                    return false;
                };
                if c.panel_entry.trim().is_empty() && c.menu.is_empty() {
                    return false;
                }
                let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                rest == key || comp == key || w == key
            });
        match found {
            Some(c) if !c.panel_entry.trim().is_empty() => {
                sender.input(FrameInput::ToggleWasmPluginPanel {
                    name: c.name,
                    entry: c.panel_entry,
                    settings: c.panel_settings,
                    capabilities: c.panel_capabilities,
                    min_width: c.panel_min_width,
                    max_height: c.panel_max_height,
                });
            }
            Some(c) => {
                sender.input(FrameInput::TogglePluginMenu {
                    name: c.name,
                    rows: c.menu,
                    min_width: c.panel_min_width,
                    max_height: c.panel_max_height,
                });
            }
            None => {
                tracing::warn!("menu plugin: no enabled plugin panel/menu for key `{key}`")
            }
        }
    }

    /// Drop every cached panel of plugin `key` so the next open loads it from disk.
    pub(super) fn reload_plugin(&mut self, key: String) {
        #[cfg(feature = "wasm-plugins")]
        {
            // Match any cached panel whose name encodes this key —
            // composite key, widget key, or the full `plugin:<comp>:<w>`.
            let names: Vec<String> = self
                .plugin_panels
                .keys()
                .filter(|name| {
                    let Some(rest) = name.strip_prefix("plugin:") else {
                        return false;
                    };
                    let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                    rest == key || comp == key || w == key
                })
                .cloned()
                .collect();
            for name in names {
                self.plugin_panels.remove(&name);
            }
        }
        #[cfg(not(feature = "wasm-plugins"))]
        let _ = key;
    }

    /// Deliver plugin keybind `bind_id` to plugin `key`'s panel and open it.
    pub(super) fn fire_plugin_keybind(
        &mut self,
        key: String,
        bind_id: String,
        widgets: &mut FrameWidgets,
        root: &<Self as Component>::Root,
    ) {
        #[cfg(feature = "wasm-plugins")]
        {
            use std::collections::hash_map::Entry;
            // Resolve the key against the synthesised plugin widgets
            // the same way TogglePluginByKey does — accept the
            // composite key, the widget key, or the full name.
            let found = mshell_config::config_manager::config_manager()
                .config()
                .bars()
                .widgets()
                .custom_widgets()
                .get_untracked()
                .into_iter()
                .find(|c| {
                    let Some(rest) = c.name.strip_prefix("plugin:") else {
                        return false;
                    };
                    if c.panel_entry.trim().is_empty() {
                        return false;
                    }
                    let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                    rest == key || comp == key || w == key
                });
            if let Some(c) = found {
                self.apply_plugin_layout(&c.name, widgets);
                // Build/get the cached panel.
                if let Entry::Vacant(slot) = self.plugin_panels.entry(c.name.clone()) {
                    let parsed: std::collections::HashMap<String, String> =
                        serde_json::from_str(c.panel_settings.trim()).unwrap_or_default();
                    match mshell_plugin_ui::PluginPanel::new(
                        &self.plugin_panel_runtime,
                        &c.name,
                        std::path::Path::new(c.panel_entry.trim()),
                        parsed,
                        &c.panel_capabilities,
                    ) {
                        Ok(panel) => {
                            slot.insert(panel);
                        }
                        Err(e) => tracing::warn!(
                            "plugin keybind `{key}/{bind_id}`: panel load failed: {e}"
                        ),
                    }
                }
                if let Some(panel) = self.plugin_panels.get(&c.name) {
                    // Deliver the keybind event so the guest can react.
                    panel.fire_event(mshell_plugin_ui::UiEvent {
                        id: bind_id,
                        kind: mshell_plugin_ui::UiEventKind::Keybind,
                        value: String::new(),
                    });
                    // Mount + open the panel surface.
                    let content: Widget = panel.widget().clone().upcast();
                    self.plugin_panel_menu
                        .sender()
                        .send(MenuInput::SetExternalContent(content))
                        .ok();
                    self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
                    self.sync_keyboard_mode(root);
                }
            } else {
                tracing::warn!("plugin keybind: no enabled plugin matches `{key}`");
            }
        }
        #[cfg(not(feature = "wasm-plugins"))]
        let _ = (key, bind_id, widgets, root);
    }
}
