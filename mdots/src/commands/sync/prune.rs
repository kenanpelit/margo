//! `--prune` on the sequential path: remove packages tracked in state that are
//! no longer declared.

use anyhow::{Context, Result};
use colored::*;
use std::collections::{HashMap, HashSet};

use crate::config::{Config, ConfigPaths, FlatpakScope};
use crate::package::PackageManager;

use super::*;

/// Prune packages tracked in state but no longer declared (sequential path):
/// compute the candidates (protecting critical packages), then remove them —
/// native via the backend, flatpaks via `flatpak uninstall`.
pub(super) fn prune_undeclared_packages(paths: &ConfigPaths, config: &Config) -> Result<()> {
    let pkg_manager = PackageManager::new(paths.clone());
    let declared = pkg_manager.get_declared_packages(config)?;

    crate::ui::step("Pruning", "undeclared packages");

    // Load state file to find packages from disabled modules. An unreadable
    // state file is a hard error: pruning against a guess is not safe.
    let state = load_prune_state(paths)?;
    let declared_names: HashSet<String> = declared.iter().map(|p| p.name.clone()).collect();

    // Get currently installed packages
    let installed_pacman = pkg_manager.get_installed_native_packages(config)?;
    let installed_pacman_map: HashMap<String, String> = installed_pacman.into_iter().collect();

    let flatpak_scope_str = match config.flatpak_scope {
        FlatpakScope::User => "--user",
        FlatpakScope::System => "--system",
    };
    let installed_flatpaks = pkg_manager.get_installed_flatpaks(flatpak_scope_str)?;
    let installed_flatpak_set: HashSet<String> = installed_flatpaks.into_iter().collect();

    // Packages tracked in state but no longer declared (and still
    // installed) are prune candidates.
    let (pacman_to_remove, flatpak_to_remove) = compute_prunable(
        &state.packages,
        &declared_names,
        &installed_pacman_map,
        &installed_flatpak_set,
    );

    // Never auto-remove critical system packages, even if a disabled
    // module used to track them.
    let (pacman_to_remove, protected_skipped) = partition_protected(pacman_to_remove);
    if !protected_skipped.is_empty() {
        println!(
            "  {} Skipping protected package{} (remove manually if intended): {}",
            "⚠".yellow(),
            if protected_skipped.len() == 1 {
                ""
            } else {
                "s"
            },
            protected_skipped.join(", ")
        );
    }

    // Remove packages
    if !pacman_to_remove.is_empty() {
        let backend = crate::backend::create_backend(config)?;
        println!(
            "  {} Removing {} native package{} from disabled modules: {}",
            "→".blue(),
            pacman_to_remove.len(),
            if pacman_to_remove.len() == 1 { "" } else { "s" },
            pacman_to_remove.join(", ")
        );

        let pkg_refs: Vec<&str> = pacman_to_remove.iter().map(|s| s.as_str()).collect();
        let success = backend.remove_packages_batch(&pkg_refs)?;

        if !success {
            crate::ui::warn("Pruning", "failed to remove some packages");
        }
    }

    if !flatpak_to_remove.is_empty() {
        println!(
            "  {} Removing {} flatpak{} from disabled modules: {}",
            "→".blue(),
            flatpak_to_remove.len(),
            if flatpak_to_remove.len() == 1 {
                ""
            } else {
                "s"
            },
            flatpak_to_remove.join(", ")
        );

        let scope_flag = match config.flatpak_scope {
            FlatpakScope::User => "--user",
            FlatpakScope::System => "--system",
        };

        for pkg in &flatpak_to_remove {
            let status = std::process::Command::new("flatpak")
                .args(["uninstall", "-y", scope_flag, pkg])
                .status()
                .context(format!("Failed to remove flatpak: {}", pkg))?;

            if !status.success() {
                crate::ui::warn("Pruning", &format!("failed to remove {}", pkg));
            }
        }
    }

    if pacman_to_remove.is_empty() && flatpak_to_remove.is_empty() {
        crate::ui::detail("nothing to prune");
    }

    Ok(())
}
