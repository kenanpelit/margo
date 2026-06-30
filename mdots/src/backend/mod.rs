pub mod pacman;

use anyhow::Result;
use std::collections::HashMap;

use crate::config::{Config, PackageManagerType};

/// Trait abstracting native package manager operations.
/// Flatpak operations are handled separately (same on all distros).
pub trait PkgBackend {
    /// Install packages non-interactively (batch mode, used by sync)
    fn install_packages_batch(&self, packages: &[&str]) -> Result<bool>;

    /// Install a single package interactively (used by `dcli install`)
    fn install_interactive(&self, package: &str) -> Result<bool>;

    /// Remove packages non-interactively (batch mode, used by sync prune)
    fn remove_packages_batch(&self, packages: &[&str]) -> Result<bool>;

    /// Remove a single package interactively (used by `dcli remove`)
    fn remove_interactive(&self, package: &str) -> Result<bool>;

    /// Refresh the package database
    fn refresh_db(&self) -> Result<()>;

    /// Full system update (interactive)
    fn system_update(&self, devel: bool) -> Result<bool>;

    /// Get all installed packages with their versions
    fn get_installed_packages(&self) -> Result<HashMap<String, String>>;

    /// Get explicitly installed package names (not dependencies)
    fn get_explicit_packages(&self) -> Result<Vec<String>>;

    /// Get all installed package names (including dependencies)
    /// Default implementation falls back to get_explicit_packages
    fn get_all_packages(&self) -> Result<Vec<String>> {
        self.get_explicit_packages()
    }

    /// Get the installed version of a package
    fn get_package_version(&self, package: &str) -> Result<Option<String>>;

    /// Check if a package is available in repos
    fn is_available(&self, package: &str) -> bool;

    /// Check package info (for validation)
    fn check_package_exists(&self, package: &str) -> bool;

    /// Get the package info command for fzf preview
    fn package_info_command(&self) -> &str;
}

/// Create a PkgBackend instance based on the resolved package manager type.
pub fn create_backend(config: &Config) -> Result<Box<dyn PkgBackend>> {
    let pm_type = crate::config::resolve_package_manager(config)?;

    match pm_type {
        PackageManagerType::Pacman => {
            let aur_helper = crate::config::resolve_aur_helper(config)?;
            Ok(Box::new(pacman::PacmanBackend::new(aur_helper)))
        }
    }
}
