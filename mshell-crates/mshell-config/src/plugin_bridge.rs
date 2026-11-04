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

use crate::schema::config::{Config, CustomMenuRow, CustomWidgetConfig, PluginSecretEnv};
use mshell_plugins::{
    InstalledPlugin, PanelLayout, PluginStore, PluginsState, WidgetDef, secret_env_var, substitute,
    substitute_command,
};
use std::collections::BTreeMap;

/// Prefix marking a custom widget as plugin-derived (not user-authored).
pub const PLUGIN_PREFIX: &str = "plugin:";

/// Replace all plugin-derived custom widgets with the current set from the
/// enabled, installed plugins. Idempotent — safe to call on every load.
///
/// Never touches the keyring: secret values come from the
/// `mshell_plugins::secrets` cache. Secrets not read yet are fetched on a
/// background thread, which calls `on_secrets_loaded` (if one was
/// registered) so the caller can resync with the real values.
pub fn resync_plugin_widgets(config: &mut Config) {
    strip_plugin_widgets(config);

    let store = PluginStore::new();
    let state = store.load_state();
    let mut missing = Vec::new();
    for plugin in store.installed() {
        if !state.is_enabled(&plugin.key) {
            continue;
        }
        let values = setting_values(&plugin, &state, &mut missing);
        let layout = state.panel(&plugin.key);
        for widget in &plugin.manifest.widgets {
            config
                .bars
                .widgets
                .custom_widgets
                .push(to_custom_widget(&plugin, widget, &values, &layout));
        }
    }
    prefetch_secrets(missing);
}

