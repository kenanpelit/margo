use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use colored::Colorize;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::DefaultsScope;

/// State tracking for default applications (persisted to defaults-state.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsState {
    /// Timestamp when this state was last updated
    #[serde(default = "Utc::now")]
    pub last_updated: DateTime<Utc>,

    /// Scope used for defaults (user or system)
    pub scope: DefaultsScope,

    /// High-level app category mappings (browser, text_editor, etc.)
    #[serde(default)]
    pub apps: HashMap<String, String>,

    /// Custom MIME type mappings
    #[serde(default)]
    pub custom_mime_types: HashMap<String, String>,
}

impl Default for DefaultsState {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            scope: DefaultsScope::System,
            apps: HashMap::new(),
            custom_mime_types: HashMap::new(),
        }
    }
}

/// Report of default apps sync operations
#[derive(Debug, Clone)]
pub struct DefaultsSyncReport {
    pub apps_updated: Vec<String>,
    pub mime_types_updated: Vec<String>,
    pub errors: Vec<DefaultError>,
}

impl DefaultsSyncReport {
    pub fn new() -> Self {
        Self {
            apps_updated: Vec::new(),
            mime_types_updated: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn has_changes(&self) -> bool {
        !self.apps_updated.is_empty() || !self.mime_types_updated.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Error information for default app operations
#[derive(Debug, Clone)]
pub struct DefaultError {
    pub app_or_mime: String,
    pub desktop_file: String,
    pub error_type: DefaultErrorType,
}

#[derive(Debug, Clone)]
pub enum DefaultErrorType {
    DesktopFileNotFound,
    InvalidDesktopFileName,
    XdgMimeCommandFailed,
}

/// Manager for default applications operations
pub struct DefaultsManager;

impl DefaultsManager {
    /// Validate desktop file name (prevent command injection)
    pub fn validate_desktop_file_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(anyhow!("Desktop file name cannot be empty"));
        }

        // Allow alphanumeric, dash, underscore, dot
        let valid_chars = name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.');

        if !valid_chars {
            return Err(anyhow!(
                "Invalid desktop file name '{}': only alphanumeric, dash, underscore, dot allowed",
                name
            ));
        }

        Ok(())
    }

    /// Resolve desktop file - handle both "firefox" and "firefox.desktop"
    pub fn resolve_desktop_file(name: &str) -> String {
        if name.ends_with(".desktop") {
            name.to_string()
        } else {
            format!("{}.desktop", name)
        }
    }

    /// Get all directories where desktop files might be located
    pub fn get_desktop_file_directories() -> Vec<PathBuf> {
        let mut dirs = vec![
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        ];

        if let Ok(home) = std::env::var("HOME") {
            let home = PathBuf::from(home);
            dirs.push(home.join(".local/share/applications"));
            dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
        }

        dirs
    }

    /// Check if desktop file exists in system, user, or flatpak applications directories
    pub fn desktop_file_exists(desktop_file: &str) -> bool {
        let desktop_file = Self::resolve_desktop_file(desktop_file);

        debug!("Checking if desktop file exists: {}", desktop_file);

        for dir in Self::get_desktop_file_directories() {
            let path = dir.join(&desktop_file);
            if path.exists() {
                debug!("Found in {}", dir.display());
                return true;
            }
        }

        debug!("Desktop file not found: {}", desktop_file);
        false
    }

    /// Get current default application for a MIME type
    pub fn get_current_default_for_mime(mime_type: &str) -> Result<Option<String>> {
        debug!("Querying default for MIME type: {}", mime_type);

        let output = Command::new("xdg-mime")
            .args(["query", "default", mime_type])
            .output()
            .context(format!(
                "Failed to query default for MIME type {}",
                mime_type
            ))?;

        if !output.status.success() {
            return Ok(None);
        }

        let result = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if result.is_empty() {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }

    /// Set default application for a MIME type
    pub fn set_default_for_mime(desktop_file: &str, mime_type: &str) -> Result<()> {
        let desktop_file = Self::resolve_desktop_file(desktop_file);

        info!("Setting default for {}: {}", mime_type, desktop_file);

        let status = Command::new("xdg-mime")
            .args(["default", &desktop_file, mime_type])
            .status()
            .context(format!(
                "Failed to set default for MIME type {} to {}",
                mime_type, desktop_file
            ))?;

        if !status.success() {
            return Err(anyhow!(
                "Failed to set default for MIME type {} to {}",
                mime_type,
                desktop_file
            ));
        }

        Ok(())
    }

    /// Get MIME types for high-level app category
    fn get_mime_types_for_category(category: &str) -> Vec<&'static str> {
        match category {
            "browser" => vec![
                "text/html",
                "application/xhtml+xml",
                "x-scheme-handler/http",
                "x-scheme-handler/https",
            ],
            "text_editor" => vec!["text/plain", "text/x-log", "text/x-readme"],
            "file_manager" => vec!["inode/directory"],
            "terminal" => vec!["x-scheme-handler/terminal"],
            "video_player" => vec![
                "video/mp4",
                "video/x-matroska",
                "video/webm",
                "video/mpeg",
                "video/x-msvideo",
            ],
            "audio_player" => vec![
                "audio/mpeg",
                "audio/mp4",
                "audio/x-flac",
                "audio/x-vorbis+ogg",
                "audio/x-wav",
            ],
            "image_viewer" => vec![
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "image/svg+xml",
            ],
            "pdf_viewer" => vec!["application/pdf"],
            _ => vec![],
        }
    }

    /// Sync default applications based on configuration
    pub fn sync_defaults(
        config_apps: &HashMap<String, String>,
        config_mime_types: &HashMap<String, String>,
        scope: &DefaultsScope,
        previous_state: &DefaultsState,
    ) -> Result<DefaultsSyncReport> {
        let mut report = DefaultsSyncReport::new();

        // Check if configuration has changed
        let apps_changed = config_apps != &previous_state.apps;
        let mime_changed = config_mime_types != &previous_state.custom_mime_types;
        let scope_changed = scope != &previous_state.scope;

        if !apps_changed && !mime_changed && !scope_changed {
            crate::ui::step("Defaults", "already in sync");
            return Ok(report);
        }

        crate::ui::step("Syncing", "default applications");

        // Phase 1: Pre-flight validation (NixOS-like)
        let mut validation_errors = Vec::new();

        // Validate all high-level app desktop files exist
        for (category, desktop_file) in config_apps {
            if Self::validate_desktop_file_name(desktop_file).is_err() {
                validation_errors.push(DefaultError {
                    app_or_mime: category.clone(),
                    desktop_file: desktop_file.clone(),
                    error_type: DefaultErrorType::InvalidDesktopFileName,
                });
                continue;
            }

            if !Self::desktop_file_exists(desktop_file) {
                validation_errors.push(DefaultError {
                    app_or_mime: category.clone(),
                    desktop_file: desktop_file.clone(),
                    error_type: DefaultErrorType::DesktopFileNotFound,
                });
            }
        }

        // Validate all custom MIME type desktop files exist
        for (mime_type, desktop_file) in config_mime_types {
            if Self::validate_desktop_file_name(desktop_file).is_err() {
                validation_errors.push(DefaultError {
                    app_or_mime: mime_type.clone(),
                    desktop_file: desktop_file.clone(),
                    error_type: DefaultErrorType::InvalidDesktopFileName,
                });
                continue;
            }

            if !Self::desktop_file_exists(desktop_file) {
                validation_errors.push(DefaultError {
                    app_or_mime: mime_type.clone(),
                    desktop_file: desktop_file.clone(),
                    error_type: DefaultErrorType::DesktopFileNotFound,
                });
            }
        }

        // FAIL if any validation errors (NixOS-like behavior)
        if !validation_errors.is_empty() {
            eprintln!();
            eprintln!(
                "{}",
                "✗ Validation failed - desktop files not found:"
                    .red()
                    .bold()
            );
            eprintln!();
            for error in &validation_errors {
                match error.error_type {
                    DefaultErrorType::DesktopFileNotFound => {
                        eprintln!(
                            "  {} {}: desktop file '{}' not found",
                            "✗".red(),
                            error.app_or_mime,
                            error.desktop_file
                        );
                        eprintln!("    Searched in:");
                        for dir in Self::get_desktop_file_directories() {
                            // Display with ~ for home directory for readability
                            let display_path = if let Ok(home) = std::env::var("HOME") {
                                dir.to_string_lossy().replace(&home, "~")
                            } else {
                                dir.to_string_lossy().to_string()
                            };
                            eprintln!("      - {}/", display_path);
                        }
                    }
                    DefaultErrorType::InvalidDesktopFileName => {
                        eprintln!(
                            "  {} {}: invalid desktop file name '{}'",
                            "✗".red(),
                            error.app_or_mime,
                            error.desktop_file
                        );
                    }
                    _ => {}
                }
            }
            eprintln!();
            eprintln!("Install the required applications before running 'mdots sync'");

            return Err(anyhow!(
                "Validation failed: {} desktop file(s) not found",
                validation_errors.len()
            ));
        }

        // Phase 2: Apply changes

        // Process high-level app categories
        for (category, desktop_file) in config_apps {
            let mime_types = Self::get_mime_types_for_category(category);

            if mime_types.is_empty() {
                warn!("Unknown app category: {}", category);
                continue;
            }

            // Set default for all MIME types in this category
            let mut category_updated = false;
            for mime_type in mime_types {
                match Self::set_default_for_mime(desktop_file, mime_type) {
                    Ok(_) => {
                        category_updated = true;
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} Failed to set {} for {}: {}",
                            "✗".red(),
                            category,
                            mime_type,
                            e
                        );
                        report.errors.push(DefaultError {
                            app_or_mime: category.clone(),
                            desktop_file: desktop_file.clone(),
                            error_type: DefaultErrorType::XdgMimeCommandFailed,
                        });
                    }
                }
            }

            if category_updated {
                println!(
                    "  {} Set {} to {}",
                    "✓".green(),
                    category.replace("_", " "),
                    desktop_file.green()
                );
                report.apps_updated.push(category.clone());
            }
        }

        // Process custom MIME types
        for (mime_type, desktop_file) in config_mime_types {
            match Self::set_default_for_mime(desktop_file, mime_type) {
                Ok(_) => {
                    println!(
                        "  {} Set {} to {}",
                        "✓".green(),
                        mime_type,
                        desktop_file.green()
                    );
                    report.mime_types_updated.push(mime_type.clone());
                }
                Err(e) => {
                    eprintln!(
                        "  {} Failed to set {} to {}: {}",
                        "✗".red(),
                        mime_type,
                        desktop_file,
                        e
                    );
                    report.errors.push(DefaultError {
                        app_or_mime: mime_type.clone(),
                        desktop_file: desktop_file.clone(),
                        error_type: DefaultErrorType::XdgMimeCommandFailed,
                    });
                }
            }
        }

        // Print summary
        if report.has_changes() {
            println!();
            if !report.apps_updated.is_empty() {
                println!("App categories updated: {}", report.apps_updated.len());
            }
            if !report.mime_types_updated.is_empty() {
                println!(
                    "Custom MIME types updated: {}",
                    report.mime_types_updated.len()
                );
            }
        } else {
            println!("  No changes needed");
        }

        if !report.errors.is_empty() {
            println!();
            eprintln!(
                "{}: {} default app operations failed",
                "Warning".yellow(),
                report.errors.len()
            );
        }

        Ok(report)
    }
}

