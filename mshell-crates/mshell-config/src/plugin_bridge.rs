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
use mshell_plugins::{InstalledPlugin, PluginStore, WidgetDef};

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
        for widget in &plugin.manifest.widgets {
            config
                .bars
                .widgets
                .custom_widgets
                .push(to_custom_widget(&plugin, widget));
        }
    }
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

fn to_custom_widget(plugin: &InstalledPlugin, w: &WidgetDef) -> CustomWidgetConfig {
    // Image paths in a manifest are relative to the plugin's folder.
    let image = if w.image.trim().is_empty() {
        String::new()
    } else {
        plugin.dir.join(&w.image).to_string_lossy().into_owned()
    };
    CustomWidgetConfig {
        name: format!("{PLUGIN_PREFIX}{}:{}", plugin.key, w.key),
        icon: w.icon.clone(),
        image,
        label: w.label.clone(),
        tooltip: w.tooltip.clone(),
        on_click: w.on_click.clone(),
        on_click_right: w.on_click_right.clone(),
        exec: w.exec.clone(),
        template: w.template.clone(),
        interval: w.interval,
        max_chars: w.max_chars,
        menu: w
            .menu
            .iter()
            .map(|r| CustomMenuRow {
                label: r.label.clone(),
                icon: r.icon.clone(),
                exec: r.exec.clone(),
            })
            .collect(),
    }
}