/// The effective setting values for a plugin: the user's stored value, or the
/// manifest default, per declared setting — plus built-in placeholders. For
/// settings marked `type = "secret"` the value comes from the system keyring
/// (Secret Service), never from `plugins.toml`.
fn setting_values(
    plugin: &InstalledPlugin,
    state: &PluginsState,
    missing: &mut Vec<(String, String)>,
) -> BTreeMap<String, String> {
    let mut values: BTreeMap<String, String> = plugin
        .manifest
        .settings
        .iter()
        .map(|s| {
            let v = if s.is_secret() {
                // Cache only: this runs on the GTK thread on every config
                // load and save. Unread secrets are fetched off-thread by
                // `prefetch_secrets`, which then resyncs.
                match mshell_plugins::secrets::cached(&plugin.key, &s.key) {
                    Some(v) => v.unwrap_or_else(|| s.default.clone()),
                    None => {
                        missing.push((plugin.key.clone(), s.key.clone()));
                        s.default.clone()
                    }
                }
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

/// Called once a background secret prefetch has filled the cache (the
/// config manager registers a resync here).
static ON_SECRETS_LOADED: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Register what to run after a background secret prefetch.
pub fn on_secrets_loaded(f: fn()) {
    let _ = ON_SECRETS_LOADED.set(f);
}

/// Read `missing` secrets on a background thread (the Secret Service call
/// blocks, and may show an unlock prompt), then run the registered callback.
/// Concurrent requests coalesce: one prefetch at a time.
fn prefetch_secrets(missing: Vec<(String, String)>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if missing.is_empty() || RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("plugin-secrets".into())
        .spawn(move || {
            for (plugin, key) in &missing {
                // `read` caches the value (or "unset").
                let _ = mshell_plugins::secrets::read(plugin, key);
            }
            RUNNING.store(false, Ordering::Release);
            if let Some(f) = ON_SECRETS_LOADED.get() {
                f();
            }
        });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::Release);
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
    // Secrets never go into text: displayed fields get an empty string,
    // commands get an env reference (see `substitute_command`).
    let secrets: Vec<String> = plugin
        .manifest
        .settings
        .iter()
        .filter(|s| s.is_secret())
        .map(|s| s.key.clone())
        .collect();
    let display_values: BTreeMap<String, String> = values
        .iter()
        .map(|(k, v)| {
            let v = if secrets.contains(k) {
                String::new()
            } else {
                v.clone()
            };
            (k.clone(), v)
        })
        .collect();
    let sub = |s: &str| substitute(s, &display_values);
    let cmd = |s: &str| substitute_command(s, values, &secrets);
    let secret_env: Vec<PluginSecretEnv> = secrets
        .iter()
        .map(|key| PluginSecretEnv {
            var: secret_env_var(key),
            plugin: plugin.key.clone(),
            key: key.clone(),
        })
        .collect();
    // A widget that opens the plugin's WASM panel carries the absolute path to
    // the compiled component + the resolved settings (for `get-setting`); the
    // shell's custom pill opens an in-shell panel instead of running a command.
    let (panel_entry, panel_settings, panel_capabilities) =
        if w.opens_panel && plugin.manifest.has_wasm_entry() {
            let path = plugin
                .dir
                .join(&plugin.manifest.entry)
                .to_string_lossy()
                .into_owned();
            let settings = serde_json::to_string(values).unwrap_or_default();
            let capabilities = plugin.manifest.capabilities.to_tokens();
            (path, settings, capabilities)
        } else {
            (String::new(), String::new(), String::new())
        };
    CustomWidgetConfig {
        name: format!("{PLUGIN_PREFIX}{}:{}", plugin.key, w.key),
        icon: w.icon.clone(),
        image,
        label: sub(&w.label),
        tooltip: sub(&w.tooltip),
        on_click: cmd(&w.on_click),
        on_click_right: cmd(&w.on_click_right),
        exec: cmd(&w.exec),
        template: sub(&w.template),
        interval: w.interval,
        max_chars: w.max_chars,
        art: w.art,
        json: false,
        menu: w
            .menu
            .iter()
            .map(|r| CustomMenuRow {
                label: sub(&r.label),
                icon: r.icon.clone(),
                exec: cmd(&r.exec),
                severity: r.severity.clone(),
                secret_env: secret_env.clone(),
            })
            .collect(),
        panel_entry,
        panel_settings,
        panel_capabilities,
        panel_min_width: layout.min_width,
        panel_max_height: layout.max_height,
        secret_env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_plugins::{Manifest, MenuRow, Setting};

    /// A secret setting's value never lands in a command (world-readable
    /// argv of `sh -c`) or a label; commands reference its env var, and the
    /// widget lists what the runner must export.
    #[test]
    fn secrets_never_reach_command_text() {
        let manifest = Manifest {
            id: "assistant".into(),
            version: "1.0.0".into(),
            settings: vec![
                Setting {
                    key: "api_key".into(),
                    kind: "secret".into(),
                    ..Default::default()
                },
                Setting {
                    key: "model".into(),
                    default: "flash".into(),
                    ..Default::default()
                },
            ],
            widgets: vec![WidgetDef {
                key: "chat".into(),
                label: "AI {{api_key}}".into(),
                exec: "curl -H \"Authorization: Bearer {{api_key}}\" x/{{model}}".into(),
                on_click: "env KEY={{api_key}} chat".into(),
                menu: vec![MenuRow {
                    label: "Ask".into(),
                    exec: "ask '{{api_key}}'".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let plugin = InstalledPlugin {
            key: "official/assistant".into(),
            manifest,
            dir: "/plugins/assistant".into(),
        };
        let mut values = BTreeMap::new();
        values.insert("api_key".to_string(), "sk-SECRET".to_string());
        values.insert("model".to_string(), "flash".to_string());

        let w = to_custom_widget(
            &plugin,
            &plugin.manifest.widgets[0],
            &values,
            &PanelLayout::default(),
        );
        for text in [&w.exec, &w.on_click, &w.label, &w.menu[0].exec] {
            assert!(!text.contains("sk-SECRET"), "{text}");
        }
        assert!(w.exec.contains("${MPLUGIN_SECRET_API_KEY}"), "{}", w.exec);
        assert!(
            w.exec.ends_with("x/flash"),
            "plain settings still substitute"
        );
        assert_eq!(w.label, "AI ");
        let env = PluginSecretEnv {
            var: "MPLUGIN_SECRET_API_KEY".into(),
            plugin: "official/assistant".into(),
            key: "api_key".into(),
        };
        assert_eq!(w.secret_env, vec![env.clone()]);
        assert_eq!(w.menu[0].secret_env, vec![env]);
    }

    /// Config loads/saves read secrets from the cache only; one not read yet
    /// is reported as missing (for the background prefetch) instead of
    /// blocking on the keyring.
    #[test]
    fn setting_values_never_block_on_the_keyring() {
        let plugin = InstalledPlugin {
            key: "test-bridge/p".into(),
            manifest: Manifest {
                id: "p".into(),
                version: "1".into(),
                settings: vec![
                    Setting {
                        key: "known".into(),
                        kind: "secret".into(),
                        ..Default::default()
                    },
                    Setting {
                        key: "unread".into(),
                        kind: "secret".into(),
                        default: "dflt".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            dir: "/p".into(),
        };
        mshell_plugins::secrets::remember("test-bridge/p", "known", Some("cached-v".into()));
        let mut missing = Vec::new();
        let v = setting_values(&plugin, &PluginsState::default(), &mut missing);
        assert_eq!(v["known"], "cached-v");
        assert_eq!(v["unread"], "dflt");
        assert_eq!(
            missing,
            vec![("test-bridge/p".to_string(), "unread".to_string())]
        );
    }
}
