//! Desktop theming configuration module
//!
//! Handles declarative theming configuration for GTK and Qt applications.
//! Writes to ~/.config/gtk-3.0/settings.ini, ~/.config/gtk-4.0/settings.ini,
//! ~/.config/qt5ct/qt5ct.conf, and manages environment variables.

pub mod state;

use crate::config::{QtBackend, ThemingConfig};
use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Manager for desktop theming operations
pub struct ThemingManager;

impl ThemingManager {
    /// Apply theming configuration to the system
    pub fn apply_theming(config: &ThemingConfig, dry_run: bool) -> Result<ThemingReport> {
        let mut report = ThemingReport::new();

        // Skip if no theming is configured
        if !has_theming_config(config) {
            debug!("No theming configuration found, skipping");
            return Ok(report);
        }

        if !dry_run {
            crate::ui::step("Syncing", "desktop theming");
        }

        // Phase 1: Validation
        if let Err(e) = Self::validate_themes(config) {
            return Err(anyhow!("Theming validation failed: {}", e));
        }

        // Phase 2: Backup existing configs
        if !dry_run {
            if let Err(e) = Self::backup_configs() {
                warn!("Failed to backup existing theming configs: {}", e);
            }
        }

        // Phase 3: Apply GTK settings
        if let Err(e) = Self::apply_gtk_settings(config, dry_run, &mut report) {
            report.errors.push(ThemingError {
                component: "gtk".to_string(),
                message: e.to_string(),
            });
        }

        // Phase 4: Apply Qt settings
        if let Err(e) = Self::apply_qt_settings(config, dry_run, &mut report) {
            report.errors.push(ThemingError {
                component: "qt".to_string(),
                message: e.to_string(),
            });
        }

        // Phase 5: Apply cursor theme
        if let Err(e) = Self::apply_cursor_theme(config, dry_run, &mut report) {
            report.errors.push(ThemingError {
                component: "cursor".to_string(),
                message: e.to_string(),
            });
        }

        // Phase 6: Apply environment variables
        if let Err(e) = Self::apply_environment_vars(config, dry_run, &mut report) {
            report.errors.push(ThemingError {
                component: "env_vars".to_string(),
                message: e.to_string(),
            });
        }

        // Print summary
        if !dry_run && report.has_changes() {
            println!();
            if !report.gtk_updated.is_empty() {
                println!("GTK settings updated: {}", report.gtk_updated.len());
            }
            if !report.qt_updated.is_empty() {
                println!("Qt settings updated: {}", report.qt_updated.len());
            }
            if report.cursor_updated {
                println!("Cursor theme updated");
            }
            if report.env_vars_updated {
                println!("Environment variables updated");
            }
        }

        if !report.errors.is_empty() {
            println!();
            eprintln!(
                "{}: {} theming operations failed",
                "Warning".yellow(),
                report.errors.len()
            );
            for error in &report.errors {
                eprintln!("  {} {}: {}", "✗".red(), error.component, error.message);
            }
        }

        Ok(report)
    }

    /// Validate that theme files exist
    fn validate_themes(config: &ThemingConfig) -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;

        // Validate icon theme
        if let Some(ref icons) = config.icons {
            if let Some(actual_name) = find_theme_name(&home, "icons", icons) {
                // Theme found, will use actual case-sensitive name during application
                info!("Icon theme '{}' found as '{}'", icons, actual_name);
            } else {
                // Theme not found, show available themes
                let available = list_available_themes(&home, "icons");
                let suggestion = find_similar_theme(icons, &available);

                let mut error_msg = format!("Icon theme '{}' not found", icons);
                if let Some(similar) = suggestion {
                    error_msg.push_str(&format!("\n\n  Did you mean: '{}' ?", similar));
                }
                if !available.is_empty() {
                    error_msg.push_str(&format!(
                        "\n\n  Available icon themes ({} found):",
                        available.len()
                    ));
                    for theme in available.iter().take(20) {
                        error_msg.push_str(&format!("\n    - {}", theme));
                    }
                    if available.len() > 20 {
                        error_msg.push_str(&format!("\n    ... and {} more", available.len() - 20));
                    }
                }
                return Err(anyhow!(error_msg));
            }
        }

