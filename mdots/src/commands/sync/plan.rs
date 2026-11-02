//! Sync planning: what to install and what `--prune` would remove, and the
//! plan report. Pure functions over declared/installed/state sets.

use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::config::{Config, ConfigPaths, PackageType};
use crate::package::Package;

use super::*;

/// Native packages mdots must never auto-remove during `--prune`, even if they
/// fall out of the declared set. Removing any of these can make the system
/// unbootable or break the package manager itself.
pub(super) const PROTECTED_PACKAGES: &[&str] = &[
    "base",
    "filesystem",
    "glibc",
    "gcc-libs",
    "bash",
    "coreutils",
    "shadow",
    "util-linux",
    "systemd",
    "systemd-libs",
    "pacman",
    "sudo",
    "linux",
    "linux-lts",
    "linux-zen",
    "linux-hardened",
    "linux-firmware",
    "mkinitcpio",
    "grub",
    "systemd-boot",
    "efibootmgr",
    "mdots",
];

/// Split a prune removal list into (removable, protected-skipped), so critical
/// system packages are never auto-removed during `--prune`.
pub(super) fn partition_protected(names: Vec<String>) -> (Vec<String>, Vec<String>) {
    names
        .into_iter()
        .partition(|n| !PROTECTED_PACKAGES.contains(&n.as_str()))
}

/// From the declared packages, pick those not yet installed — the install set.
/// Returns `(native, flatpak)`. Nix packages are applied by home-manager, so
/// they are skipped here.
pub(crate) fn compute_installable(
    declared: &[Package],
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut native = Vec::new();
    let mut flatpak = Vec::new();
    for pkg in declared {
        match pkg.package_type {
            PackageType::Flatpak => {
                if !installed_flatpak.contains(&pkg.name) {
                    flatpak.push(pkg.name.clone());
                }
            }
            PackageType::Native => {
                if !installed_native.contains_key(&pkg.name) {
                    native.push(pkg.name.clone());
                }
            }
            PackageType::Nix => {}
        }
    }
    (native, flatpak)
}

/// From the recorded state, pick packages that are no longer declared but are
/// still installed — the candidates for `--prune`. Returns `(native, flatpak)`.
/// A package keeps being managed if it is still declared; one that is gone from
/// config but already uninstalled needs no action. Unknown/absent `type` is
/// treated as native (pacman). Protected-package filtering is applied by the
/// caller via [`partition_protected`].
pub(super) fn compute_prunable(
    state_packages: &[StatePackage],
    declared_names: &HashSet<String>,
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut native = Vec::new();
    let mut flatpak = Vec::new();
    for pkg in state_packages {
        if declared_names.contains(&pkg.name) {
            continue;
        }
        match pkg.pkg_type.as_deref() {
            Some("flatpak") => {
                if installed_flatpak.contains(&pkg.name) {
                    flatpak.push(pkg.name.clone());
                }
            }
            _ => {
                if installed_native.contains_key(&pkg.name) {
                    native.push(pkg.name.clone());
                }
            }
        }
    }
    (native, flatpak)
}

/// Pure prune-preview: from the recorded state packages, the set that a
/// `--prune` would actually remove — undeclared-but-still-installed packages
/// with critical system packages filtered out (they are never auto-removed).
/// Returns `(native_to_remove, flatpak_to_remove)`. Used by the read-only TUI
/// preview via [`compute_prune_preview`]; the real sync path calls
/// `compute_prunable`/`partition_protected` directly so it can also report the
/// protected-package skip warning that this helper discards.
pub(super) fn prune_set_filtered(
    state_packages: &[StatePackage],
    declared_names: &HashSet<String>,
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let (native, flatpak) = compute_prunable(
        state_packages,
        declared_names,
        installed_native,
        installed_flatpak,
    );
    let (native, _protected_skipped) = partition_protected(native);
    (native, flatpak)
}

