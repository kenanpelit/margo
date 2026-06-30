use anyhow::Result;
use colored::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::config::ConfigPaths;
use crate::package::{Package, PackageManager};

/// The package drift between what is declared and what is installed / tracked.
///
/// - `native_to_install` / `flatpak_to_install`: declared but not yet installed.
/// - `native_to_remove` / `flatpak_to_remove`: tracked in state, no longer
///   declared, still installed, and not a protected system package.  These are
///   what a `mdots sync --prune` would remove.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Drift {
    pub native_to_install: Vec<String>,
    pub flatpak_to_install: Vec<String>,
    pub native_to_remove: Vec<String>,
    pub flatpak_to_remove: Vec<String>,
}

impl Drift {
    /// True when declared packages exactly match what is installed (no action
    /// needed).
    pub fn is_in_sync(&self) -> bool {
        self.native_to_install.is_empty()
            && self.flatpak_to_install.is_empty()
            && self.native_to_remove.is_empty()
            && self.flatpak_to_remove.is_empty()
    }

    /// Total number of packages that would be installed by a sync.
    pub fn install_count(&self) -> usize {
        self.native_to_install.len() + self.flatpak_to_install.len()
    }

    /// Total number of packages that would be removed by a `sync --prune`.
    pub fn remove_count(&self) -> usize {
        self.native_to_remove.len() + self.flatpak_to_remove.len()
    }
}

/// Pure drift computation from already-loaded data.  Used by unit tests and by
/// [`compute_drift`], which adds the disk-load step.
///
/// `prunable_native` / `prunable_flatpak` are the already-filtered (protected
/// packages excluded) lists returned by
/// [`crate::commands::sync::compute_prune_preview`].
pub(crate) fn compute_drift_from_parts(
    declared: &[Package],
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
    prunable_native: Vec<String>,
    prunable_flatpak: Vec<String>,
) -> Drift {
    let (native_to_install, flatpak_to_install) =
        crate::commands::sync::compute_installable(declared, installed_native, installed_flatpak);
    Drift {
        native_to_install,
        flatpak_to_install,
        native_to_remove: prunable_native,
        flatpak_to_remove: prunable_flatpak,
    }
}