        // Validate main theme
        if let Some(ref theme) = config.theme {
            if let Some(actual_name) = find_theme_name(&home, "themes", theme) {
                info!("GTK theme '{}' found as '{}'", theme, actual_name);
            } else {
                let available = list_available_themes(&home, "themes");
                let suggestion = find_similar_theme(theme, &available);

                let mut error_msg = format!("GTK theme '{}' not found", theme);
                if let Some(similar) = suggestion {
                    error_msg.push_str(&format!("\n\n  Did you mean: '{}' ?", similar));
                }
                if !available.is_empty() {
                    error_msg.push_str(&format!(
                        "\n\n  Available themes ({} found):",
                        available.len()
                    ));
                    for t in available.iter().take(20) {
                        error_msg.push_str(&format!("\n    - {}", t));
                    }
                    if available.len() > 20 {
                        error_msg.push_str(&format!("\n    ... and {} more", available.len() - 20));
                    }
                }
                return Err(anyhow!(error_msg));
            }
        }

        // Validate cursor theme
        if let Some(ref cursor) = config.cursor {
            if let Some(actual_name) = find_cursor_theme_name(&home, &cursor.theme) {
                info!("Cursor theme '{}' found as '{}'", cursor.theme, actual_name);
            } else {
                let available = list_available_cursors(&home);
                let suggestion = find_similar_theme(&cursor.theme, &available);

                let mut error_msg = format!("Cursor theme '{}' not found", cursor.theme);
                if let Some(similar) = suggestion {
                    error_msg.push_str(&format!("\n\n  Did you mean: '{}' ?", similar));
                }
                if !available.is_empty() {
                    error_msg.push_str(&format!(
                        "\n\n  Available cursor themes ({} found):",
                        available.len()
                    ));
                    for t in available.iter().take(20) {
                        error_msg.push_str(&format!("\n    - {}", t));
                    }
                    if available.len() > 20 {
                        error_msg.push_str(&format!("\n    ... and {} more", available.len() - 20));
                    }
                }
                return Err(anyhow!(error_msg));
            }
        }

