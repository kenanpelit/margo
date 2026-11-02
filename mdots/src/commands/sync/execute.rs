//! Applying a sync: installing packages (batched or module by module) and the
//! post-package steps (dotfiles, services, defaults, theming, Home Manager).

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use colored::*;
use std::collections::{HashMap, HashSet};

use crate::config::{
    Config, ConfigPaths, FlatpakScope, PackageType, ServicesConfig, load_config,
    resolve_flatpak_remote,
};
use crate::defaults::{
    DefaultsManager, create_updated_state as create_updated_defaults_state, load_defaults_state,
    save_defaults_state,
};
use crate::package::{Package, PackageManager};
use crate::service_profile::ServiceProfileManager;
use crate::services::{
    ServiceManager, ServicesPreview, create_updated_state, load_services_state, save_services_state,
};
use crate::theming::ThemingManager;
use crate::theming::state::{create_theming_state, load_theming_state, save_theming_state};

use super::*;

/// Summary of package installation results
#[derive(Debug, Clone)]
pub(super) struct InstallationSummary {
    pub(super) succeeded: Vec<String>,
    pub(super) failed: Vec<(String, String)>, // (package_name, error_message)
}

impl InstallationSummary {
    pub(super) fn new() -> Self {
        Self {
            succeeded: Vec::new(),
            failed: Vec::new(),
        }
    }
}

/// Load a module by name, searching in modules directory
pub(super) fn load_module_by_name(
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

/// Install packages from a single module
pub(super) fn install_module_packages(
    paths: &ConfigPaths,
    packages: &[Package],
    flatpak_scope: &FlatpakScope,
    host_flatpak_remote: Option<&str>,
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
                .args([
                    "install",
                    "-y",
                    scope_flag,
                    resolve_flatpak_remote(pkg.remote.as_deref(), host_flatpak_remote),
                    &pkg.name,
                ])
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
pub(super) fn auto_commit_changes(paths: &ConfigPaths, json: bool) -> anyhow::Result<()> {
    use std::process::Command;

    // Check if the config directory is a git repository
    let git_dir = paths.config_dir.join(".git");
    if !git_dir.exists() {
        anyhow::bail!("Not a git repository");
    }

    let config_dir = paths
        .config_dir
        .to_str()
        .context("config dir path is not valid UTF-8")?;

    // Check if there are any changes to commit
    let status_output = Command::new("git")
        .args(["-C", config_dir, "status", "--porcelain"])
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
        .args(["-C", config_dir, "add", "."])
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
        .args(["-C", config_dir, "commit", "-m", &commit_message])
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
pub(super) fn install_module_packages_strict_order(
    paths: &ConfigPaths,
    packages: &[Package],
    flatpak_scope: &FlatpakScope,
    host_flatpak_remote: Option<&str>,
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
            .args([
                "install",
                "-y",
                scope_flag,
                resolve_flatpak_remote(pkg.remote.as_deref(), host_flatpak_remote),
                &pkg.name,
            ])
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
pub(super) fn refresh_package_database(paths: &ConfigPaths) -> Result<()> {
    crate::ui::step("Refreshing", "package database");

    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;
    backend.refresh_db()?;

    Ok(())
}

/// Process modules sequentially in order specified by enabled_modules
pub(super) fn sync_modules_sequential(
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
                        config.flatpak_remote.as_deref(),
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
                    install_module_packages(
                        paths,
                        &packages_to_install,
                        &config.flatpak_scope,
                        config.flatpak_remote.as_deref(),
                    )?
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

/// Post-package steps of a sync: dotfile symlinks (and pruning), then system
/// services, default apps, desktop theming, source builds and Home Manager.
/// The `!dry_run` guards are retained for safety even though callers only reach
/// this on a real sync.
pub(super) fn run_post_sync_steps(
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
pub(super) fn create_sync_backups(paths: &ConfigPaths, config: &Config) {
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

/// Sync default applications based on configuration
pub(super) fn sync_defaults(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
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
pub(super) fn sync_theming(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
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

pub(super) fn execute_sync(paths: &ConfigPaths, plan: &SyncPlan, json: bool) -> Result<()> {
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
            let remote = resolve_flatpak_remote(
                plan.flatpak_remotes.get(pkg).map(String::as_str),
                config.flatpak_remote.as_deref(),
            );
            execute_command(
                "flatpak",
                &["install", "-y", scope_flag, remote, pkg],
                json,
                false,
            )?;
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

pub(super) fn execute_command(
    program: &str,
    args: &[&str],
    json: bool,
    suppress_output: bool,
) -> Result<()> {
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

/// Collect all services from config and enabled service profiles
pub(super) fn collect_all_services(paths: &ConfigPaths, config: &Config) -> Result<ServicesConfig> {
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
pub(super) fn preview_services(paths: &ConfigPaths, config: &Config) -> Result<ServicesPreview> {
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
pub(super) fn sync_services(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
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
pub(super) fn sync_home_manager(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
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
