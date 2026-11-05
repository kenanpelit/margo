//! Detect the user's tools (backup tool, AUR helper, default applications)
//! for the generated templates.

use super::*;

/// Detect which backup tool is installed
pub(super) fn detect_backup_tool() -> String {
    // Check for timeshift first (most common)
    if which::which("timeshift").is_ok() {
        println!(
            "  {} Detected backup tool: {}",
            "→".blue(),
            "timeshift".green()
        );
        return "timeshift".to_string();
    }

    // Check for snapper
    if which::which("snapper").is_ok() {
        println!(
            "  {} Detected backup tool: {}",
            "→".blue(),
            "snapper".green()
        );
        return "snapper".to_string();
    }

    // Default to timeshift if neither is found
    println!(
        "  {} No backup tool detected, defaulting to: {}",
        "→".yellow(),
        "timeshift".cyan()
    );
    println!("    Install with: sudo pacman -S timeshift");
    println!("    Or: sudo pacman -S snapper");
    "timeshift".to_string()
}

/// Detect which AUR helper is installed
pub(super) fn detect_aur_helper() -> String {
    // Check for paru first (preferred)
    if which::which("paru").is_ok() {
        return "paru".to_string();
    }

    // Check for yay
    if which::which("yay").is_ok() {
        return "yay".to_string();
    }

    // Default to paru if neither is found
    "paru".to_string()
}

/// Detect default applications using xdg-mime
pub(super) fn detect_default_apps() -> DefaultApps {
    DefaultApps {
        browser: detect_xdg_default("x-scheme-handler/http")
            .unwrap_or_else(|| "firefox".to_string()),
        terminal: detect_terminal().unwrap_or_else(|| "kitty".to_string()),
        text_editor: detect_xdg_default("text/plain").unwrap_or_else(|| "code".to_string()),
        file_manager: detect_xdg_default("inode/directory").unwrap_or_else(|| "thunar".to_string()),
    }
}

pub(super) struct DefaultApps {
    pub(super) browser: String,
    pub(super) terminal: String,
    pub(super) text_editor: String,
    pub(super) file_manager: String,
}

/// Query xdg-mime for a default application
pub(super) fn detect_xdg_default(mime_type: &str) -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime_type])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let desktop_file = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if desktop_file.is_empty() {
        return None;
    }

    // Strip .desktop extension and org.* prefix for cleaner names
    let name = desktop_file
        .strip_suffix(".desktop")
        .unwrap_or(&desktop_file)
        .to_string();

    // Handle org.kde.* and org.gnome.* prefixes
    let name = if let Some(stripped) = name.strip_prefix("org.kde.") {
        stripped.to_string()
    } else if let Some(stripped) = name.strip_prefix("org.gnome.") {
        stripped.to_string()
    } else {
        name
    };

    Some(name)
}

/// Detect default terminal (terminals don't have a standard MIME type)
pub(super) fn detect_terminal() -> Option<String> {
    // Check $TERMINAL environment variable first
    if let Ok(terminal) = std::env::var("TERMINAL")
        && !terminal.is_empty()
    {
        // Extract just the binary name
        let name = std::path::Path::new(&terminal)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&terminal);
        return Some(name.to_string());
    }

    // Check for common terminals in order of preference
    let terminals = [
        "kitty",
        "alacritty",
        "foot",
        "wezterm",
        "konsole",
        "gnome-terminal",
        "xfce4-terminal",
        "xterm",
    ];

    for term in terminals {
        if which::which(term).is_ok() {
            return Some(term.to_string());
        }
    }

    None
}