        Ok(())
    }

    /// Backup existing config files before modification
    fn backup_configs() -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");

        // Backup GTK configs
        for version in ["3.0", "4.0"] {
            let gtk_config =
                PathBuf::from(&home).join(format!(".config/gtk-{}/settings.ini", version));
            if gtk_config.exists() {
                let backup_path =
                    gtk_config.with_extension(format!("ini.mdots-backup-{}", timestamp));
                fs::copy(&gtk_config, &backup_path)
                    .with_context(|| format!("Failed to backup {:?}", gtk_config))?;
                debug!("Backed up {:?} to {:?}", gtk_config, backup_path);
            }
        }

        // Backup Qt configs
        for version in ["qt5ct", "qt6ct"] {
            let qt_config =
                PathBuf::from(&home).join(format!(".config/{}/{}ct.conf", version, &version[..3]));
            if qt_config.exists() {
                let backup_path =
                    qt_config.with_extension(format!("conf.mdots-backup-{}", timestamp));
                fs::copy(&qt_config, &backup_path)
                    .with_context(|| format!("Failed to backup {:?}", qt_config))?;
                debug!("Backed up {:?} to {:?}", qt_config, backup_path);
            }
        }

        Ok(())
    }

    /// Apply GTK settings to ~/.config/gtk-3.0/settings.ini and gtk-4.0/settings.ini
    fn apply_gtk_settings(
        config: &ThemingConfig,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;

        for version in ["3.0", "4.0"] {
            let gtk_dir = PathBuf::from(&home).join(format!(".config/gtk-{}", version));
            let settings_file = gtk_dir.join("settings.ini");

            // Create directory if it doesn't exist
            if !dry_run {
                fs::create_dir_all(&gtk_dir)
                    .with_context(|| format!("Failed to create {:?}", gtk_dir))?;
            }

            // Build settings
            let mut settings: HashMap<String, String> = HashMap::new();

            // Use case-sensitive theme names (find actual name from filesystem)
            if let Some(ref theme) = config.theme {
                let actual_theme =
                    find_theme_name(&home, "themes", theme).unwrap_or_else(|| theme.clone());
                settings.insert("gtk-theme-name".to_string(), actual_theme);
            }

            if let Some(ref icons) = config.icons {
                let actual_icons =
                    find_theme_name(&home, "icons", icons).unwrap_or_else(|| icons.clone());
                settings.insert("gtk-icon-theme-name".to_string(), actual_icons);
            }

            if let Some(ref cursor) = config.cursor {
                let actual_cursor = find_cursor_theme_name(&home, &cursor.theme)
                    .unwrap_or_else(|| cursor.theme.clone());
                settings.insert("gtk-cursor-theme-name".to_string(), actual_cursor);
                if let Some(size) = cursor.size {
                    settings.insert("gtk-cursor-theme-size".to_string(), size.to_string());
                }
            }

            if let Some(ref font) = config.font {
                if let Some(ref family) = font.family {
                    let size = font.size.map(|s| format!(" {:.1}", s)).unwrap_or_default();
                    settings.insert("gtk-font-name".to_string(), format!("{}{}", family, size));
                }
            }

            if let Some(ref dark_or_light) = config.dark_or_light {
                let prefer_dark = match dark_or_light.as_str() {
                    "dark" => "1",
                    "light" => "0",
                    _ => "0",
                };
                settings.insert(
                    "gtk-application-prefer-dark-theme".to_string(),
                    prefer_dark.to_string(),
                );
            }

            // GTK-specific settings
            if let Some(ref gtk) = config.gtk {
                if let Some(decorations) = gtk.decorations {
                    // GTK decorations layout
                    let layout = if decorations {
                        ":minimize,maximize,close"
                    } else {
                        ""
                    };
                    settings.insert("gtk-decoration-layout".to_string(), layout.to_string());
                }

                if let Some(ref primary_button) = gtk.primary_button {
                    settings.insert(
                        "gtk-primary-button-warps-slider".to_string(),
                        if primary_button == "left" { "1" } else { "0" }.to_string(),
                    );
                }

                if let Some(enable_animations) = gtk.enable_animations {
                    settings.insert(
                        "gtk-enable-animations".to_string(),
                        if enable_animations { "1" } else { "0" }.to_string(),
                    );
                }
            }

            if settings.is_empty() {
                continue;
            }

            // Write settings file
            if dry_run {
                info!("Would update GTK {} settings: {:?}", version, settings);
                for key in settings.keys() {
                    report.gtk_updated.push(format!("gtk-{}-{}", version, key));
                }
            } else {
                write_ini_file(&settings_file, &settings, "Settings")?;
                println!("  {} Updated GTK {} settings", "✓".green(), version);
                for key in settings.keys() {
                    report.gtk_updated.push(format!("gtk-{}-{}", version, key));
                }
            }
        }

        Ok(())
    }

    /// Apply Qt settings (qt5ct/qt6ct or KDE)
    fn apply_qt_settings(
        config: &ThemingConfig,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;

        // Determine which backend to use
        let backend = if let Some(ref qt) = config.qt {
            match qt.backend {
                QtBackend::Kde => QtBackend::Kde,
                QtBackend::Qt5ct => QtBackend::Qt5ct,
                QtBackend::Auto => detect_qt_backend(),
            }
        } else {
            return Ok(());
        };

        match backend {
            QtBackend::Kde => Self::apply_kde_settings(config, dry_run, report),
            QtBackend::Qt5ct | QtBackend::Auto => {
                Self::apply_qtct_settings(config, &home, dry_run, report)
            }
        }
    }

    /// Apply Qt settings via qt5ct/qt6ct
    fn apply_qtct_settings(
        config: &ThemingConfig,
        home: &str,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        // Apply to both qt5ct and qt6ct
        for version in ["qt5ct", "qt6ct"] {
            let qt_dir = PathBuf::from(home).join(format!(".config/{}", version));
            let config_file = qt_dir.join(format!("{}ct.conf", &version[..3]));

            // Create directory if it doesn't exist
            if !dry_run {
                fs::create_dir_all(&qt_dir)
                    .with_context(|| format!("Failed to create {:?}", qt_dir))?;
            }

            // Build settings
            let mut settings: HashMap<String, HashMap<String, String>> = HashMap::new();
            let mut appearance = HashMap::new();

            // Apply Qt-specific settings if present
            if let Some(ref qt) = config.qt {
                if let Some(ref style) = qt.style {
                    appearance.insert("style".to_string(), style.clone());
                }

                // Use Qt-specific icon theme or fall back to global icon theme
                let icons = qt
                    .icon_theme
                    .as_ref()
                    .map(|name| {
                        find_theme_name(home, "icons", name).unwrap_or_else(|| name.clone())
                    })
                    .or_else(|| {
                        config.icons.as_ref().map(|name| {
                            find_theme_name(home, "icons", name).unwrap_or_else(|| name.clone())
                        })
                    });
                if let Some(icon_theme) = icons {
                    appearance.insert("icon_theme".to_string(), icon_theme);
                }
            } else if let Some(ref icons) = config.icons {
                // Apply global icon theme to Qt even without qt section
                let icon_theme =
                    find_theme_name(home, "icons", icons).unwrap_or_else(|| icons.clone());
                appearance.insert("icon_theme".to_string(), icon_theme);
            }

            if config.theme.is_some() {
                appearance.insert("standard_dialogs".to_string(), "default".to_string());
            }

            if !appearance.is_empty() {
                settings.insert("Appearance".to_string(), appearance);
            }

            // Font settings - use Qt-specific font or fall back to global font
            let font_to_apply = config
                .qt
                .as_ref()
                .and_then(|qt| qt.font.as_ref())
                .or(config.font.as_ref());
            if let Some(font) = font_to_apply {
                let mut font_settings = HashMap::new();
                if let Some(ref family) = font.family {
                    let size = font.size.map(|s| format!(", {:.1}", s)).unwrap_or_default();
                    font_settings.insert("general".to_string(), format!("{}{}", family, size));
                }
                if !font_settings.is_empty() {
                    settings.insert("Fonts".to_string(), font_settings);
                }
            }

            if settings.is_empty() {
                continue;
            }

            // Write config file
            if dry_run {
                info!("Would update {} configuration", version);
                report.qt_updated.push(version.to_string());
            } else {
                write_qtct_config(&config_file, &settings)?;
                println!("  {} Updated {} configuration", "✓".green(), version);
                report.qt_updated.push(version.to_string());
            }
        }

        Ok(())
    }

    /// Apply Qt settings via KDE's kwriteconfig
    fn apply_kde_settings(
        config: &ThemingConfig,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        let mut commands: Vec<(String, String, String, String)> = Vec::new();

        if let Some(ref qt) = config.qt {
            if let Some(ref style) = qt.style {
                commands.push((
                    "kwriteconfig5".to_string(),
                    "kcm_style".to_string(),
                    "widgetStyle".to_string(),
                    style.clone(),
                ));
            }
        }

        if let Some(ref icons) = config.icons {
            // Use case-sensitive icon theme name
            let home = std::env::var("HOME").unwrap_or_default();
            let actual_icons =
                find_theme_name(&home, "icons", icons).unwrap_or_else(|| icons.clone());
            commands.push((
                "kwriteconfig5".to_string(),
                "kcm_icons".to_string(),
                "Theme".to_string(),
                actual_icons,
            ));
        }

        if let Some(ref font) = config.font {
            if let Some(ref family) = font.family {
                let font_str = if let Some(size) = font.size {
                    format!("{}, {}", family, size)
                } else {
                    family.clone()
                };
                commands.push((
                    "kwriteconfig5".to_string(),
                    "kcm_fonts".to_string(),
                    "font".to_string(),
                    font_str,
                ));
            }
        }

        let has_commands = !commands.is_empty();

        for (cmd, group, key, value) in &commands {
            if dry_run {
                info!(
                    "Would run: {} --file {} --group {} --key {} {}",
                    cmd, group, group, key, value
                );
            } else {
                let status = Command::new(cmd)
                    .args(["--file", group, "--group", group, "--key", key, value])
                    .status()
                    .with_context(|| format!("Failed to run {}", cmd))?;

                if !status.success() {
                    warn!("{} command failed for {}.{}", cmd, group, key);
                }
            }
        }

        if has_commands {
            report.qt_updated.push("kde".to_string());
        }

        Ok(())
    }

    /// Apply cursor theme settings
    fn apply_cursor_theme(
        config: &ThemingConfig,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        let Some(ref cursor) = config.cursor else {
            return Ok(());
        };

        let home = std::env::var("HOME").context("HOME environment variable not set")?;

        // Create/update ~/.icons/default/index.theme
        let icons_default_dir = PathBuf::from(&home).join(".icons/default");
        let index_file = icons_default_dir.join("index.theme");

        // Use case-sensitive cursor theme name
        let actual_cursor_theme =
            find_cursor_theme_name(&home, &cursor.theme).unwrap_or_else(|| cursor.theme.clone());

        if !dry_run {
            fs::create_dir_all(&icons_default_dir)
                .with_context(|| format!("Failed to create {:?}", icons_default_dir))?;

            let content = format!("[icon theme]\nInherits={}\n", actual_cursor_theme);
            fs::write(&index_file, content)
                .with_context(|| format!("Failed to write {:?}", index_file))?;
        }

        report.cursor_updated = true;

        if !dry_run {
            println!(
                "  {} Updated cursor theme to {} (size: {})",
                "✓".green(),
                actual_cursor_theme.green(),
                cursor
                    .size
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "default".to_string())
            );
        }

        Ok(())
    }

    /// Apply environment variables to ~/.mdots/environment
    fn apply_environment_vars(
        config: &ThemingConfig,
        dry_run: bool,
        report: &mut ThemingReport,
    ) -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        let mdots_dir = PathBuf::from(&home).join(".mdots");
        let env_file = mdots_dir.join("environment");

        let mut env_vars: HashMap<String, String> = HashMap::new();

        // Add cursor theme env vars (use case-sensitive name)
        if let Some(ref cursor) = config.cursor {
            let actual_cursor = find_cursor_theme_name(&home, &cursor.theme)
                .unwrap_or_else(|| cursor.theme.clone());
            env_vars.insert("XCURSOR_THEME".to_string(), actual_cursor);
            if let Some(size) = cursor.size {
                env_vars.insert("XCURSOR_SIZE".to_string(), size.to_string());
            }
        }

        // Add theme env var (use case-sensitive name)
        if let Some(ref theme) = config.theme {
            let actual_theme =
                find_theme_name(&home, "themes", theme).unwrap_or_else(|| theme.clone());
            env_vars.insert("GTK_THEME".to_string(), actual_theme);
        }

        // Add Qt platform theme based on backend
        if let Some(ref qt) = config.qt {
            let platform_theme = match qt.backend {
                QtBackend::Kde => "kde",
                QtBackend::Qt5ct | QtBackend::Auto => "qt5ct",
            };
            env_vars.insert(
                "QT_QPA_PLATFORMTHEME".to_string(),
                platform_theme.to_string(),
            );
        }

        // Add user-specified env vars
        for (key, value) in &config.env_vars {
            env_vars.insert(key.clone(), value.clone());
        }

        if env_vars.is_empty() {
            return Ok(());
        }

        // Write environment file
        if dry_run {
            info!("Would write environment variables to {:?}", env_file);
        } else {
            fs::create_dir_all(&mdots_dir)
                .with_context(|| format!("Failed to create {:?}", mdots_dir))?;

            let mut content =
                String::from("# mdots theming - auto-generated, do not edit manually\n\n");
            for (key, value) in &env_vars {
                content.push_str(&format!("export {}=\"{}\"\n", key, value));
            }

            fs::write(&env_file, content)
                .with_context(|| format!("Failed to write {:?}", env_file))?;

            println!(
                "  {} Updated environment variables in ~/.mdots/environment",
                "✓".green()
            );

            // Check if shell configs already source this file
            Self::update_shell_configs(dry_run)?;
        }

        report.env_vars_updated = true;
        Ok(())
    }

    /// Update shell configs to source ~/.mdots/environment
    fn update_shell_configs(dry_run: bool) -> Result<()> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;

        let shell_configs = [
            (".bashrc", "bash"),
            (".bash_profile", "bash"),
            (".zshrc", "zsh"),
            (".config/fish/config.fish", "fish"),
        ];

        let source_line =
            "# mdots theming\n[ -f ~/.mdots/environment ] && source ~/.mdots/environment\n";
        let fish_source_line =
            "# mdots theming\nif test -f ~/.mdots/environment\n  source ~/.mdots/environment\nend\n";

        for (config_file, shell) in &shell_configs {
            let config_path = PathBuf::from(&home).join(config_file);

            if !config_path.exists() {
                continue;
            }

            let content = fs::read_to_string(&config_path)?;

            // Check if already sourced
            if content.contains(".mdots/environment") {
                debug!("{} already sources .mdots/environment", config_file);
                continue;
            }

            if dry_run {
                info!("Would update {} to source .mdots/environment", config_file);
            } else {
                let append_content = if *shell == "fish" {
                    fish_source_line
                } else {
                    source_line
                };

                let mut file = fs::OpenOptions::new().append(true).open(&config_path)?;
                use std::io::Write;
                writeln!(file, "{}", append_content)?;

                println!(
                    "  {} Updated {} to source theming environment",
                    "✓".green(),
                    config_file
                );
            }
        }

        Ok(())
    }
}

