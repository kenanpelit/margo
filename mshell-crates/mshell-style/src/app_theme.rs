//! Theme side effects for external apps that can follow margo's matugen
//! palette. Kept in `mshell-style` so both the Settings panel ("Apply now")
//! and the style manager's matugen-complete path use the same code.

use mshell_config::schema::config::{HeliumTargetMode, HeliumTheme};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeliumProfile {
    pub instance: String,
    pub profile: String,
    pub display_name: String,
    pub preferences_path: PathBuf,
    pub last_used: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeliumApplyReport {
    pub applied: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

impl HeliumApplyReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.errors.is_empty() {
            format!(
                "{} applied, {} unchanged, {} skipped",
                self.applied, self.unchanged, self.skipped
            )
        } else {
            format!(
                "{} applied, {} unchanged, {} skipped, {} errors",
                self.applied,
                self.unchanged,
                self.skipped,
                self.errors.len()
            )
        }
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if path == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    PathBuf::from(path)
}

pub fn discover_helium_profiles(root: &Path) -> Vec<HeliumProfile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };

    for entry in entries.flatten() {
        let instance_dir = entry.path();
        if !instance_dir.join("Local State").is_file() {
            continue;
        }
        let instance = entry.file_name().to_string_lossy().to_string();
        out.extend(discover_instance_profiles(&instance, &instance_dir));
    }

    out.sort_by(|a, b| {
        a.instance
            .cmp(&b.instance)
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| a.profile.cmp(&b.profile))
    });
    out
}

fn discover_instance_profiles(instance: &str, instance_dir: &Path) -> Vec<HeliumProfile> {
    let local_state = std::fs::read_to_string(instance_dir.join("Local State"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);

    let last_used = local_state
        .pointer("/profile/last_used")
        .and_then(Value::as_str)
        .unwrap_or("Default");

    let mut names: BTreeMap<String, String> = BTreeMap::new();
    if let Some(info) = local_state
        .pointer("/profile/info_cache")
        .and_then(Value::as_object)
    {
        for (profile, meta) in info {
            let display = meta
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(profile)
                .to_string();
            names.insert(profile.clone(), display);
        }
    }

    let mut profiles: BTreeSet<String> = names.keys().cloned().collect();
    if let Ok(entries) = std::fs::read_dir(instance_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("Preferences").is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                profiles.insert(name.to_string());
            }
        }
    }

    profiles
        .into_iter()
        .filter_map(|profile| {
            let preferences_path = instance_dir.join(&profile).join("Preferences");
            if !preferences_path.is_file() {
                return None;
            }
            let display_name = names
                .get(&profile)
                .cloned()
                .unwrap_or_else(|| profile.clone());
            Some(HeliumProfile {
                instance: instance.to_string(),
                profile: profile.clone(),
                display_name,
                preferences_path,
                last_used: profile == last_used,
            })
        })
        .collect()
}

pub fn apply_helium_from_cache(config: &HeliumTheme) -> HeliumApplyReport {
    if !config.enabled {
        return HeliumApplyReport {
            skipped: 1,
            ..HeliumApplyReport::default()
        };
    }

    let Some(hex) = read_matugen_primary() else {
        return HeliumApplyReport {
            errors: vec!["Could not read primary_color.base from mshell-colors.toml".to_string()],
            ..HeliumApplyReport::default()
        };
    };
    apply_helium_color(config, &hex)
}

pub fn apply_helium_color(config: &HeliumTheme, hex: &str) -> HeliumApplyReport {
    let Some(color) = chromium_color_int(hex) else {
        return HeliumApplyReport {
            errors: vec![format!("Invalid matugen colour: {hex}")],
            ..HeliumApplyReport::default()
        };
    };

    let root = expand_home(&config.isolated_root);
    let profiles = discover_helium_profiles(&root);
    let targets = select_targets(config, profiles);
    let mut report = HeliumApplyReport::default();

    if targets.is_empty() {
        report.skipped += 1;
        return report;
    }

    for target in targets {
        match apply_profile_color(&target.preferences_path, color) {
            Ok(true) => report.applied += 1,
            Ok(false) => report.unchanged += 1,
            Err(err) => report.errors.push(format!(
                "{} / {}: {err}",
                target.instance, target.display_name
            )),
        }
    }

    report
}