/// Compute the full drift between declared packages and the installed+tracked
/// state.  Read-only: loads the state file and package lists but never mutates
/// anything.
pub(crate) fn compute_drift(
    paths: &ConfigPaths,
    declared: &[Package],
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> Drift {
    let declared_names: HashSet<String> = declared.iter().map(|p| p.name.clone()).collect();
    let (prunable_native, prunable_flatpak) = crate::commands::sync::compute_prune_preview(
        paths,
        &declared_names,
        installed_native,
        installed_flatpak,
    );
    compute_drift_from_parts(
        declared,
        installed_native,
        installed_flatpak,
        prunable_native,
        prunable_flatpak,
    )
}

/// `mdots diff` — print a colorized declared-vs-installed diff.  Read-only.
pub fn run(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = crate::config::load_config(paths)?;
    let pkg_manager = PackageManager::new(paths.clone());
    let declared = pkg_manager.get_declared_packages(&config)?;

    let installed_native = pkg_manager
        .get_installed_native_packages(&config)
        .unwrap_or_default();

    let installed_flatpak: HashSet<String> = pkg_manager
        .get_installed_flatpaks(config.flatpak_scope.as_arg())
        .unwrap_or_default()
        .into_iter()
        .collect();

    let drift = compute_drift(paths, &declared, &installed_native, &installed_flatpak);

    if json {
        println!("{}", serde_json::to_string_pretty(&drift)?);
        return Ok(());
    }

    if drift.is_in_sync() {
        println!("{}", "✓ in sync — no package changes needed".green().bold());
        return Ok(());
    }

    // --- packages to install ---
    if !drift.native_to_install.is_empty() {
        println!(
            "{}",
            format!(
                "Native packages to install ({}):",
                drift.native_to_install.len()
            )
            .cyan()
            .bold()
        );
        let mut sorted = drift.native_to_install.clone();
        sorted.sort();
        for pkg in &sorted {
            println!("  {} {}", "+".green().bold(), pkg.green());
        }
        println!();
    }
    if !drift.flatpak_to_install.is_empty() {
        println!(
            "{}",
            format!(
                "Flatpak packages to install ({}):",
                drift.flatpak_to_install.len()
            )
            .cyan()
            .bold()
        );
        let mut sorted = drift.flatpak_to_install.clone();
        sorted.sort();
        for pkg in &sorted {
            println!("  {} {}", "+".green().bold(), pkg.green());
        }
        println!();
    }

    // --- packages to remove ---
    if !drift.native_to_remove.is_empty() {
        println!(
            "{}",
            format!(
                "Native packages to remove ({}):",
                drift.native_to_remove.len()
            )
            .cyan()
            .bold()
        );
        let mut sorted = drift.native_to_remove.clone();
        sorted.sort();
        for pkg in &sorted {
            println!("  {} {}", "-".red().bold(), pkg.red());
        }
        println!();
    }
    if !drift.flatpak_to_remove.is_empty() {
        println!(
            "{}",
            format!(
                "Flatpak packages to remove ({}):",
                drift.flatpak_to_remove.len()
            )
            .cyan()
            .bold()
        );
        let mut sorted = drift.flatpak_to_remove.clone();
        sorted.sort();
        for pkg in &sorted {
            println!("  {} {}", "-".red().bold(), pkg.red());
        }
        println!();
    }

    // --- summary ---
    let mut parts: Vec<String> = Vec::new();
    if drift.install_count() > 0 {
        parts.push(
            format!("{} to install", drift.install_count())
                .green()
                .to_string(),
        );
    }
    if drift.remove_count() > 0 {
        parts.push(
            format!("{} to remove", drift.remove_count())
                .red()
                .to_string(),
        );
    }
    println!("Summary: {}", parts.join(", "));
    println!("{}", "Run 'mdots sync' to apply.".dimmed());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PackageType;
    use crate::package::Package;

    fn pkg(name: &str, pkg_type: PackageType) -> Package {
        Package {
            name: name.to_string(),
            package_type: pkg_type,
        }
    }

    fn native_map(names: &[&str]) -> HashMap<String, String> {
        names
            .iter()
            .map(|&n| (n.to_string(), "1.0".to_string()))
            .collect()
    }

    fn flatpak_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|&n| n.to_string()).collect()
    }

    /// Declared not installed → add set.
    #[test]
    fn compute_drift_install_side() {
        let declared = vec![
            pkg("vim", PackageType::Native),
            pkg("git", PackageType::Native),
            pkg("htop", PackageType::Native),
        ];
        let installed = native_map(&["vim", "git"]);
        let drift =
            compute_drift_from_parts(&declared, &installed, &flatpak_set(&[]), vec![], vec![]);
        assert_eq!(drift.native_to_install, vec!["htop"]);
        assert!(drift.flatpak_to_install.is_empty());
        assert!(drift.native_to_remove.is_empty());
        assert!(!drift.is_in_sync());
    }

    /// Tracked-in-state but undeclared → remove set (protected excluded by
    /// `compute_prune_preview` before reaching here).
    #[test]
    fn compute_drift_remove_side() {
        let declared = vec![pkg("vim", PackageType::Native)];
        let installed = native_map(&["vim", "old-pkg"]);
        // Synthetic: "old-pkg" was tracked in state, still installed, no longer
        // declared.  The protection filter has already been applied upstream.
        let drift = compute_drift_from_parts(
            &declared,
            &installed,
            &flatpak_set(&[]),
            vec!["old-pkg".to_string()],
            vec![],
        );
        assert!(drift.native_to_install.is_empty());
        assert_eq!(drift.native_to_remove, vec!["old-pkg"]);
        assert_eq!(drift.remove_count(), 1);
        assert!(!drift.is_in_sync());
    }

    /// Declared == installed → in sync.
    #[test]
    fn compute_drift_in_sync() {
        let declared = vec![pkg("vim", PackageType::Native)];
        let installed = native_map(&["vim"]);
        let drift =
            compute_drift_from_parts(&declared, &installed, &flatpak_set(&[]), vec![], vec![]);
        assert!(drift.is_in_sync());
        assert_eq!(drift.install_count(), 0);
        assert_eq!(drift.remove_count(), 0);
    }

    /// Flatpak declared but not installed → flatpak install side.
    #[test]
    fn compute_drift_flatpak_install_side() {
        let declared = vec![
            pkg("com.spotify.Client", PackageType::Flatpak),
            pkg("com.github.tchx84.Flatseal", PackageType::Flatpak),
        ];
        let fp = flatpak_set(&["com.spotify.Client"]);
        let drift = compute_drift_from_parts(&declared, &native_map(&[]), &fp, vec![], vec![]);
        assert_eq!(drift.flatpak_to_install, vec!["com.github.tchx84.Flatseal"]);
        assert!(drift.native_to_install.is_empty());
        assert!(!drift.is_in_sync());
        assert_eq!(drift.install_count(), 1);
    }
}