/// Read-only prune preview for the TUI sync screen. Loads the state file
/// (best-effort — a missing/unreadable state file means "nothing tracked", so
/// nothing to prune) and computes the prune set via [`prune_set_filtered`].
/// Returns `(native_to_remove, flatpak_to_remove)`. Never mutates anything.
pub(crate) fn compute_prune_preview(
    paths: &ConfigPaths,
    declared_names: &HashSet<String>,
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    match load_state_file(paths) {
        Ok(state) => prune_set_filtered(
            &state.packages,
            declared_names,
            installed_native,
            installed_flatpak,
        ),
        Err(_) => (Vec::new(), Vec::new()),
    }
}

/// The set of package changes a sync will apply, after diffing declared vs
/// installed and filtering protected packages. Bundles the four lists that were
/// previously threaded through the sync functions individually.
#[derive(Default)]
pub(super) struct SyncPlan {
    pub(super) to_install: Vec<String>,
    pub(super) to_remove: Vec<String>,
    pub(super) flatpak_to_install: Vec<String>,
    pub(super) flatpak_to_remove: Vec<String>,
    /// The `remote:` of each Flatpak to install that names its own (else the host's `flatpak_remote` / flathub applies).
    pub(super) flatpak_remotes: HashMap<String, String>,
}

impl SyncPlan {
    /// True when there are no package changes to apply.
    pub(super) fn is_empty(&self) -> bool {
        self.to_install.is_empty()
            && self.to_remove.is_empty()
            && self.flatpak_to_install.is_empty()
            && self.flatpak_to_remove.is_empty()
    }
}

/// Print the computed package-change plan — JSON for `--json`, otherwise a
/// human-readable summary. In dry-run mode the human path also previews service
/// changes. Returns `true` when this was a dry run and the caller should stop
/// before mutating anything.
pub(super) fn report_sync_plan(
    paths: &ConfigPaths,
    config: &Config,
    plan: &SyncPlan,
    dry_run: bool,
    json: bool,
) -> Result<bool> {
    if json {
        let output = SyncOutput {
            success: true,
            dry_run,
            summary: SyncSummary {
                to_install: plan.to_install.len(),
                to_remove: plan.to_remove.len(),
                flatpak_to_install: plan.flatpak_to_install.len(),
                flatpak_to_remove: plan.flatpak_to_remove.len(),
            },
            actions: SyncActions {
                install: plan.to_install.clone(),
                remove: plan.to_remove.clone(),
                flatpak_install: plan.flatpak_to_install.clone(),
                flatpak_remove: plan.flatpak_to_remove.clone(),
            },
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(dry_run);
    }

    // Human-readable plan
    let installs = plan.to_install.len() + plan.flatpak_to_install.len();
    let removes = plan.to_remove.len() + plan.flatpak_to_remove.len();

    if plan.is_empty() {
        crate::ui::step("Resolved", "packages already in sync");
    } else {
        crate::ui::step(
            "Resolved",
            &format!("{} to install, {} to remove", installs, removes),
        );
        for pkg in plan.to_install.iter().chain(plan.flatpak_to_install.iter()) {
            crate::ui::detail(&format!("+ {}", pkg));
        }
        for pkg in plan.to_remove.iter().chain(plan.flatpak_to_remove.iter()) {
            crate::ui::detail(&format!("− {}", pkg));
        }
    }

    if dry_run {
        // Preview service changes
        match preview_services(paths, config) {
            Ok(service_preview) => {
                if service_preview.has_changes() {
                    let categories = [
                        ("would enable", &service_preview.services_to_enable),
                        ("would start", &service_preview.services_to_start),
                        ("would stop", &service_preview.services_to_stop),
                        ("would disable", &service_preview.services_to_disable),
                    ];
                    for (label, list) in categories {
                        if !list.is_empty() {
                            crate::ui::note("Services", &format!("{} {}", label, list.join(", ")));
                        }
                    }
                } else {
                    crate::ui::step("Services", "already in sync");
                }
            }
            Err(e) => {
                log::warn!("Failed to preview service changes: {}", e);
            }
        }
        return Ok(true);
    }

    Ok(false)
}