/// Load defaults state from YAML file
pub fn load_defaults_state(state_file: &Path) -> Result<DefaultsState> {
    if !state_file.exists() {
        debug!("Defaults state file does not exist, returning default state");
        return Ok(DefaultsState::default());
    }

    let content = fs::read_to_string(state_file).context(format!(
        "Failed to read defaults state file: {:?}",
        state_file
    ))?;

    let state: DefaultsState =
        serde_yaml::from_str(&content).context("Failed to parse defaults state YAML")?;

    debug!("Loaded defaults state from {:?}", state_file);
    Ok(state)
}

/// Save defaults state to YAML file
pub fn save_defaults_state(state_file: &Path, state: &DefaultsState) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .context(format!("Failed to create state directory: {:?}", parent))?;
    }

    let yaml =
        serde_yaml::to_string(state).context("Failed to serialize defaults state to YAML")?;

    fs::write(state_file, yaml).context(format!(
        "Failed to write defaults state file: {:?}",
        state_file
    ))?;

    debug!("Saved defaults state to {:?}", state_file);
    Ok(())
}

/// Create updated defaults state from config
pub fn create_updated_state(
    apps: &HashMap<String, String>,
    custom_mime_types: &HashMap<String, String>,
    scope: &DefaultsScope,
) -> DefaultsState {
    DefaultsState {
        last_updated: Utc::now(),
        scope: scope.clone(),
        apps: apps.clone(),
        custom_mime_types: custom_mime_types.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_desktop_file_name() {
        assert!(DefaultsManager::validate_desktop_file_name("firefox.desktop").is_ok());
        assert!(DefaultsManager::validate_desktop_file_name("firefox").is_ok());
        assert!(DefaultsManager::validate_desktop_file_name("code-oss").is_ok());
        assert!(DefaultsManager::validate_desktop_file_name("org.gnome.Nautilus").is_ok());

        assert!(DefaultsManager::validate_desktop_file_name("").is_err());
        assert!(DefaultsManager::validate_desktop_file_name("firefox; rm -rf /").is_err());
        assert!(DefaultsManager::validate_desktop_file_name("firefox && malicious").is_err());
    }

    #[test]
    fn test_resolve_desktop_file() {
        assert_eq!(
            DefaultsManager::resolve_desktop_file("firefox"),
            "firefox.desktop"
        );
        assert_eq!(
            DefaultsManager::resolve_desktop_file("firefox.desktop"),
            "firefox.desktop"
        );
    }

    #[test]
    fn test_mime_types_for_category() {
        let browser_mimes = DefaultsManager::get_mime_types_for_category("browser");
        assert!(browser_mimes.contains(&"text/html"));
        assert!(browser_mimes.contains(&"x-scheme-handler/http"));

        let pdf_mimes = DefaultsManager::get_mime_types_for_category("pdf_viewer");
        assert!(pdf_mimes.contains(&"application/pdf"));

        let unknown = DefaultsManager::get_mime_types_for_category("unknown_category");
        assert!(unknown.is_empty());
    }
}