/// Check if any theming configuration is present
pub fn has_theming_config(config: &ThemingConfig) -> bool {
    config.cursor.is_some()
        || config.icons.is_some()
        || config.theme.is_some()
        || config.dark_or_light.is_some()
        || config.font.is_some()
        || config.gtk.is_some()
        || config.qt.is_some()
        || !config.env_vars.is_empty()
}

/// Find the actual case-sensitive theme name in a directory (case-insensitive search)
fn find_theme_name_in_dir(dir: &Path, theme_name: &str) -> Option<String> {
    if !dir.exists() {
        return None;
    }

    let theme_name_lower = theme_name.to_lowercase();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                if file_name.to_lowercase() == theme_name_lower {
                    return Some(file_name);
                }
            }
        }
    }

    None
}

/// Find the actual theme name with correct casing (searches user and system dirs)
fn find_theme_name(home: &str, theme_type: &str, theme_name: &str) -> Option<String> {
    let user_dir = PathBuf::from(home).join(format!(".{}", theme_type));
    let system_dir = PathBuf::from("/usr/share").join(theme_type);

    // Check user dir first, then system dir
    find_theme_name_in_dir(&user_dir, theme_name)
        .or_else(|| find_theme_name_in_dir(&system_dir, theme_name))
}

/// Find the actual cursor theme name with correct casing
fn find_cursor_theme_name(home: &str, theme_name: &str) -> Option<String> {
    let user_dir = PathBuf::from(home).join(".icons");
    let system_dir = PathBuf::from("/usr/share/icons");

    find_theme_name_in_dir(&user_dir, theme_name)
        .or_else(|| find_theme_name_in_dir(&system_dir, theme_name))
}

