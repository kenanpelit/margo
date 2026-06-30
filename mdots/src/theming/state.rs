//! State tracking for theming configuration
//!
//! Tracks applied theming settings to enable idempotent updates
//! and detect configuration changes.

use crate::config::ThemingConfig;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// State tracking for theming (persisted to theming-state.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemingState {
    /// Timestamp when this state was last updated
    #[serde(default = "Utc::now")]
    pub last_updated: DateTime<Utc>,

    /// Scope used for theming
    pub scope: String,

    /// Cursor theme
    #[serde(default)]
    pub cursor_theme: Option<String>,

    /// Cursor size
    #[serde(default)]
    pub cursor_size: Option<u32>,

    /// Icon theme
    #[serde(default)]
    pub icons: Option<String>,

    /// Main theme
    #[serde(default)]
    pub theme: Option<String>,

    /// Dark/light mode preference
    #[serde(default)]
    pub dark_or_light: Option<String>,

    /// Font family
    #[serde(default)]
    pub font_family: Option<String>,

    /// Font size
    #[serde(default)]
    pub font_size: Option<f32>,

    /// GTK-specific settings
    #[serde(default)]
    pub gtk_settings: HashMap<String, String>,

    /// Qt-specific settings
    #[serde(default)]
    pub qt_settings: HashMap<String, String>,

    /// Environment variables
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
}

impl Default for ThemingState {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            scope: "user".to_string(),
            cursor_theme: None,
            cursor_size: None,
            icons: None,
            theme: None,
            dark_or_light: None,
            font_family: None,
            font_size: None,
            gtk_settings: HashMap::new(),
            qt_settings: HashMap::new(),
            env_vars: HashMap::new(),
        }
    }
}

impl ThemingState {
    /// Check if the configuration has changed from this state
    pub fn has_changed(&self, config: &ThemingConfig) -> bool {
        // Check cursor
        if let Some(ref cursor) = config.cursor {
            if self.cursor_theme.as_ref() != Some(&cursor.theme) {
                return true;
            }
            if self.cursor_size != cursor.size {
                return true;
            }
        } else if self.cursor_theme.is_some() {
            return true;
        }

        // Check icons
        if self.icons != config.icons {
            return true;
        }

        // Check theme
        if self.theme != config.theme {
            return true;
        }

        // Check dark/light mode
        if self.dark_or_light != config.dark_or_light {
            return true;
        }

        // Check font
        if let Some(ref font) = config.font {
            if self.font_family != font.family {
                return true;
            }
            if self.font_size != font.size {
                return true;
            }
        } else if self.font_family.is_some() {
            return true;
        }

        // Check GTK settings
        if let Some(ref gtk) = config.gtk {
            let mut gtk_map = HashMap::new();
            if let Some(decorations) = gtk.decorations {
                gtk_map.insert("decorations".to_string(), decorations.to_string());
            }
            if let Some(ref primary) = gtk.primary_button {
                gtk_map.insert("primary_button".to_string(), primary.clone());
            }
            if let Some(animations) = gtk.enable_animations {
                gtk_map.insert("enable_animations".to_string(), animations.to_string());
            }
            if self.gtk_settings != gtk_map {
                return true;
            }
        } else if !self.gtk_settings.is_empty() {
            return true;
        }

        // Check Qt settings
        if let Some(ref qt) = config.qt {
            let mut qt_map = HashMap::new();
            qt_map.insert("backend".to_string(), format!("{:?}", qt.backend));
            if let Some(ref style) = qt.style {
                qt_map.insert("style".to_string(), style.clone());
            }
            if let Some(ref icon_theme) = qt.icon_theme {
                qt_map.insert("icon_theme".to_string(), icon_theme.clone());
            }
            if self.qt_settings != qt_map {
                return true;
            }
        } else if !self.qt_settings.is_empty() {
            return true;
        }

        // Check env vars
        if self.env_vars != config.env_vars {
            return true;
        }

        false
    }
}

/// Create theming state from config
pub fn create_theming_state(config: &ThemingConfig) -> ThemingState {
    let mut gtk_map = HashMap::new();
    if let Some(ref gtk) = config.gtk {
        if let Some(decorations) = gtk.decorations {
            gtk_map.insert("decorations".to_string(), decorations.to_string());
        }
        if let Some(ref primary) = gtk.primary_button {
            gtk_map.insert("primary_button".to_string(), primary.clone());
        }
        if let Some(animations) = gtk.enable_animations {
            gtk_map.insert("enable_animations".to_string(), animations.to_string());
        }
    }

    let mut qt_map = HashMap::new();
    if let Some(ref qt) = config.qt {
        qt_map.insert("backend".to_string(), format!("{:?}", qt.backend));
        if let Some(ref style) = qt.style {
            qt_map.insert("style".to_string(), style.clone());
        }
        if let Some(ref icon_theme) = qt.icon_theme {
            qt_map.insert("icon_theme".to_string(), icon_theme.clone());
        }
    }

    ThemingState {
        last_updated: Utc::now(),
        scope: format!("{:?}", config.scope).to_lowercase(),
        cursor_theme: config.cursor.as_ref().map(|c| c.theme.clone()),
        cursor_size: config.cursor.as_ref().and_then(|c| c.size),
        icons: config.icons.clone(),
        theme: config.theme.clone(),
        dark_or_light: config.dark_or_light.clone(),
        font_family: config.font.as_ref().and_then(|f| f.family.clone()),
        font_size: config.font.as_ref().and_then(|f| f.size),
        gtk_settings: gtk_map,
        qt_settings: qt_map,
        env_vars: config.env_vars.clone(),
    }
}

/// Load theming state from YAML file
pub fn load_theming_state(state_file: &Path) -> Result<ThemingState> {
    if !state_file.exists() {
        log::debug!("Theming state file does not exist, returning default state");
        return Ok(ThemingState::default());
    }

    let content = fs::read_to_string(state_file).context(format!(
        "Failed to read theming state file: {:?}",
        state_file
    ))?;

    let state: ThemingState =
        serde_yaml::from_str(&content).context("Failed to parse theming state YAML")?;

    log::debug!("Loaded theming state from {:?}", state_file);
    Ok(state)
}

/// Save theming state to YAML file
pub fn save_theming_state(state_file: &Path, state: &ThemingState) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .context(format!("Failed to create state directory: {:?}", parent))?;
    }

    let yaml = serde_yaml::to_string(state).context("Failed to serialize theming state to YAML")?;

    fs::write(state_file, yaml).context(format!(
        "Failed to write theming state file: {:?}",
        state_file
    ))?;

    log::debug!("Saved theming state to {:?}", state_file);
    Ok(())
}
