use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use crate::config::{ConfigPaths, FlatpakScope, ModuleProcessing, PackageType, load_config};
use crate::package::PackageManager;

mod execute;
mod hooks;
mod plan;
mod prune;
mod state;
#[cfg(test)]
mod tests;

use execute::*;
use hooks::*;
use plan::*;
use prune::*;
use state::*;

pub use hooks::{HookStatus, check_hook_status, mark_hook_executed, mark_hook_skipped};
pub(crate) use plan::{compute_installable, compute_prune_preview};
pub(crate) use state::sync_was_interrupted;

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

/// CLI flag set passed to [`run`]; one field per sync flag.
pub struct SyncOptions {
    pub dry_run: bool,
    pub prune: bool,
    pub force: bool,
    pub no_backup: bool,
    pub no_hooks: bool,
    pub force_dotfiles: bool,
    pub json: bool,
    pub auto_commit: bool,
}

pub fn run(paths: &ConfigPaths, opts: SyncOptions) -> Result<()> {
    let SyncOptions {
        dry_run,
        prune,
        force,
        no_backup,
        no_hooks,
        force_dotfiles,
        json,
        auto_commit,
    } = opts;
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
        let state = load_prune_state(paths)?;
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

    let flatpak_remotes = declared
        .iter()
        .filter(|p| matches!(p.package_type, PackageType::Flatpak))
        .filter_map(|p| Some((p.name.clone(), p.remote.clone()?)))
        .collect();
    let plan = SyncPlan {
        to_install,
        to_remove,
        flatpak_to_install,
        flatpak_to_remove,
        flatpak_remotes,
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
    if !no_hooks && let Err(e) = run_post_install_hooks(paths, &config, json) {
        eprintln!("{} {}", "⚠".yellow(), e);
        eprintln!(
            "  Packages are installed and tracked; fix the hook(s) and re-run `mdots hooks run`."
        );
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
    if should_auto_commit
        && !dry_run
        && let Err(e) = auto_commit_changes(paths, json)
        && !json
    {
        eprintln!("{} Auto-commit failed: {}", "Warning:".yellow(), e);
    }

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