/// List all available themes in user and system directories
fn list_available_themes(home: &str, theme_type: &str) -> Vec<String> {
    let mut themes = Vec::new();
    let user_dir = PathBuf::from(home).join(format!(".{}", theme_type));
    let system_dir = PathBuf::from("/usr/share").join(theme_type);

    for dir in [&user_dir, &system_dir] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(file_name) = entry.file_name().into_string() {
                    // Only include directories (themes are directories)
                    if entry.path().is_dir() && !themes.contains(&file_name) {
                        themes.push(file_name);
                    }
                }
            }
        }
    }

    themes.sort();
    themes
}

/// List all available cursor themes
fn list_available_cursors(home: &str) -> Vec<String> {
    list_available_themes(home, "icons")
}

/// Find a similar theme name using simple string similarity
fn find_similar_theme(target: &str, available: &[String]) -> Option<String> {
    let target_lower = target.to_lowercase();

    // First, check for exact match ignoring case
    for theme in available {
        if theme.to_lowercase() == target_lower {
            return Some(theme.clone());
        }
    }

    // Then check for themes that contain the target or vice versa
    for theme in available {
        let theme_lower = theme.to_lowercase();
        if theme_lower.contains(&target_lower) || target_lower.contains(&theme_lower) {
            return Some(theme.clone());
        }
    }

    None
}

