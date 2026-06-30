use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::{
    load_config, Config, ConfigPaths, FlatpakScope, ModuleProcessing, PackageType, ServicesConfig,
};
use crate::defaults::{
    create_updated_state as create_updated_defaults_state, load_defaults_state,
    save_defaults_state, DefaultsManager,
};
use crate::package::{Package, PackageManager};
use crate::service_profile::ServiceProfileManager;
use crate::services::{
    create_updated_state, load_services_state, save_services_state, ServiceManager, ServicesPreview,
};
use crate::theming::state::{create_theming_state, load_theming_state, save_theming_state};
use crate::theming::ThemingManager;

/// Native packages mdots must never auto-remove during `--prune`, even if they
/// fall out of the declared set. Removing any of these can make the system
/// unbootable or break the package manager itself.
const PROTECTED_PACKAGES: &[&str] = &[
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
fn partition_protected(names: Vec<String>) -> (Vec<String>, Vec<String>) {
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
fn compute_prunable(
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
fn prune_set_filtered(
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
struct SyncPlan {
    to_install: Vec<String>,
    to_remove: Vec<String>,
    flatpak_to_install: Vec<String>,
    flatpak_to_remove: Vec<String>,
}

impl SyncPlan {
    /// True when there are no package changes to apply.
    fn is_empty(&self) -> bool {
        self.to_install.is_empty()
            && self.to_remove.is_empty()
            && self.flatpak_to_install.is_empty()
            && self.flatpak_to_remove.is_empty()
    }
}

#[derive(Serialize)]
struct SyncOutput {
    success: bool,
    dry_run: bool,
    summary: SyncSummary,
    actions: SyncActions,
}

#[derive(Serialize)]
struct SyncSummary {
    to_install: usize,
    to_remove: usize,
    flatpak_to_install: usize,
    flatpak_to_remove: usize,
}

#[derive(Serialize)]
struct SyncActions {
    install: Vec<String>,
    remove: Vec<String>,
    flatpak_install: Vec<String>,
    flatpak_remove: Vec<String>,
}

/// State tracking for sequential module processing
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModulesState {
    #[serde(default = "Utc::now")]
    last_updated: DateTime<Utc>,

    #[serde(default)]
    completed_modules: HashSet<String>,
}

impl Default for ModulesState {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            completed_modules: HashSet::new(),
        }
    }
}

/// Summary of package installation results
#[derive(Debug, Clone)]
struct InstallationSummary {
    succeeded: Vec<String>,
    failed: Vec<(String, String)>, // (package_name, error_message)
}

impl InstallationSummary {
    fn new() -> Self {
        Self {
            succeeded: Vec::new(),
            failed: Vec::new(),
        }
    }
}

fn load_modules_state(state_file: &Path) -> Result<ModulesState> {
    if !state_file.exists() {
        return Ok(ModulesState::default());
    }

    let content =
        std::fs::read_to_string(state_file).context("Failed to read modules state file")?;

    let state: ModulesState =
        serde_yaml::from_str(&content).context("Failed to parse modules state YAML")?;

    Ok(state)
}

fn save_modules_state(state_file: &Path, state: &ModulesState) -> Result<()> {
    if let Some(parent) = state_file.parent() {
        std::fs::create_dir_all(parent).context("Failed to create state directory")?;
    }

    let yaml = serde_yaml::to_string(state).context("Failed to serialize modules state")?;

    std::fs::write(state_file, yaml).context("Failed to write modules state file")?;

    Ok(())
}

// ===== Sequential Module Processing Helper Functions =====

/// Load a module by name, searching in modules directory
fn load_module_by_name(
    paths: &ConfigPaths,
    module_name: &str,
) -> Result<crate::config::ModuleStructure> {
    use crate::config::load_module;

    let modules_dir = paths.modules_dir();
    let module_yaml = modules_dir.join(format!("{}.yaml", module_name));
    let module_lua = modules_dir.join(format!("{}.lua", module_name));
    let module_dir = modules_dir.join(module_name);

    let module_path = if module_dir.exists()
        && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
    {
        module_dir
    } else if module_yaml.exists() {
        module_yaml
    } else if module_lua.exists() {
        module_lua
    } else {
        return Err(anyhow!("Module '{}' not found", module_name));
    };

    load_module(&module_path)
}

/// Execute a module hook script with error handling
fn execute_module_hook(
    paths: &ConfigPaths,
    module: &crate::config::ModuleStructure,
    hook_script: &str,
    module_name: &str,
    is_pre_install: bool,
    json: bool,
    run_as_user: Option<String>,
) -> Result<()> {
    // Resolve hook path
    let hook_path = if std::path::Path::new(hook_script).is_absolute() {
        std::path::PathBuf::from(hook_script)
    } else if module.is_directory() || module.is_lua() {
        module.root_dir().join(hook_script)
    } else {
        paths.config_dir.join(hook_script)
    };

    if !hook_path.exists() {
        return Err(anyhow!("Hook script not found: {:?}", hook_path));
    }

    let hook_type = if is_pre_install {
        "pre-install"
    } else {
        "post-install"
    };

    // Execute hook based on run_as_user configuration
    let status = match run_as_user {
        Some(username) => {
            // Run as specified user using sudo -u
            std::process::Command::new("sudo")
                .args([
                    "--preserve-env=MDOTS_LOG_INDENT",
                    "-u",
                    &username,
                    "bash",
                    hook_path.to_str().unwrap(),
                ])
                .env("MDOTS_LOG_INDENT", " ".repeat(crate::ui::DETAIL_INDENT))
                .stdin(std::process::Stdio::inherit())
                .stdout(if json {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::inherit()
                })
                .stderr(if json {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::inherit()
                })
                .status()
                .context(format!(
                    "Failed to execute {} hook for {}",
                    hook_type, module_name
                ))?
        }
        None => {
            // Run with sudo as root (default)
            std::process::Command::new("sudo")
                .args([
                    "--preserve-env=MDOTS_LOG_INDENT",
                    "bash",
                    hook_path.to_str().unwrap(),
                ])
                .env("MDOTS_LOG_INDENT", " ".repeat(crate::ui::DETAIL_INDENT))
                .stdin(std::process::Stdio::inherit())
                .stdout(if json {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::inherit()
                })
                .stderr(if json {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::inherit()
                })
                .status()
                .context(format!(
                    "Failed to execute {} hook for {}",
                    hook_type, module_name
                ))?
        }
    };

    if !status.success() {
        return Err(anyhow!(
            "Hook {} failed for module {} (exit code: {})",
            hook_type,
            module_name,
            status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

/// Install packages from a single module
fn install_module_packages(
    paths: &ConfigPaths,
    packages: &[Package],
    flatpak_scope: &FlatpakScope,
) -> Result<InstallationSummary> {
    let mut summary = InstallationSummary::new();

    if packages.is_empty() {
        return Ok(summary);
    }

    // Separate native and flatpak packages
    let (native_pkgs, flatpak_pkgs): (Vec<_>, Vec<_>) = packages
        .iter()
        .partition(|p| matches!(p.package_type, PackageType::Native));

    // Install native packages via backend
    if !native_pkgs.is_empty() {
        let pkg_names: Vec<String> = native_pkgs.iter().map(|p| p.name.clone()).collect();
        let config = load_config(paths)?;
        let backend = crate::backend::create_backend(&config)?;

        let pkg_refs: Vec<&str> = pkg_names.iter().map(|s| s.as_str()).collect();
        let success = backend.install_packages_batch(&pkg_refs)?;

        if !success {
            // On failure, mark all native packages as failed
            let error_msg = "Batch installation failed".to_string();
            for pkg in native_pkgs {
                summary.failed.push((pkg.name.clone(), error_msg.clone()));
            }
        } else {
            // On success, mark all native packages as succeeded
            for pkg in native_pkgs {
                summary.succeeded.push(pkg.name.clone());
            }
        }
    }

    // Install flatpak packages
    if !flatpak_pkgs.is_empty() {
        let scope_flag = match flatpak_scope {
            FlatpakScope::User => "--user",
            FlatpakScope::System => "--system",
        };

        for pkg in flatpak_pkgs {
            let status = std::process::Command::new("flatpak")
                .args(["install", "-y", scope_flag, "flathub", &pkg.name])
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .context(format!("Failed to install flatpak: {}", pkg.name))?;

            if !status.success() {
                let error_msg = format!(
                    "Installation failed (exit code: {})",
                    status.code().unwrap_or(-1)
                );
                summary.failed.push((pkg.name.clone(), error_msg));
            } else {
                summary.succeeded.push(pkg.name.clone());
            }
        }
    }

    Ok(summary)
}

/// Automatically stage and commit changes to git repository
fn auto_commit_changes(paths: &ConfigPaths, json: bool) -> anyhow::Result<()> {
    use std::process::Command;

    // Check if the config directory is a git repository
    let git_dir = paths.config_dir.join(".git");
    if !git_dir.exists() {
        anyhow::bail!("Not a git repository");
    }

    // Check if there are any changes to commit
    let status_output = Command::new("git")
        .args([
            "-C",
            paths.config_dir.to_str().unwrap(),
            "status",
            "--porcelain",
        ])
        .output()?;

    if status_output.stdout.is_empty() {
        // No changes to commit
        return Ok(());
    }

    if !json {
        crate::ui::step("Committing", "changes to git");
    }

    // Stage all changes
    let add_result = Command::new("git")
        .args(["-C", paths.config_dir.to_str().unwrap(), "add", "."])
        .status()?;

    if !add_result.success() {
        anyhow::bail!("Failed to stage changes");
    }

    // Get hostname for the commit message
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());

    // Commit changes
    let commit_message = format!("Synced changes from {}", hostname);
    let commit_result = Command::new("git")
        .args([
            "-C",
            paths.config_dir.to_str().unwrap(),
            "commit",
            "-m",
            &commit_message,
        ])
        .status()?;

    if !commit_result.success() {
        anyhow::bail!("Failed to commit changes");
    }

    if !json {
        crate::ui::detail(&format!("committed as “{}”", commit_message));
    }

    Ok(())
}

/// Install packages one-at-a-time in strict order
fn install_module_packages_strict_order(
    paths: &ConfigPaths,
    packages: &[Package],
    flatpak_scope: &FlatpakScope,
    module_name: &str,
    module_num: usize,
    total_modules: usize,
) -> Result<InstallationSummary> {
    let mut summary = InstallationSummary::new();

    if packages.is_empty() {
        return Ok(summary);
    }

    // Separate native and flatpak packages (preserving order within each type)
    let (native_pkgs, flatpak_pkgs): (Vec<_>, Vec<_>) = packages
        .iter()
        .partition(|p| matches!(p.package_type, PackageType::Native));

    // Create backend once
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;

    let total_packages = packages.len() as u64;

    // Create detailed progress display
    use crate::progress::DetailedProgress;
    let mut progress =
        DetailedProgress::new(module_name, module_num, total_modules, total_packages);

    // Install native packages one at a time
    for pkg in native_pkgs.iter() {
        progress.set_current_package(&pkg.name);

        let start = std::time::Instant::now();
        let result = backend.install_packages_batch(&[&pkg.name]);

        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(true) => {
                progress.package_completed(&pkg.name, duration, true);
                summary.succeeded.push(pkg.name.clone());
            }
            Ok(false) => {
                progress.package_completed(&pkg.name, duration, false);
                let error_msg = "Installation failed".to_string();
                summary.failed.push((pkg.name.clone(), error_msg));
            }
            Err(e) => {
                progress.package_completed(&pkg.name, duration, false);
                let error_msg = format!("Failed to execute installer: {}", e);
                summary.failed.push((pkg.name.clone(), error_msg));
            }
        }
    }

    // Install flatpak packages one at a time
    let scope_flag = match flatpak_scope {
        FlatpakScope::User => "--user",
        FlatpakScope::System => "--system",
    };

    for pkg in flatpak_pkgs.iter() {
        progress.set_current_package(&pkg.name);

        let start = std::time::Instant::now();
        let result = std::process::Command::new("flatpak")
            .args(["install", "-y", scope_flag, "flathub", &pkg.name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(status) if status.success() => {
                progress.package_completed(&pkg.name, duration, true);
                summary.succeeded.push(pkg.name.clone());
            }
            Ok(status) => {
                progress.package_completed(&pkg.name, duration, false);
                let error_msg = format!(
                    "Installation failed (exit code: {})",
                    status.code().unwrap_or(-1)
                );
                summary.failed.push((pkg.name.clone(), error_msg));
            }
            Err(e) => {
                progress.package_completed(&pkg.name, duration, false);
                let error_msg = format!("Failed to execute flatpak: {}", e);
                summary.failed.push((pkg.name.clone(), error_msg));
            }
        }
    }

    progress.finish(module_name);
    Ok(summary)
}

/// Refresh package database after repo changes
fn refresh_package_database(paths: &ConfigPaths) -> Result<()> {
    crate::ui::step("Refreshing", "package database");

    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;
    backend.refresh_db()?;

    Ok(())
}

/// Process modules sequentially in order specified by enabled_modules
fn sync_modules_sequential(
    paths: &ConfigPaths,
    config: &Config,
    force_dotfiles: bool,
    json: bool,
) -> Result<()> {
    let state_file = paths.state_dir.join("modules-state.yaml");
    let mut modules_state = load_modules_state(&state_file)?;

    for (idx, module_name) in config.enabled_modules.iter().enumerate() {
        let module_num = idx + 1;
        let total_modules = config.enabled_modules.len();

        crate::ui::step(
            "Processing",
            &format!(
                "{} {}",
                module_name.cyan(),
                format!("({}/{})", module_num, total_modules).dimmed()
            ),
        );

        // Load module
        let module = match load_module_by_name(paths, module_name) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "{} Failed to load module '{}': {}",
                    "✗".red(),
                    module_name,
                    e
                );
                return Err(anyhow!("Module loading failed: {}", module_name));
            }
        };

        // Step 1: Run pre-install hook (with behavior checking)
        let mut ran_pre_hook = false;
        if let Some(hook_script) = module.pre_install_hook() {
            // Resolve hook path
            let hook_path = if std::path::Path::new(hook_script).is_absolute() {
                std::path::PathBuf::from(hook_script)
            } else if module.is_directory() {
                module.root_dir().join(hook_script)
            } else {
                paths.config_dir.join(hook_script)
            };

            if hook_path.exists() && !hook_path.is_dir() {
                let hook_behavior = module.pre_hook_behavior();

                // Check hook status
                let should_run =
                    match check_hook_status(paths, &format!("pre_{}", module_name), &hook_path)? {
                        HookStatus::Executed => {
                            // If behavior is "always", still run it
                            hook_behavior == "always"
                        }
                        HookStatus::Skipped => {
                            // If behavior is "always", override skip
                            hook_behavior == "always"
                        }
                        HookStatus::Modified | HookStatus::NotRun => {
                            // If behavior is "skip", don't run
                            hook_behavior != "skip"
                        }
                    };

                if should_run {
                    if !json {
                        crate::ui::detail("running pre-install hook");
                    }
                    execute_module_hook(
                        paths,
                        &module,
                        hook_script,
                        module_name,
                        true,
                        json,
                        module.run_hooks_as_user(),
                    )?;
                    mark_hook_executed(paths, &format!("pre_{}", module_name), &hook_path)?;
                    ran_pre_hook = true;
                }
            }
        }

        // Step 1.5: Refresh package database if hook ran (likely repo setup)
        if ran_pre_hook {
            refresh_package_database(paths)?;
        }

        // Step 2: Install module packages
        let module_packages: Vec<Package> = module.packages().iter().map(Package::from).collect();

        if !module_packages.is_empty() {
            // Check which packages need to be installed
            let total_packages = module_packages.len();
            let pkg_manager = PackageManager::new(paths.clone());
            let installed_pacman = pkg_manager.get_installed_native_packages(config)?;
            let installed_pacman_map: HashMap<String, String> =
                installed_pacman.into_iter().collect();

            let flatpak_scope_str = match config.flatpak_scope {
                FlatpakScope::User => "--user",
                FlatpakScope::System => "--system",
            };
            let installed_flatpaks = pkg_manager.get_installed_flatpaks(flatpak_scope_str)?;
            let installed_flatpak_set: HashSet<String> = installed_flatpaks.into_iter().collect();

            // Filter to only packages that need installation
            let packages_to_install: Vec<Package> = module_packages
                .into_iter()
                .filter(|pkg| match pkg.package_type {
                    PackageType::Native => !installed_pacman_map.contains_key(&pkg.name),
                    PackageType::Flatpak => !installed_flatpak_set.contains(&pkg.name),
                    PackageType::Nix => false,
                })
                .collect();

            if !packages_to_install.is_empty() {
                let summary = if config.strict_package_order {
                    // Don't print the old message - the progress display will handle it
                    install_module_packages_strict_order(
                        paths,
                        &packages_to_install,
                        &config.flatpak_scope,
                        module_name,
                        module_num,
                        total_modules,
                    )?
                } else {
                    println!(
                        "  {} Installing {} package{} (from {} total)...",
                        "→".blue(),
                        packages_to_install.len(),
                        if packages_to_install.len() == 1 {
                            ""
                        } else {
                            "s"
                        },
                        total_packages
                    );
                    install_module_packages(paths, &packages_to_install, &config.flatpak_scope)?
                };

                // Print summary
                if !summary.failed.is_empty() {
                    crate::ui::warn(
                        "Packages",
                        &format!(
                            "{} succeeded, {} failed",
                            summary.succeeded.len(),
                            summary.failed.len()
                        ),
                    );
                    for (pkg, err) in &summary.failed {
                        crate::ui::detail(&format!("{}: {}", pkg, err));
                    }
                } else if !summary.succeeded.is_empty() {
                    crate::ui::detail(&format!("{} package(s) installed", summary.succeeded.len()));
                }
            } else {
                crate::ui::detail(&format!("{} package(s) already installed", total_packages));
            }
        }

        // Step 3: Run post-install hook (with behavior checking)
        if let Some(hook_script) = module.post_install_hook() {
            // Resolve hook path
            let hook_path = if std::path::Path::new(hook_script).is_absolute() {
                std::path::PathBuf::from(hook_script)
            } else if module.is_directory() {
                module.root_dir().join(hook_script)
            } else {
                paths.config_dir.join(hook_script)
            };

            if hook_path.exists() && !hook_path.is_dir() {
                let hook_behavior = module.post_hook_behavior();

                // Check hook status
                let should_run = match check_hook_status(paths, module_name, &hook_path)? {
                    HookStatus::Executed => {
                        // If behavior is "always", still run it
                        hook_behavior == "always"
                    }
                    HookStatus::Skipped => {
                        // If behavior is "always", override skip
                        hook_behavior == "always"
                    }
                    HookStatus::Modified | HookStatus::NotRun => {
                        // If behavior is "skip", don't run
                        hook_behavior != "skip"
                    }
                };

                if should_run {
                    if !json {
                        crate::ui::detail("running post-install hook");
                    }
                    execute_module_hook(
                        paths,
                        &module,
                        hook_script,
                        module_name,
                        false,
                        json,
                        module.run_hooks_as_user(),
                    )?;
                    mark_hook_executed(paths, module_name, &hook_path)?;
                }
            }
        }

        // Update state: mark module as completed
        modules_state.completed_modules.insert(module_name.clone());
        modules_state.last_updated = Utc::now();
        save_modules_state(&state_file, &modules_state)?;
    }

    // Step 4: Sync ALL dotfiles after all modules complete
    crate::dotfiles::sync_dotfiles(paths, config, force_dotfiles, json)?;

    Ok(())
}

/// Print the computed package-change plan — JSON for `--json`, otherwise a
/// human-readable summary. In dry-run mode the human path also previews service
/// changes. Returns `true` when this was a dry run and the caller should stop
/// before mutating anything.
fn report_sync_plan(
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

/// Prune packages tracked in state but no longer declared (sequential path):
/// compute the candidates (protecting critical packages), then remove them —
/// native via the backend, flatpaks via `flatpak uninstall`.
fn prune_undeclared_packages(paths: &ConfigPaths, config: &Config) -> Result<()> {
    let pkg_manager = PackageManager::new(paths.clone());
    let declared = pkg_manager.get_declared_packages(config)?;

    crate::ui::step("Pruning", "undeclared packages");

    // Load state file to find packages from disabled modules
    if let Ok(state) = load_state_file(paths) {
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
    }

    Ok(())
}

/// Post-package steps of a sync: dotfile symlinks (and pruning), then system
/// services, default apps, desktop theming, source builds and Home Manager.
/// The `!dry_run` guards are retained for safety even though callers only reach
/// this on a real sync.
fn run_post_sync_steps(
    paths: &ConfigPaths,
    config: &Config,
    should_prune: bool,
    force_dotfiles: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    // Sync dotfiles (create symlinks)
    crate::dotfiles::sync_dotfiles(paths, config, force_dotfiles, json)?;

    // Prune old dotfiles if needed
    if should_prune {
        crate::dotfiles::prune_dotfiles(paths, config, json)?;
    }

    // Decrypt SOPS secrets into place. Per-secret failures are warned and
    // skipped inside sync_secrets, so this never aborts the sync.
    crate::secrets::sync_secrets(paths, config, dry_run, should_prune, json)?;

    // Sync system services
    if !dry_run {
        sync_services(paths, config, json)?;
    }

    // Sync default applications
    if !dry_run {
        sync_defaults(paths, config, json)?;
    }

    // Sync desktop theming
    if !dry_run {
        sync_theming(paths, config, json)?;
    }

    // Build source packages (only those not yet installed)
    if !dry_run {
        crate::commands::source::sync_sources(paths)?;
    }

    // Sync Home Manager (nix packages)
    if !dry_run && config.nix.home_manager_enabled {
        sync_home_manager(paths, config, json)?;
    }

    Ok(())
}

/// Create the pre-sync config and system backups. Best-effort: a backup
/// failure warns and the sync continues (it must not block applying config).
fn create_sync_backups(paths: &ConfigPaths, config: &Config) {
    // Config backup (independent of system backup)
    if config.config_backups.enabled {
        crate::ui::step("Backing up", "configuration");

        match crate::commands::config_backup::save_config(paths, "auto-sync", true) {
            Ok(_) => {}
            Err(e) => {
                crate::ui::warn(
                    "Backing up",
                    &format!("config backup failed: {} (continuing)", e),
                );
            }
        }
    }

    // System backup (independent of config backup)
    if config.system_backups.enabled && config.system_backups.backup_on_sync {
        match crate::commands::backup::create_backup_if_enabled(
            paths,
            "sync",
            "mdots sync autobackup",
        ) {
            Ok(true) => {}
            Ok(false) => {
                // Disabled in config, already checked but belt-and-suspenders
            }
            Err(e) => {
                crate::ui::warn(
                    "Backing up",
                    &format!("system backup failed: {} (continuing)", e),
                );
            }
        }
    }
}

// CLI flag set: this command exposes one parameter per sync flag.
#[allow(clippy::too_many_arguments)]
pub fn run(
    paths: &ConfigPaths,
    dry_run: bool,
    prune: bool,
    force: bool,
    no_backup: bool,
    no_hooks: bool,
    force_dotfiles: bool,
    json: bool,
    auto_commit: bool,
) -> Result<()> {
    let started = std::time::Instant::now();

    // If a previous sync left an in-progress marker, it did not finish — the
    // system may be partially applied. Warn before doing anything; re-running
    // sync (idempotent) reconciles it. Detect now, before we set our own marker.
    if sync_was_interrupted(&paths.state_dir) && !json {
        crate::ui::warn(
            "Warning",
            "previous sync did not finish — system may be partially applied",
        );
        crate::ui::detail("re-running sync now reconciles it to your configuration");
    }

    // Pre-flight validation check (similar to NixOS rebuild)
    if !json {
        crate::ui::step("Validating", "configuration");
    }

    let validation_errors = run_preflight_validation(paths, json)?;

    if validation_errors > 0 {
        if !json {
            crate::ui::error(
                "Validating",
                &format!("{} error(s) — refusing to sync", validation_errors),
            );
            crate::ui::detail("fix the errors above, or run `mdots validate` for details");
        }
        anyhow::bail!("Validation failed with {} error(s)", validation_errors);
    }

    // Load configuration
    let config = load_config(paths)?;

    // If sync_sudo is enabled, prompt for sudo password immediately
    // This caches the credentials so subsequent operations don't prompt
    if config.sync_sudo && !dry_run {
        if !json {
            crate::ui::step("Authorizing", "sudo access");
        }
        let status = std::process::Command::new("sudo")
            .arg("-v")
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to validate sudo credentials")?;
        if !status.success() {
            anyhow::bail!("Sudo authentication failed");
        }
    }

    // Use auto_prune from config if prune flag not explicitly set
    let should_prune = prune || config.auto_prune;

    // Branch based on module processing mode
    match config.module_processing {
        ModuleProcessing::Sequential => {
            // Sequential mode: process modules one by one in order
            if dry_run {
                if !json {
                    crate::ui::warn(
                        "Skipping",
                        "modules — dry-run is unsupported in sequential mode",
                    );

                    // Preview service changes
                    match preview_services(paths, &config) {
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
                                        crate::ui::note(
                                            "Services",
                                            &format!("{} {}", label, list.join(", ")),
                                        );
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

                    crate::ui::step(
                        "Finished",
                        &format!(
                            "dry run · no changes · {}",
                            crate::ui::format_elapsed(started.elapsed())
                        ),
                    );
                }
                return Ok(());
            } else {
                // About to mutate the system — mark the sync in progress so an
                // interrupted run is detectable next time.
                let _ = begin_sync_marker(&paths.state_dir);

                // Run sequential module processing
                sync_modules_sequential(paths, &config, force_dotfiles, json)?;

                // Prune old dotfiles if needed
                if should_prune {
                    if !json {
                        crate::ui::step("Pruning", "old dotfiles");
                    }
                    crate::dotfiles::prune_dotfiles(paths, &config, json)?;
                }
            }

            // Sync system services
            if !dry_run {
                sync_services(paths, &config, json)?;
            }

            // Sync default applications
            if !dry_run {
                sync_defaults(paths, &config, json)?;
            }

            // Sync Home Manager (nix packages)
            if !dry_run && config.nix.home_manager_enabled {
                sync_home_manager(paths, &config, json)?;
            }

            // Prune packages from disabled modules if auto_prune is enabled
            if should_prune {
                prune_undeclared_packages(paths, &config)?;
            }

            // Update state file with all packages (commit point — clears marker).
            let pkg_manager = PackageManager::new(paths.clone());
            let declared = pkg_manager.get_declared_packages(&config)?;
            commit_package_state(paths, &declared, json)?;

            if !json {
                crate::ui::step(
                    "Finished",
                    &format!("sync in {}", crate::ui::format_elapsed(started.elapsed())),
                );
            }

            return Ok(());
        }
        ModuleProcessing::Parallel => {
            // Parallel mode: existing behavior (all packages at once)
            // Continue with existing logic below
        }
    }

    // === PARALLEL MODE (existing behavior) ===
    let pkg_manager = PackageManager::new(paths.clone());

    if !json {
        crate::ui::step("Resolving", "packages");
    }

    // Get declared packages
    let declared = pkg_manager.get_declared_packages(&config)?;

    // Get installed packages
    let installed_pacman = pkg_manager.get_installed_native_packages(&config)?;
    let flatpak_scope = match config.flatpak_scope {
        FlatpakScope::User => "--user",
        FlatpakScope::System => "--system",
    };
    let installed_flatpaks = pkg_manager.get_installed_flatpaks(flatpak_scope)?;

    if !json {
        crate::ui::detail(&format!(
            "{} declared · {} installed",
            declared.len(),
            installed_pacman.len()
        ));
    }

    // Create maps for quick lookup
    let installed_pacman_map: HashMap<String, String> = installed_pacman.into_iter().collect();

    let installed_flatpak_set: HashSet<String> = installed_flatpaks.into_iter().collect();

    // Build sync plan: declared-but-not-installed packages.
    let (to_install, flatpak_to_install) =
        compute_installable(&declared, &installed_pacman_map, &installed_flatpak_set);

    // Find packages to remove (if pruning): tracked in state, no longer
    // declared, and still installed.
    let mut to_remove: Vec<String> = Vec::new();
    let mut flatpak_to_remove: Vec<String> = Vec::new();

    if should_prune {
        if let Ok(state) = load_state_file(paths) {
            let declared_names: HashSet<String> = declared.iter().map(|p| p.name.clone()).collect();
            let (native, flatpak) = compute_prunable(
                &state.packages,
                &declared_names,
                &installed_pacman_map,
                &installed_flatpak_set,
            );
            to_remove = native;
            flatpak_to_remove = flatpak;
        }
    }

    // Never auto-remove critical system packages, even when pruning.
    let (to_remove, protected_skipped) = partition_protected(to_remove);
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

    let plan = SyncPlan {
        to_install,
        to_remove,
        flatpak_to_install,
        flatpak_to_remove,
    };
    let no_package_changes = plan.is_empty();

    // Display the plan (and, in dry-run, preview services). Stop here if dry run.
    if report_sync_plan(paths, &config, &plan, dry_run, json)? {
        if !json {
            crate::ui::step(
                "Finished",
                &format!(
                    "dry run · no changes applied · {}",
                    crate::ui::format_elapsed(started.elapsed())
                ),
            );
        }
        return Ok(());
    }

    // From here on the system may be mutated (packages, then services and
    // dotfiles below) — mark the sync in progress so an interrupted run is
    // detectable next time. Cleared on every clean exit, including a cancel.
    let _ = begin_sync_marker(&paths.state_dir);

    // Only do package operations if there are changes
    if !no_package_changes {
        // Confirm unless --force or --json
        if !force && !json {
            println!();
            print!("Apply these changes? [y/N] ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if !input.trim().eq_ignore_ascii_case("y") {
                crate::ui::warn("Cancelled", "no changes applied");
                // Nothing was mutated — clear the marker before bailing.
                clear_sync_marker(&paths.state_dir);
                return Ok(());
            }
        }

        // Create backups unless --no-backup
        if !no_backup && !json {
            create_sync_backups(paths, &config);
        }

        // Run pre-install hooks before package installation
        if !no_hooks {
            run_pre_install_hooks(paths, &config, json)?;
        }

        // Execute sync operations
        execute_sync(paths, &plan, json)?;
    }

    // Apply dotfiles, services, defaults, theming, source builds and Home Manager.
    run_post_sync_steps(paths, &config, should_prune, force_dotfiles, dry_run, json)?;

    // Run post-install hooks. Failures here are reported but do NOT abort the
    // sync: the packages are already installed and must still be recorded in
    // state (otherwise the next sync sees an inconsistent system).
    if !no_hooks {
        if let Err(e) = run_post_install_hooks(paths, &config, json) {
            eprintln!("{} {}", "⚠".yellow(), e);
            eprintln!(
                "  Packages are installed and tracked; fix the hook(s) and re-run `mdots hooks run`."
            );
        }
    }

    // Update state file (commit point — clears the marker on success; a later
    // auto-commit failure does not leave the system half-applied).
    commit_package_state(paths, &declared, json)?;

    if !json {
        let installed = plan.to_install.len() + plan.flatpak_to_install.len();
        let removed = plan.to_remove.len() + plan.flatpak_to_remove.len();
        let mut parts = Vec::new();
        if installed > 0 {
            parts.push(format!("{} installed", installed));
        }
        if removed > 0 {
            parts.push(format!("{} removed", removed));
        }
        parts.push(crate::ui::format_elapsed(started.elapsed()));
        crate::ui::step("Finished", &format!("sync · {}", parts.join(" · ")));
    }

    // Auto-commit changes to git if enabled
    let should_auto_commit = auto_commit || config.auto_commit;
    if should_auto_commit && !dry_run {
        if let Err(e) = auto_commit_changes(paths, json) {
            if !json {
                eprintln!("{} Auto-commit failed: {}", "Warning:".yellow(), e);
            }
        }
    }

    Ok(())
}

/// Sync default applications based on configuration
fn sync_defaults(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
    // Skip if no default apps are configured
    let apps_map = config.default_apps.to_apps_map();
    if apps_map.is_empty() && config.default_apps.mime_types.is_empty() {
        return Ok(());
    }

    // Load previous defaults state
    let previous_state = load_defaults_state(&paths.defaults_state_file)
        .context("Failed to load previous defaults state")?;

    // Sync defaults
    let report = DefaultsManager::sync_defaults(
        &apps_map,
        &config.default_apps.mime_types,
        &config.default_apps.scope,
        &previous_state,
    )
    .context("Failed to sync default applications")?;

    // Update state file with new configuration
    let new_state = create_updated_defaults_state(
        &apps_map,
        &config.default_apps.mime_types,
        &config.default_apps.scope,
    );

    save_defaults_state(&paths.defaults_state_file, &new_state)
        .context("Failed to save defaults state")?;

    // Check for errors
    if report.has_errors() && !json {
        eprintln!();
        eprintln!(
            "{}: Some default app operations failed. Check the output above for details.",
            "Warning".yellow()
        );
    }

    Ok(())
}

/// Sync desktop theming based on configuration
fn sync_theming(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
    use crate::theming::has_theming_config;

    // Skip if no theming is configured
    if !has_theming_config(&config.theming) {
        return Ok(());
    }

    if !json {
        println!();
    }

    // Load previous theming state
    let previous_state = load_theming_state(&paths.theming_state_file)
        .context("Failed to load previous theming state")?;

    // Check if configuration has changed
    if !previous_state.has_changed(&config.theming) {
        if !json {
            crate::ui::step("Theming", "already in sync");
        }
        return Ok(());
    }

    // Sync theming
    let report = ThemingManager::apply_theming(&config.theming, false)
        .context("Failed to sync desktop theming")?;

    // Update state file with new configuration
    let new_state = create_theming_state(&config.theming);
    save_theming_state(&paths.theming_state_file, &new_state)
        .context("Failed to save theming state")?;

    // Check for errors
    if report.has_errors() && !json {
        eprintln!();
        eprintln!(
            "{}: Some theming operations failed. Check the output above for details.",
            "Warning".yellow()
        );
    }

    Ok(())
}

fn execute_sync(paths: &ConfigPaths, plan: &SyncPlan, json: bool) -> Result<()> {
    let to_install = &plan.to_install;
    let to_remove = &plan.to_remove;
    let flatpak_to_install = &plan.flatpak_to_install;
    let flatpak_to_remove = &plan.flatpak_to_remove;

    // Load config and create backend
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;

    // Install native packages
    if !to_install.is_empty() {
        let pkg_refs: Vec<&str> = to_install.iter().map(|s| s.as_str()).collect();
        if !json {
            crate::ui::step("Installing", &format!("{} package(s)", to_install.len()));
        }
        backend.install_packages_batch(&pkg_refs)?;
    }

    // Install flatpaks
    if !flatpak_to_install.is_empty() {
        if !json {
            crate::ui::step(
                "Installing",
                &format!("{} flatpak(s)", flatpak_to_install.len()),
            );
        }

        let scope_flag = match config.flatpak_scope {
            FlatpakScope::User => "--user",
            FlatpakScope::System => "--system",
        };

        for pkg in flatpak_to_install {
            execute_command("flatpak", &["install", "-y", scope_flag, pkg], json, false)?;
        }
    }

    // Remove native packages
    if !to_remove.is_empty() {
        if !json {
            crate::ui::step("Removing", &format!("{} package(s)", to_remove.len()));
        }

        let pkg_refs: Vec<&str> = to_remove.iter().map(|s| s.as_str()).collect();
        backend.remove_packages_batch(&pkg_refs)?;
    }

    // Remove flatpaks
    if !flatpak_to_remove.is_empty() {
        if !json {
            crate::ui::step(
                "Removing",
                &format!("{} flatpak(s)", flatpak_to_remove.len()),
            );
        }

        let scope_flag = match config.flatpak_scope {
            FlatpakScope::User => "--user",
            FlatpakScope::System => "--system",
        };

        for pkg in flatpak_to_remove {
            execute_command(
                "flatpak",
                &["uninstall", "-y", scope_flag, pkg],
                json,
                false,
            )?;
        }
    }

    Ok(())
}

fn run_pre_install_hooks(
    paths: &ConfigPaths,
    config: &crate::config::Config,
    json: bool,
) -> Result<()> {
    use crate::config::load_module;

    let mut hooks_to_run: Vec<(String, PathBuf, String)> = Vec::new(); // (module_name, hook_path, behavior)
    let mut skipped_count = 0;

    // First pass: check which hooks need to run
    for module_name in &config.enabled_modules {
        // Load the module to get hook info and resolve paths correctly
        let modules_dir = paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_lua = modules_dir.join(format!("{}.lua", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            module_dir
        } else if module_file.exists() {
            module_file
        } else if module_lua.exists() {
            module_lua
        } else {
            continue;
        };

        let module = match load_module(&module_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(hook_script) = module.pre_install_hook() {
            // Resolve hook path relative to module root (for directory modules)
            // or relative to config dir (for legacy modules)
            let hook_path = if std::path::Path::new(hook_script).is_absolute() {
                std::path::PathBuf::from(hook_script)
            } else if module.is_directory() {
                // For directory modules, resolve relative to module directory
                module.root_dir().join(hook_script)
            } else {
                // For legacy modules, resolve relative to config directory
                paths.config_dir.join(hook_script)
            };

            // Skip if path doesn't exist or is a directory
            if !hook_path.exists() || hook_path.is_dir() {
                continue;
            }

            let hook_behavior = module.pre_hook_behavior();

            // Check hook status
            match check_hook_status(paths, &format!("pre_{}", module_name), &hook_path)? {
                HookStatus::Executed => {
                    // If behavior is "always", still run it
                    if hook_behavior == "always" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
                HookStatus::Skipped => {
                    // If behavior is "always", override skip
                    if hook_behavior == "always" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
                HookStatus::Modified | HookStatus::NotRun => {
                    // If behavior is "skip", don't run
                    if hook_behavior != "skip" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
            }
        }
    }

    // Show summary of skipped hooks
    if !json && skipped_count > 0 {
        println!();
        println!(
            "{} {} pre-install hook{} already completed or skipped",
            "✓".green(),
            skipped_count,
            if skipped_count == 1 { "" } else { "s" }
        );
    }

    // Run hooks that need execution with interactive prompts
    for (module_name, hook_path, behavior) in hooks_to_run {
        // Handle behavior
        let should_run = if behavior == "always" {
            // Always run, no prompt
            if !json {
                println!();
                println!(
                    "{}",
                    format!("Pre-install hook for module '{}' (always run)", module_name)
                        .blue()
                        .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                crate::ui::detail("executing hook");
            }
            true
        } else if behavior == "once" {
            // Run once without asking
            if !json {
                println!();
                println!(
                    "{}",
                    format!("Pre-install hook for module '{}' (run once)", module_name)
                        .blue()
                        .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                crate::ui::detail("executing hook");
            }
            true
        } else if behavior == "ask" || behavior.is_empty() {
            // Ask user
            if json {
                // In JSON mode, skip asking and just run
                true
            } else {
                println!();
                println!(
                    "{}",
                    format!("Pre-install hook for module '{}'", module_name)
                        .blue()
                        .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                print!("Run this hook? [Y/n/s] (Y=yes, n=no this time, s=skip permanently): ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let choice = input.trim().to_lowercase();

                match choice.as_str() {
                    "n" => {
                        println!("{}", "Skipping this time".yellow());
                        false
                    }
                    "s" => {
                        println!("{}", "Marking as permanently skipped".yellow());
                        // Mark as skipped in state file
                        mark_hook_skipped(paths, &format!("pre_{}", module_name))?;
                        false
                    }
                    "" | "y" => {
                        println!();
                        crate::ui::detail("executing hook");
                        true
                    }
                    _ => {
                        println!("{}", "Invalid choice, skipping".yellow());
                        false
                    }
                }
            }
        } else {
            // Unknown behavior, default to true
            true
        };

        if !should_run {
            continue;
        }

        execute_command("sudo", &["bash", hook_path.to_str().unwrap()], json, false)?;

        mark_hook_executed(paths, &format!("pre_{}", module_name), &hook_path)?;

        if !json {
            println!("{}", "✓ Hook completed successfully".green());
        }
    }

    Ok(())
}

fn run_post_install_hooks(
    paths: &ConfigPaths,
    config: &crate::config::Config,
    json: bool,
) -> Result<()> {
    use crate::config::load_module;

    let mut hooks_to_run: Vec<(String, PathBuf, String)> = Vec::new(); // (module_name, hook_path, behavior)
    let mut skipped_count = 0;

    // First pass: check which hooks need to run
    for module_name in &config.enabled_modules {
        // Load the module to get hook info and resolve paths correctly
        let modules_dir = paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_lua = modules_dir.join(format!("{}.lua", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            module_dir
        } else if module_file.exists() {
            module_file
        } else if module_lua.exists() {
            module_lua
        } else {
            continue;
        };

        let module = match load_module(&module_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(hook_script) = module.post_install_hook() {
            // Resolve hook path relative to module root (for directory modules)
            // or relative to config dir (for legacy modules)
            let hook_path = if std::path::Path::new(hook_script).is_absolute() {
                std::path::PathBuf::from(hook_script)
            } else if module.is_directory() {
                // For directory modules, resolve relative to module directory
                module.root_dir().join(hook_script)
            } else {
                // For legacy modules, resolve relative to config directory
                paths.config_dir.join(hook_script)
            };

            // Skip if path doesn't exist or is a directory
            if !hook_path.exists() || hook_path.is_dir() {
                continue;
            }

            let hook_behavior = module.post_hook_behavior();

            // Check hook status
            match check_hook_status(paths, module_name, &hook_path)? {
                HookStatus::Executed => {
                    // If behavior is "always", still run it
                    if hook_behavior == "always" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
                HookStatus::Skipped => {
                    // If behavior is "always", override skip
                    if hook_behavior == "always" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
                HookStatus::Modified | HookStatus::NotRun => {
                    // If behavior is "skip", don't run
                    if hook_behavior != "skip" {
                        hooks_to_run.push((
                            module_name.clone(),
                            hook_path,
                            hook_behavior.to_string(),
                        ));
                    } else {
                        skipped_count += 1;
                    }
                }
            }
        }
    }

    // Show summary of skipped hooks
    if !json && skipped_count > 0 {
        println!();
        println!(
            "{} {} post-install hook{} already completed or skipped",
            "✓".green(),
            skipped_count,
            if skipped_count == 1 { "" } else { "s" }
        );
    }

    // Run hooks that need execution with interactive prompts
    let mut failed_hooks: Vec<String> = Vec::new();
    for (module_name, hook_path, behavior) in hooks_to_run {
        // Handle behavior
        let should_run = if behavior == "always" {
            // Always run, no prompt
            if !json {
                println!();
                println!(
                    "{}",
                    format!(
                        "Post-install hook for module '{}' (always run)",
                        module_name
                    )
                    .blue()
                    .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                crate::ui::detail("executing hook");
            }
            true
        } else if behavior == "once" {
            // Run once without asking
            if !json {
                println!();
                println!(
                    "{}",
                    format!("Post-install hook for module '{}' (run once)", module_name)
                        .blue()
                        .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                crate::ui::detail("executing hook");
            }
            true
        } else if behavior == "ask" || behavior.is_empty() {
            // Ask user
            if json {
                // In JSON mode, skip asking and just run
                true
            } else {
                println!();
                println!(
                    "{}",
                    format!("Post-install hook for module '{}'", module_name)
                        .blue()
                        .bold()
                );
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                print!("Run this hook? [Y/n/s] (Y=yes, n=no this time, s=skip permanently): ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let choice = input.trim().to_lowercase();

                match choice.as_str() {
                    "n" => {
                        println!("{}", "Skipping this time".yellow());
                        false
                    }
                    "s" => {
                        println!("{}", "Marking as permanently skipped".yellow());
                        // Mark as skipped in state file
                        mark_hook_skipped(paths, &module_name)?;
                        false
                    }
                    "" | "y" => {
                        println!();
                        crate::ui::detail("executing hook");
                        true
                    }
                    _ => {
                        println!("{}", "Invalid choice, skipping".yellow());
                        false
                    }
                }
            }
        } else {
            // Unknown behavior, default to true
            true
        };

        if !should_run {
            continue;
        }

        match execute_command("sudo", &["bash", hook_path.to_str().unwrap()], json, false) {
            Ok(()) => {
                mark_hook_executed(paths, &module_name, &hook_path)?;
                if !json {
                    crate::ui::detail("hook completed");
                }
            }
            Err(e) => {
                // One bad hook must not abort the rest of the hooks (or the sync):
                // the packages are already installed. Report it and move on; the
                // hook stays "not executed" so it is retried on the next sync.
                eprintln!(
                    "{} Post-install hook for module '{}' failed: {}",
                    "⚠".yellow(),
                    module_name,
                    e
                );
                failed_hooks.push(module_name.clone());
            }
        }
    }

    if !failed_hooks.is_empty() {
        anyhow::bail!(
            "{} post-install hook(s) failed: {}",
            failed_hooks.len(),
            failed_hooks.join(", ")
        );
    }

    Ok(())
}

// Helper functions

fn execute_command(program: &str, args: &[&str], json: bool, suppress_output: bool) -> Result<()> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);

    if json || suppress_output {
        // Suppress output in JSON mode or when showing progress bars
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
    } else {
        // In interactive mode, inherit stdin/stdout/stderr to allow user interaction
        cmd.stdin(std::process::Stdio::inherit());
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());
    }

    let status = cmd
        .status()
        .context(format!("Failed to execute {}", program))?;

    if !status.success() {
        anyhow::bail!("{} command failed", program);
    }

    Ok(())
}

#[derive(Debug)]
#[allow(dead_code)]
struct StatePackage {
    name: String,
    version: Option<String>,
    pkg_type: Option<String>,
}

#[derive(Debug)]
struct StateFile {
    packages: Vec<StatePackage>,
}

fn load_state_file(paths: &ConfigPaths) -> Result<StateFile> {
    use serde_yaml::Value;

    let content =
        std::fs::read_to_string(&paths.state_file).context("Failed to read state file")?;

    let yaml: Value = serde_yaml::from_str(&content).context("Failed to parse state file")?;

    let mut packages = Vec::new();

    if let Some(pkgs) = yaml.get("packages").and_then(|v| v.as_sequence()) {
        for pkg in pkgs {
            if let Some(pkg_map) = pkg.as_mapping() {
                let name = pkg_map
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let version = pkg_map
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let pkg_type = pkg_map
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if !name.is_empty() {
                    packages.push(StatePackage {
                        name,
                        version,
                        pkg_type,
                    });
                }
            }
        }
    }

    Ok(StateFile { packages })
}

fn update_state_file(paths: &ConfigPaths, declared: &[Package]) -> Result<()> {
    use chrono::Utc;
    use std::io::Write as _;

    // Ensure state directory exists
    std::fs::create_dir_all(&paths.state_dir).context(format!(
        "Failed to create state directory: {:?}",
        paths.state_dir
    ))?;

    let mut file = std::fs::File::create(&paths.state_file).context(format!(
        "Failed to create state file: {:?}",
        paths.state_file
    ))?;

    writeln!(
        file,
        "# Auto-generated state file - tracks mdots-managed packages"
    )?;
    writeln!(file, "# Generated: {}", Utc::now().to_rfc3339())?;
    writeln!(file)?;
    writeln!(file, "packages:")?;

    for pkg in declared {
        writeln!(file, "  - name: {}", pkg.name)?;

        // Get current version (don't fail if we can't get it)
        let version = if pkg.package_type == PackageType::Flatpak {
            get_flatpak_version(&pkg.name).unwrap_or(None)
        } else {
            get_package_version(&pkg.name, paths).unwrap_or(None)
        };

        if let Some(ver) = version {
            writeln!(file, "    version: \"{}\"", ver)?;
        }

        let type_str = match pkg.package_type {
            PackageType::Flatpak => "flatpak",
            PackageType::Native => "native",
            PackageType::Nix => "nix",
        };
        writeln!(file, "    type: {}", type_str)?;
    }

    Ok(())
}

/// Path of the "a sync is mid-flight" marker inside the state directory.
fn sync_marker_path(state_dir: &Path) -> PathBuf {
    state_dir.join("sync-in-progress")
}

/// Drop a marker recording that a system-mutating sync has begun. It is removed
/// only on a clean finish ([`clear_sync_marker`]); if the process dies or errors
/// out mid-sync the marker survives, so the next run can warn the user that the
/// system may be partially applied. Best-effort but creates the state dir first.
fn begin_sync_marker(state_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    std::fs::write(
        sync_marker_path(state_dir),
        format!(
            "# mdots sync in progress (pid {})\n# If this file lingers, a previous \
             sync did not finish. Re-run `mdots sync` to reconcile.\n",
            std::process::id()
        ),
    )
}

/// Remove the in-progress marker after a sync finishes cleanly. Idempotent: a
/// missing marker is not an error.
fn clear_sync_marker(state_dir: &Path) {
    let _ = std::fs::remove_file(sync_marker_path(state_dir));
}

/// True if a previous sync left an in-progress marker behind (interrupted run).
/// Surfaced both at the start of `sync` and by `mdots status`.
pub(crate) fn sync_was_interrupted(state_dir: &Path) -> bool {
    sync_marker_path(state_dir).exists()
}

/// Commit point of a sync: write the final package state, then clear the
/// in-progress marker. On a state-write failure the marker is intentionally
/// left in place so the next sync warns that the system may be half-applied.
/// Shared by the sequential and parallel sync paths.
fn commit_package_state(paths: &ConfigPaths, declared: &[Package], json: bool) -> Result<()> {
    let state_spinner = if !json {
        let spinner = crate::progress::create_spinner("Updating package state...");
        spinner.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(spinner)
    } else {
        None
    };

    if let Err(e) = update_state_file(paths, declared) {
        if let Some(spinner) = state_spinner {
            spinner.finish_and_clear();
        }
        eprintln!("Error updating state file: {}", e);
        return Err(e);
    }

    if let Some(spinner) = state_spinner {
        spinner.finish_and_clear();
    }

    // State committed cleanly — the system matches config; clear the marker.
    clear_sync_marker(&paths.state_dir);
    Ok(())
}

fn get_package_version(package: &str, paths: &ConfigPaths) -> Result<Option<String>> {
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;
    backend.get_package_version(package)
}

fn get_flatpak_version(package: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("flatpak")
        .args(["info", package])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let result = String::from_utf8_lossy(&out.stdout);
            for line in result.lines() {
                if line.starts_with("Version:") {
                    if let Some(ver) = line.split(':').nth(1) {
                        return Ok(Some(ver.trim().to_string()));
                    }
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

pub enum HookStatus {
    NotRun,
    Executed,
    Skipped,
    Modified, // Hook exists but script has been modified
}

pub fn check_hook_status(
    paths: &ConfigPaths,
    module: &str,
    hook_path: &std::path::PathBuf,
) -> Result<HookStatus> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    if !paths.hooks_state_file.exists() {
        return Ok(HookStatus::NotRun);
    }

    let content = std::fs::read_to_string(&paths.hooks_state_file)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;

    // Get current hash of script
    let script_content = std::fs::read_to_string(hook_path)?;
    let mut hasher = DefaultHasher::new();
    script_content.hash(&mut hasher);
    let current_hash = hasher.finish().to_string();

    // Check stored hash and status
    if let Some(hooks) = yaml.get("hooks").and_then(|v| v.as_sequence()) {
        for hook in hooks {
            if let Some(hook_map) = hook.as_mapping() {
                let stored_module = hook_map
                    .get("module")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if stored_module == module {
                    let status = hook_map
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("executed");

                    // If marked as skipped, always return skipped
                    if status == "skipped" {
                        return Ok(HookStatus::Skipped);
                    }

                    let stored_hash = hook_map
                        .get("script_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    // If executed, check if script has been modified
                    if stored_hash == current_hash {
                        return Ok(HookStatus::Executed);
                    } else {
                        return Ok(HookStatus::Modified);
                    }
                }
            }
        }
    }

    Ok(HookStatus::NotRun)
}

pub fn mark_hook_skipped(paths: &ConfigPaths, module: &str) -> Result<()> {
    // Initialize or load hooks state file
    let mut yaml: serde_yaml::Value = if paths.hooks_state_file.exists() {
        let content = std::fs::read_to_string(&paths.hooks_state_file)?;
        serde_yaml::from_str(&content)?
    } else {
        serde_yaml::from_str("hooks: []")?
    };

    // Remove existing entry for this module
    if let Some(hooks) = yaml.get_mut("hooks").and_then(|v| v.as_sequence_mut()) {
        hooks.retain(|hook| {
            hook.get("module")
                .and_then(|v| v.as_str())
                .map(|m| m != module)
                .unwrap_or(true)
        });

        // Add new entry with skipped status
        let new_hook = serde_yaml::to_value(serde_json::json!({
            "module": module,
            "status": "skipped",
        }))?;

        hooks.push(new_hook);
    }

    // Write back to file
    std::fs::create_dir_all(paths.state_dir.as_path())?;
    let content = serde_yaml::to_string(&yaml)?;
    std::fs::write(&paths.hooks_state_file, content)?;

    Ok(())
}

pub fn mark_hook_executed(
    paths: &ConfigPaths,
    module: &str,
    hook_path: &std::path::PathBuf,
) -> Result<()> {
    use chrono::Utc;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Get hash of script
    let script_content = std::fs::read_to_string(hook_path)?;
    let mut hasher = DefaultHasher::new();
    script_content.hash(&mut hasher);
    let script_hash = hasher.finish().to_string();

    let timestamp = Utc::now().to_rfc3339();

    // Initialize or load hooks state file
    let mut yaml: serde_yaml::Value = if paths.hooks_state_file.exists() {
        let content = std::fs::read_to_string(&paths.hooks_state_file)?;
        serde_yaml::from_str(&content)?
    } else {
        serde_yaml::from_str("hooks: []")?
    };

    // Remove existing entry for this module
    if let Some(hooks) = yaml.get_mut("hooks").and_then(|v| v.as_sequence_mut()) {
        hooks.retain(|hook| {
            hook.get("module")
                .and_then(|v| v.as_str())
                .map(|m| m != module)
                .unwrap_or(true)
        });

        // Add new entry
        let new_hook = serde_yaml::to_value(serde_json::json!({
            "module": module,
            "script": hook_path.to_string_lossy().to_string(),
            "script_hash": script_hash,
            "executed_at": timestamp,
            "status": "executed",
        }))?;

        hooks.push(new_hook);
    }

    // Write back to file
    std::fs::create_dir_all(paths.state_dir.as_path())?;
    let content = serde_yaml::to_string(&yaml)?;
    std::fs::write(&paths.hooks_state_file, content)?;

    Ok(())
}

/// Run pre-flight validation before sync (similar to NixOS rebuild)
fn run_preflight_validation(paths: &ConfigPaths, json: bool) -> Result<usize> {
    use crate::config::{load_module, validate_module};
    use walkdir::WalkDir;

    let mut total_errors = 0;
    let mut total_warnings = 0;

    // Load config to get enabled modules
    let config = match load_config(paths) {
        Ok(c) => c,
        Err(e) => {
            if !json {
                println!("  {} Failed to load config: {}", "✗".red(), e);
            }
            return Ok(1); // Return 1 error
        }
    };

    // Validate each enabled module
    for module_name in &config.enabled_modules {
        let modules_dir = paths.modules_dir();

        // Try to find the module (file or directory)
        let module_path = if module_name.contains('/') {
            // Full path like "window-managers/hyprland"
            let yaml_path = modules_dir.join(format!("{}.yaml", module_name));
            let lua_path = modules_dir.join(format!("{}.lua", module_name));
            let nix_path = modules_dir.join(format!("{}.nix", module_name));
            let dir_path = modules_dir.join(module_name);

            if yaml_path.exists() {
                yaml_path
            } else if lua_path.exists() {
                lua_path
            } else if nix_path.exists() {
                nix_path
            } else if dir_path.exists() {
                dir_path
            } else {
                if !json {
                    println!("  {} Module not found: {}", "✗".red(), module_name);
                }
                total_errors += 1;
                continue;
            }
        } else {
            // Short name - search for it
            let mut found = None;

            // First try direct yaml/lua/nix file
            let direct_yaml = modules_dir.join(format!("{}.yaml", module_name));
            let direct_lua = modules_dir.join(format!("{}.lua", module_name));
            let direct_nix = modules_dir.join(format!("{}.nix", module_name));
            if direct_yaml.exists() {
                found = Some(direct_yaml);
            } else if direct_lua.exists() {
                found = Some(direct_lua);
            } else if direct_nix.exists() {
                found = Some(direct_nix);
            } else {
                // Try direct directory
                let direct_dir = modules_dir.join(module_name);
                if direct_dir.exists() && direct_dir.is_dir() {
                    found = Some(direct_dir);
                } else {
                    // Search subdirectories
                    for entry in WalkDir::new(&modules_dir)
                        .max_depth(3)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        let path = entry.path();
                        let ext = path.extension().and_then(|s| s.to_str());
                        let is_module_file =
                            ext == Some("yaml") || ext == Some("lua") || ext == Some("nix");

                        if (path.is_file()
                            && path.file_stem().and_then(|s| s.to_str()) == Some(module_name)
                            && is_module_file)
                            || (path.is_dir()
                                && path.file_name().and_then(|s| s.to_str()) == Some(module_name))
                        {
                            found = Some(path.to_path_buf());
                            break;
                        }
                    }
                }
            }

            if let Some(p) = found {
                p
            } else {
                if !json {
                    println!("  {} Module not found: {}", "✗".red(), module_name);
                }
                total_errors += 1;
                continue;
            }
        };

        // Load and validate the module
        match load_module(&module_path) {
            Ok(module) => {
                let validation = validate_module(&module, module_name);

                if !validation.is_clean() {
                    if !json {
                        // Print module name
                        let module_type = if module.is_directory() {
                            "directory"
                        } else if module.is_lua() {
                            "lua"
                        } else if module.is_nix() {
                            "nix"
                        } else {
                            "yaml"
                        };
                        println!(
                            "  {} Module '{}' ({}):",
                            "→".blue(),
                            module_name,
                            module_type
                        );

                        // Print errors
                        for error in &validation.errors {
                            println!("    {} {}", "✗".red(), error);
                        }

                        // Print warnings
                        for warning in &validation.warnings {
                            println!("    {} {}", "⚠".yellow(), warning);
                        }
                    }

                    total_errors += validation.errors.len();
                    total_warnings += validation.warnings.len();
                }
            }
            Err(e) => {
                if !json {
                    println!(
                        "  {} Failed to load module '{}': {}",
                        "✗".red(),
                        module_name,
                        e
                    );
                }
                total_errors += 1;
            }
        }
    }

    if !json && total_warnings > 0 && total_errors == 0 {
        println!();
        println!("  {} {} warning(s) found", "⚠".yellow(), total_warnings);
    }

    Ok(total_errors)
}

/// Collect all services from config and enabled service profiles
fn collect_all_services(paths: &ConfigPaths, config: &Config) -> Result<ServicesConfig> {
    let mut all_services = config.services.clone();

    // Load enabled service profiles and merge their services
    if !config.enabled_service_profiles.is_empty() {
        let profile_manager = ServiceProfileManager::new(paths.clone());

        for profile_name in &config.enabled_service_profiles {
            match profile_manager.load_profile(profile_name) {
                Ok(profile) => {
                    // Merge enabled services (deduplicate)
                    for svc in profile.services.enabled {
                        if !all_services.enabled.contains(&svc) {
                            all_services.enabled.push(svc);
                        }
                    }
                    // Merge disabled services (deduplicate)
                    for svc in profile.services.disabled {
                        if !all_services.disabled.contains(&svc) {
                            all_services.disabled.push(svc);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load service profile '{}': {}", profile_name, e);
                }
            }
        }
    }

    Ok(all_services)
}

/// Preview service changes (for dry-run mode)
fn preview_services(paths: &ConfigPaths, config: &Config) -> Result<ServicesPreview> {
    // Collect services from config and all enabled service profiles
    let all_services = collect_all_services(paths, config)?;

    // Skip if no services are configured
    if all_services.enabled.is_empty() && all_services.disabled.is_empty() {
        return Ok(ServicesPreview::default());
    }

    // Load previous service state
    let previous_state = load_services_state(&paths.services_state_file)
        .context("Failed to load previous services state")?;

    // Get preview of changes
    ServiceManager::preview_services(
        &all_services.enabled,
        &all_services.disabled,
        &previous_state,
        all_services.scope,
    )
    .context("Failed to preview services")
}

/// Sync system services based on configuration
fn sync_services(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
    // Collect services from config and all enabled service profiles
    let all_services = collect_all_services(paths, config)?;

    // Skip if no services are configured
    if all_services.enabled.is_empty() && all_services.disabled.is_empty() {
        return Ok(());
    }

    // Load previous service state
    let previous_state = load_services_state(&paths.services_state_file)
        .context("Failed to load previous services state")?;

    // Sync services
    let report = ServiceManager::sync_services(
        &all_services.enabled,
        &all_services.disabled,
        &previous_state,
        all_services.scope,
    )
    .context("Failed to sync services")?;

    // Update state file with new service configuration
    let new_state = create_updated_state(&all_services.enabled, &all_services.disabled);

    save_services_state(&paths.services_state_file, &new_state)
        .context("Failed to save services state")?;

    // Check for errors
    if report.has_errors() && !json {
        eprintln!();
        eprintln!(
            "{}: Some service operations failed. Check the output above for details.",
            "Warning".yellow()
        );
    }

    Ok(())
}

/// Sync Home Manager configuration (nix packages)
fn sync_home_manager(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
    if !crate::nix::is_home_manager_installed() {
        if !json {
            println!();
            println!(
                "{}",
                "⚠ Home Manager is not installed. Skipping nix package sync.".yellow()
            );
            println!("  Run {} to set it up.", "mdots init --nix-init".cyan());
        }
        return Ok(());
    }

    // Collect nix packages from all modules
    let nix_packages = crate::nix::collect_nix_packages(config, paths)?;

    if !json {
        if nix_packages.is_empty() {
            crate::ui::step("Home Mgr", "no nix packages");
        } else {
            crate::ui::step(
                "Home Mgr",
                &format!("{} nix package(s)", nix_packages.len()),
            );
            println!(
                "  {} {} nix package(s) to manage:",
                "→".blue(),
                nix_packages.len()
            );
            for pkg in &nix_packages {
                println!("    {}", pkg);
            }
        }
    }

    // Determine where to write mdots-packages.nix
    let mdots_packages_path = if crate::nix::use_per_host_structure(paths) {
        // Per-host structure: hosts/{hostname}/mdots-packages.nix
        crate::nix::home_manager_host_dir(paths, &config.host).join("mdots-packages.nix")
    } else {
        // Flat structure (backward compat): root mdots-packages.nix
        paths.home_manager_dir().join("mdots-packages.nix")
    };

    if let Some(parent) = mdots_packages_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create home-manager directory")?;
    }

    crate::nix::generate_mdots_packages_nix(&nix_packages, &mdots_packages_path)?;

    if !json {
        println!(
            "  {} Generated {}",
            "✓".green(),
            mdots_packages_path.display()
        );
    }

    // Run home-manager switch
    crate::nix::home_manager_switch(paths, config)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_protected_separates_critical_packages() {
        let (removable, protected) = partition_protected(vec![
            "firefox".to_string(),
            "linux".to_string(),
            "steam".to_string(),
            "glibc".to_string(),
            "pacman".to_string(),
        ]);
        // Ordinary packages stay removable, in order.
        assert_eq!(removable, vec!["firefox".to_string(), "steam".to_string()]);
        // Critical system packages are pulled out and never removed.
        let mut p = protected;
        p.sort();
        assert_eq!(
            p,
            vec![
                "glibc".to_string(),
                "linux".to_string(),
                "pacman".to_string()
            ]
        );
    }

    #[test]
    fn test_sync_marker_roundtrip_signals_interruption() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");

        // A fresh checkout has never been interrupted.
        assert!(!sync_was_interrupted(&state_dir));

        // Beginning a sync drops a marker (creating the state dir if needed).
        begin_sync_marker(&state_dir).unwrap();
        assert!(
            sync_was_interrupted(&state_dir),
            "an in-progress sync must be detectable"
        );

        // A clean completion clears it, so the next run does not warn.
        clear_sync_marker(&state_dir);
        assert!(
            !sync_was_interrupted(&state_dir),
            "a completed sync must clear the marker"
        );
    }

    #[test]
    fn test_clear_sync_marker_is_idempotent_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        // Clearing a marker that was never written must be a harmless no-op.
        clear_sync_marker(&state_dir);
        assert!(!sync_was_interrupted(&state_dir));
    }

    fn sp(name: &str, ty: Option<&str>) -> StatePackage {
        StatePackage {
            name: name.to_string(),
            version: None,
            pkg_type: ty.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_compute_prunable_selects_undeclared_but_installed() {
        let state = vec![
            sp("vim", None),                  // declared → keep
            sp("htop", Some("native")),       // undeclared + installed → prune
            sp("gone", None),                 // undeclared + NOT installed → skip
            sp("org.x.App", Some("flatpak")), // undeclared flatpak + installed → prune
        ];
        let declared: HashSet<String> = ["vim".to_string()].into_iter().collect();
        let installed_native: HashMap<String, String> = [("htop".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        let installed_flatpak: HashSet<String> = ["org.x.App".to_string()].into_iter().collect();

        let (native, flatpak) =
            compute_prunable(&state, &declared, &installed_native, &installed_flatpak);
        assert_eq!(native, vec!["htop".to_string()]);
        assert_eq!(flatpak, vec!["org.x.App".to_string()]);
    }

    #[test]
    fn test_compute_prunable_treats_unknown_type_as_native() {
        let state = vec![sp("mystery", Some("weird"))];
        let installed_native: HashMap<String, String> = [("mystery".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        let (native, flatpak) =
            compute_prunable(&state, &HashSet::new(), &installed_native, &HashSet::new());
        assert_eq!(native, vec!["mystery".to_string()]);
        assert!(flatpak.is_empty());
    }

    #[test]
    fn test_prune_set_filtered_excludes_protected_and_keeps_declared() {
        // Mixed state: a declared pkg (keep), a normal undeclared+installed pkg
        // (prune), a PROTECTED undeclared+installed pkg (must be filtered out),
        // and an undeclared+installed flatpak (prune).
        let state = vec![
            sp("vim", Some("native")),        // declared → keep
            sp("htop", Some("native")),       // undeclared + installed → prune
            sp("systemd", Some("native")),    // protected → never removed
            sp("org.x.App", Some("flatpak")), // undeclared flatpak + installed → prune
        ];
        let declared: HashSet<String> = ["vim".to_string()].into_iter().collect();
        let installed_native: HashMap<String, String> = [
            ("htop".to_string(), "1".to_string()),
            ("systemd".to_string(), "255".to_string()),
        ]
        .into_iter()
        .collect();
        let installed_flatpak: HashSet<String> = ["org.x.App".to_string()].into_iter().collect();

        let (native, flatpak) =
            prune_set_filtered(&state, &declared, &installed_native, &installed_flatpak);
        // "systemd" is protected and must not appear; "htop" must.
        assert_eq!(native, vec!["htop".to_string()]);
        assert!(
            !native.contains(&"systemd".to_string()),
            "protected packages must never be in the prune set"
        );
        assert_eq!(flatpak, vec!["org.x.App".to_string()]);
    }

    #[test]
    fn test_compute_prunable_keeps_declared_packages() {
        let state = vec![sp("keep", None)];
        let declared: HashSet<String> = ["keep".to_string()].into_iter().collect();
        let installed_native: HashMap<String, String> = [("keep".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        let (native, flatpak) =
            compute_prunable(&state, &declared, &installed_native, &HashSet::new());
        assert!(native.is_empty(), "declared package must never be pruned");
        assert!(flatpak.is_empty());
    }

    fn pkg(name: &str, ty: PackageType) -> Package {
        Package {
            name: name.to_string(),
            package_type: ty,
        }
    }

    #[test]
    fn test_compute_installable_selects_only_missing_packages() {
        let declared = vec![
            pkg("vim", PackageType::Native),      // installed → skip
            pkg("htop", PackageType::Native),     // missing → install
            pkg("org.x.A", PackageType::Flatpak), // installed → skip
            pkg("org.y.B", PackageType::Flatpak), // missing → install
        ];
        let installed_native: HashMap<String, String> =
            [("vim".to_string(), "1".to_string())].into_iter().collect();
        let installed_flatpak: HashSet<String> = ["org.x.A".to_string()].into_iter().collect();

        let (native, flatpak) =
            compute_installable(&declared, &installed_native, &installed_flatpak);
        assert_eq!(native, vec!["htop".to_string()]);
        assert_eq!(flatpak, vec!["org.y.B".to_string()]);
    }

    #[test]
    fn test_compute_installable_ignores_nix_packages() {
        // Nix packages are applied via home-manager, not this diff.
        let declared = vec![pkg("hello", PackageType::Nix)];
        let (native, flatpak) = compute_installable(&declared, &HashMap::new(), &HashSet::new());
        assert!(native.is_empty());
        assert!(flatpak.is_empty());
    }
}
