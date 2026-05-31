//! Bridge between installed mplugins and the shell's custom-widget engine.
//!
//! Plugin widgets are a *derived* config layer: they're computed from the
//! plugin manager's own files (`plugins.toml` + installed manifests), never
//! authored in the user's profile. So they're **added on load**
//! ([`resync_plugin_widgets`]) and **stripped before persist**
//! ([`strip_plugin_widgets`]) — the profile YAML stays the user's alone,
//! while the runtime config carries each enabled plugin's widgets named
//! `plugin:<key>:<widget-key>` so the bar can place them like any custom
//! widget.

use crate::schema::config::{Config, CustomMenuRow, CustomWidgetConfig};
use mshell_plugins::{
    InstalledPlugin, PanelLayout, PluginStore, PluginsState, WidgetDef, substitute,
};
use std::collections::BTreeMap;

/// Prefix marking a custom widget as plugin-derived (not user-authored).
pub const PLUGIN_PREFIX: &str = "plugin:";

/// Replace all plugin-derived custom widgets with the current set from the
/// enabled, installed plugins. Idempotent — safe to call on every load.
pub fn resync_plugin_widgets(config: &mut Config) {
    strip_plugin_widgets(config);

    let store = PluginStore::new();
    let state = store.load_state();
    for plugin in store.installed() {
        if !state.is_enabled(&plugin.key) {
            continue;
        }
        let values = setting_values(&plugin, &state);
        let layout = state.panel(&plugin.key);
        for widget in &plugin.manifest.widgets {
            config
                .bars
                .widgets
                .custom_widgets
                .push(to_custom_widget(&plugin, widget, &values, &layout));
        }
    }
}

/// The effective setting values for a plugin: the user's stored value, or the
/// manifest default, per declared setting — plus built-in placeholders. For
/// settings marked `type = "secret"` the value comes from the system keyring
/// (Secret Service), never from `plugins.toml`.
fn setting_values(plugin: &InstalledPlugin, state: &PluginsState) -> BTreeMap<String, String> {
    let mut values: BTreeMap<String, String> = plugin
        .manifest
        .settings
        .iter()
        .map(|s| {
            let v = if s.is_secret() {
                mshell_plugins::secrets::read(&plugin.key, &s.key)
                    .unwrap_or_else(|| s.default.clone())
            } else {
                state
                    .setting(&plugin.key, &s.key)
                    .cloned()
                    .unwrap_or_else(|| s.default.clone())
            };
            (s.key.clone(), v)
        })
        .collect();
    // Built-in: the plugin's install directory, so commands can run bundled
    // scripts/assets — e.g. `sh {{plugin_dir}}/chat.sh`. Inserted last so it
    // wins over any same-named user setting.
    values.insert(
        "plugin_dir".to_string(),
        plugin.dir.to_string_lossy().into_owned(),
    );
    values
}

/// Drop every plugin-derived custom widget (used before persisting a layer
/// so the user's profile never accumulates derived entries).
pub fn strip_plugin_widgets(config: &mut Config) {
    config
        .bars
        .widgets
        .custom_widgets
        .retain(|c| !c.name.starts_with(PLUGIN_PREFIX));
}

fn to_custom_widget(
    plugin: &InstalledPlugin,
    w: &WidgetDef,
    values: &BTreeMap<String, String>,
    layout: &PanelLayout,
) -> CustomWidgetConfig {
    // Image paths in a manifest are relative to the plugin's folder.
    let image = if w.image.trim().is_empty() {
        String::new()
    } else {
        plugin.dir.join(&w.image).to_string_lossy().into_owned()
    };
    let sub = |s: &str| substitute(s, values);
    // A widget that opens the plugin's WASM panel carries the absolute path to
    // the compiled component + the resolved settings (for `get-setting`); the
    // shell's custom pill opens an in-shell panel instead of running a command.
    let (panel_entry, panel_settings) = if w.opens_panel && plugin.manifest.has_wasm_entry() {
        let path = plugin
            .dir
            .join(&plugin.manifest.entry)
            .to_string_lossy()
            .into_owned();
        let settings = serde_json::to_string(values).unwrap_or_default();
        (path, settings)
    } else {
        (String::new(), String::new())
    };
    CustomWidgetConfig {
        name: format!("{PLUGIN_PREFIX}{}:{}", plugin.key, w.key),
        icon: w.icon.clone(),
        image,
        label: sub(&w.label),
        tooltip: sub(&w.tooltip),
        on_click: sub(&w.on_click),
        on_click_right: sub(&w.on_click_right),
        exec: sub(&w.exec),
        template: sub(&w.template),
        interval: w.interval,
        max_chars: w.max_chars,
        art: w.art,
        menu: w
            .menu
            .iter()
            .map(|r| CustomMenuRow {
                label: sub(&r.label),
                icon: r.icon.clone(),
                exec: sub(&r.exec),
                severity: r.severity.clone(),
            })
            .collect(),
        panel_entry,
        panel_settings,
        panel_min_width: layout.min_width,
        panel_max_height: layout.max_height,
    }
}