/// Detect Qt backend based on desktop environment
fn detect_qt_backend() -> QtBackend {
    if let Ok(de) = std::env::var("XDG_CURRENT_DESKTOP") {
        let de = de.to_lowercase();
        if de.contains("kde") {
            return QtBackend::Kde;
        }
    }

    // Check if running KDE
    if is_process_running("plasmashell") {
        return QtBackend::Kde;
    }

    QtBackend::Qt5ct
}

/// Check if a process is running
fn is_process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Write an INI-style settings file
fn write_ini_file(path: &Path, settings: &HashMap<String, String>, section: &str) -> Result<()> {
    let mut content = format!("[{}]\n", section);

    for (key, value) in settings {
        content.push_str(&format!("{}={}\n", key, value));
    }

    fs::write(path, content).with_context(|| format!("Failed to write {:?}", path))?;

    Ok(())
}

/// Write a qt5ct/qt6ct config file
fn write_qtct_config(
    path: &Path,
    settings: &HashMap<String, HashMap<String, String>>,
) -> Result<()> {
    let mut content = String::new();

    for (section, values) in settings {
        content.push_str(&format!("[{}]\n", section));
        for (key, value) in values {
            content.push_str(&format!("{}={}\n", key, value));
        }
        content.push('\n');
    }

    fs::write(path, content).with_context(|| format!("Failed to write {:?}", path))?;

    Ok(())
}

/// Report of theming sync operations
#[derive(Debug, Clone, Default)]
pub struct ThemingReport {
    pub gtk_updated: Vec<String>,
    pub qt_updated: Vec<String>,
    pub cursor_updated: bool,
    pub env_vars_updated: bool,
    pub errors: Vec<ThemingError>,
}

impl ThemingReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_changes(&self) -> bool {
        !self.gtk_updated.is_empty()
            || !self.qt_updated.is_empty()
            || self.cursor_updated
            || self.env_vars_updated
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Error information for theming operations
#[derive(Debug, Clone)]
pub struct ThemingError {
    pub component: String,
    pub message: String,
}
