//! Resolve the editor, package manager and AUR helper from the config and
//! the system.

use super::*;

/// Resolve the editor to use for editing config files
/// Resolution order: 1) config.editor, 2) $EDITOR env var, 3) fallback (nano or vim)
pub fn resolve_editor(config: &Config) -> Result<String> {
    // Priority 1: Config file setting
    if let Some(ref editor) = config.editor {
        return Ok(editor.clone());
    }

    // Priority 2: $EDITOR environment variable
    if let Ok(editor) = std::env::var("EDITOR")
        && !editor.trim().is_empty()
    {
        return Ok(editor);
    }

    // Priority 3: Fallback to commonly available editors
    if which::which("nano").is_ok() {
        return Ok("nano".to_string());
    }

    if which::which("vim").is_ok() {
        return Ok("vim".to_string());
    }

    anyhow::bail!(
        "No editor found. Please set 'editor' in your config.yaml or set the $EDITOR environment variable."
    )
}

/// Resolve the effective package manager type from config or auto-detect from the system.
/// Resolution order: 1) config.package_manager, 2) auto-detect from /etc/os-release
pub fn resolve_package_manager(config: &Config) -> Result<PackageManagerType> {
    // Priority 1: Config file setting
    if let Some(ref pm) = config.package_manager {
        return Ok(pm.clone());
    }

    // Priority 2: Auto-detect from system
    detect_package_manager_type()
}

/// Detect the package manager type from the system
pub fn detect_package_manager_type() -> Result<PackageManagerType> {
    // Check /etc/os-release for distro family
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let id = content
            .lines()
            .find(|l| l.starts_with("ID="))
            .map(|l| l.trim_start_matches("ID=").trim_matches('"').to_lowercase());
        let id_like = content
            .lines()
            .find(|l| l.starts_with("ID_LIKE="))
            .map(|l| {
                l.trim_start_matches("ID_LIKE=")
                    .trim_matches('"')
                    .to_lowercase()
            });

        if let Some(ref id) = id {
            // Direct ID matches for Arch and Arch-based distros
            if matches!(
                id.as_str(),
                "arch" | "manjaro" | "endeavouros" | "garuda" | "cachyos" | "artix"
            ) {
                return Ok(PackageManagerType::Pacman);
            }
        }

        // Check ID_LIKE for derivative distros
        if let Some(ref id_like) = id_like
            && id_like.contains("arch")
        {
            return Ok(PackageManagerType::Pacman);
        }
    }

    // Fallback: check that pacman exists
    if which::which("pacman").is_ok() {
        return Ok(PackageManagerType::Pacman);
    }

    anyhow::bail!(
        "mdots officially supports only Arch and Arch-based distributions (pacman). pacman was not found on this system. If you are building mdots on another distro, ensure pacman is available."
    )
}

/// Resolve the AUR helper to use for package management
/// Resolution order: 1) config.aur_helper, 2) auto-detect (paru, then yay)
pub fn resolve_aur_helper(config: &Config) -> Result<String> {
    // Priority 1: Config file setting
    if let Some(ref aur_helper) = config.aur_helper {
        // Validate the configured helper exists
        if which::which(aur_helper).is_ok() {
            return Ok(aur_helper.clone());
        } else {
            anyhow::bail!(
                "Configured AUR helper '{}' not found in PATH. Please install it or update your config.",
                aur_helper
            );
        }
    }

    // Priority 2: Auto-detect (prefer paru, then yay)
    if which::which("paru").is_ok() {
        return Ok("paru".to_string());
    }

    if which::which("yay").is_ok() {
        return Ok("yay".to_string());
    }

    anyhow::bail!(
        "No AUR helper found. Please install paru or yay, or set 'aur_helper' in your config.yaml."
    )
}