pub fn apply_helium_from_cache_async(config: HeliumTheme) {
    if !config.enabled || !config.apply_on_theme_change {
        return;
    }
    std::thread::spawn(move || {
        let report = apply_helium_from_cache(&config);
        if report.is_ok() {
            debug!(summary = %report.summary(), "helium theme sync complete");
        } else {
            warn!(summary = %report.summary(), errors = ?report.errors, "helium theme sync failed");
        }
    });
}

fn select_targets(config: &HeliumTheme, profiles: Vec<HeliumProfile>) -> Vec<HeliumProfile> {
    match config.target_mode {
        HeliumTargetMode::All => profiles,
        HeliumTargetMode::LastUsed => profiles.into_iter().filter(|p| p.last_used).collect(),
        HeliumTargetMode::Selected => {
            let selected: BTreeSet<(String, String)> = config
                .targets
                .iter()
                .filter(|t| t.enabled)
                .map(|t| (t.instance.clone(), t.profile.clone()))
                .collect();
            profiles
                .into_iter()
                .filter(|p| selected.contains(&(p.instance.clone(), p.profile.clone())))
                .collect()
        }
    }
}

fn apply_profile_color(path: &Path, color: i64) -> anyhow::Result<bool> {
    let body = std::fs::read_to_string(path)?;
    let mut prefs: Value = serde_json::from_str(&body)?;

    let browser = object_child(root_object(&mut prefs)?, "browser");
    let theme = object_child(browser, "theme");
    let before = theme.get("user_color2").cloned();
    theme.insert("user_color2".to_string(), Value::from(color));
    theme.insert("color_variant2".to_string(), Value::from(1));

    let extensions = object_child(root_object(&mut prefs)?, "extensions");
    let ext_theme = object_child(extensions, "theme");
    ext_theme.insert(
        "id".to_string(),
        Value::from("user_color_theme_id".to_string()),
    );

    if before.as_ref() == Some(&Value::from(color)) {
        return Ok(false);
    }

    let backup = path.with_file_name("Preferences.margo-theme-backup");
    if !backup.exists()
        && let Err(err) = std::fs::copy(path, &backup)
    {
        warn!(path = %backup.display(), error = %err, "helium theme: backup failed");
    }

    let next = serde_json::to_string(&prefs)?;
    let tmp = path.with_file_name("Preferences.margo-theme.tmp");
    std::fs::write(&tmp, next)?;
    std::fs::rename(&tmp, path)?;
    Ok(true)
}

fn root_object(value: &mut Value) -> anyhow::Result<&mut Map<String, Value>> {
    value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Preferences root is not a JSON object"))
}

fn object_child<'a>(parent: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let entry = parent
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry.as_object_mut().expect("object inserted above")
}

fn read_matugen_primary() -> Option<String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let body = std::fs::read_to_string(base.join("margo").join("mshell-colors.toml")).ok()?;
    field_base(&body, "primary_color")
}

fn field_base(toml: &str, key: &str) -> Option<String> {
    let line = toml.lines().find(|l| l.trim_start().starts_with(key))?;
    let after = line.split("base").nth(1)?;
    let h = after.find('#')?;
    let hex = after[h..].chars().take(7).collect::<String>();
    if chromium_color_int(&hex).is_some() {
        Some(hex)
    } else {
        None
    }
}

fn chromium_color_int(hex: &str) -> Option<i64> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    let argb = 0xff00_0000u32 | rgb;
    Some((argb as i32) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_color_uses_signed_argb() {
        assert_eq!(chromium_color_int("#336699"), Some(-13408615));
    }

    #[test]
    fn matugen_primary_inline_table_is_parsed() {
        let body = r##"
            [appearance]
            primary_color = { base = "#aabbcc", text = "#111111" }
        "##;
        assert_eq!(
            field_base(body, "primary_color").as_deref(),
            Some("#aabbcc")
        );
    }
}
