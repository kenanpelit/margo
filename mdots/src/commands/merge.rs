use anyhow::{Context, Result};
use colored::*;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

use crate::config::{load_config, Config, ConfigPaths, RunHooksAsUser};
use crate::defaults::DefaultsManager;
use crate::package::PackageManager;
use crate::services::ServiceManager;

/// Create a backup path with timestamp
fn create_backup_path(original: &Path) -> std::path::PathBuf {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let parent = original.parent().unwrap_or(Path::new("."));
    let name = original.file_name().unwrap().to_str().unwrap();
    parent.join(format!("{}.backup.{}", name, timestamp))
}

/// Insert services into the services.enabled table in Lua content
fn insert_services_into_lua(
    content: &str,
    services: &[String],
    user_scope: bool,
) -> Result<String> {
    // Find the services.enabled table pattern
    // Look for: services = { enabled = { ... },
    let pattern = regex::Regex::new(r"(services\s*=\s*\{[^}]*enabled\s*=\s*\{)([^}]*)").unwrap();

    if let Some(captures) = pattern.captures(content) {
        let existing_entries = captures.get(2).unwrap().as_str();

        // Build new entries string
        let mut new_entries = String::new();
        for svc in services {
            if !existing_entries.contains(&format!("\"{}\"", svc)) {
                new_entries.push_str(&format!("\"{}\", ", svc));
            }
        }

        // Replace in content
        let result = pattern.replace(content, |caps: &regex::Captures| {
            format!(
                "{}{}{}",
                caps.get(1).unwrap().as_str(),
                new_entries,
                caps.get(2).unwrap().as_str()
            )
        });

        Ok(result.to_string())
    } else {
        // Fallback: try to find services table with different pattern
        let alt_pattern =
            regex::Regex::new(r"(local\s+services\s*=\s*\{[^}]*enabled\s*=\s*\{)([^}]*)").unwrap();

        if let Some(_captures) = alt_pattern.captures(content) {
            let mut new_entries = String::new();
            for svc in services {
                let full_svc = format!("\"{}\"", svc);
                if !content.contains(&full_svc) {
                    new_entries.push_str(&format!("\"{}\", ", svc));
                }
            }

            let result = alt_pattern.replace(content, |caps: &regex::Captures| {
                format!(
                    "{}{}{}",
                    caps.get(1).unwrap().as_str(),
                    new_entries,
                    caps.get(2).unwrap().as_str()
                )
            });

            Ok(result.to_string())
        } else {
            // No services table exists yet — build one and insert before the final closing brace
            let mut entries = String::new();
            for svc in services {
                entries.push_str(&format!("        \"{}\",\n", svc));
            }

            let scope_line = if user_scope {
                "        scope = \"user\",\n"
            } else {
                ""
            };
            let new_table = format!(
                "\n    services = {{\n{}        enabled = {{\n{}        }},\n        disabled = {{}},\n    }},\n",
                scope_line, entries
            );

            let insert_pos = content
                .rfind('}')
                .ok_or_else(|| anyhow::anyhow!("Could not find closing brace in Lua config"))?;

            let mut result = content.to_string();
            result.insert_str(insert_pos, &new_table);
            Ok(result)
        }
    }
}

/// Insert default apps into the default_apps table in Lua content
fn insert_defaults_into_lua(
    content: &str,
    defaults: &std::collections::HashMap<String, String>,
) -> Result<String> {
    // Find the default_apps table pattern
    let pattern = regex::Regex::new(r"(default_apps\s*=\s*\{)([^}]*)(\})").unwrap();

    if let Some(captures) = pattern.captures(content) {
        let existing_entries = captures.get(2).unwrap().as_str();

        // Build new entries string
        let mut new_entries = existing_entries.to_string();
        for (category, desktop_file) in defaults {
            let key = match category.as_str() {
                "browser" => "browser",
                "text_editor" => "text_editor",
                "file_manager" => "file_manager",
                "terminal" => "terminal",
                "video_player" => "video_player",
                "audio_player" => "audio_player",
                "image_viewer" => "image_viewer",
                "pdf_viewer" => "pdf_viewer",
                _ => continue,
            };

            // Clean up the desktop file name
            let clean_name = desktop_file.trim_end_matches(".desktop");
            let entry = format!("{} = \"{}\",", key, clean_name);

            // Only add if not already present
            if !existing_entries.contains(&format!("{} =", key)) {
                if !new_entries.is_empty() && !new_entries.ends_with('\n') {
                    new_entries.push_str("\n        ");
                }
                new_entries.push_str(&entry);
            }
        }

        // Replace in content
        let result = pattern.replace(content, |caps: &regex::Captures| {
            format!(
                "{}{}{}",
                caps.get(1).unwrap().as_str(),
                new_entries,
                caps.get(3).unwrap().as_str()
            )
        });

        Ok(result.to_string())
    } else {
        // No default_apps table exists yet — build one and insert it before the final closing brace
        let mut entries = String::new();
        for (category, desktop_file) in defaults {
            let key = match category.as_str() {
                "browser" => "browser",
                "text_editor" => "text_editor",
                "file_manager" => "file_manager",
                "terminal" => "terminal",
                "video_player" => "video_player",
                "audio_player" => "audio_player",
                "image_viewer" => "image_viewer",
                "pdf_viewer" => "pdf_viewer",
                _ => continue,
            };
            let clean_name = desktop_file.trim_end_matches(".desktop");
            entries.push_str(&format!("        {} = \"{}\",\n", key, clean_name));
        }

        let new_table = format!("\n    default_apps = {{\n{}}},\n", entries);

        // Insert before the final closing brace of the return table
        let insert_pos = content
            .rfind('}')
            .ok_or_else(|| anyhow::anyhow!("Could not find closing brace in Lua config"))?;

        let mut result = content.to_string();
        result.insert_str(insert_pos, &new_table);
        Ok(result)
    }
}

/// Backup the original Lua file and write modified content
fn backup_and_write_lua(host_file: &Path, modified_content: &str) -> Result<std::path::PathBuf> {
    // Create backup path
    let backup_path = create_backup_path(host_file);

    // Rename original to backup
    std::fs::rename(host_file, &backup_path)
        .context(format!("Failed to backup Lua file to {:?}", backup_path))?;

    // Write modified content to original location
    std::fs::write(host_file, modified_content).context(format!(
        "Failed to write modified Lua file: {:?}",
        host_file
    ))?;

    Ok(backup_path)
}

pub fn run(
    paths: &ConfigPaths,
    dry_run: bool,
    services: bool,
    user: bool,
    defaults: bool,
    include_deps: bool,
) -> Result<()> {
    if defaults {
        run_defaults_merge(paths, dry_run)
    } else if services {
        run_services_merge(paths, dry_run, user)
    } else {
        run_packages_merge(paths, dry_run, include_deps)
    }
}

fn run_packages_merge(paths: &ConfigPaths, dry_run: bool, include_deps: bool) -> Result<()> {
    println!("{} Loading configuration...", "→".blue());

    let config = load_config(paths)?;

    // Use declared-packages file (modules/declared-packages.lua or .yaml)
    let (preferred_declared, fallback_declared) = crate::config::declared_packages_paths(paths)?;
    let packages_file = if preferred_declared.exists() {
        preferred_declared
    } else if fallback_declared.exists() {
        fallback_declared
    } else {
        preferred_declared
    };

    // Initialize package manager
    let pkg_manager = PackageManager::new(paths.clone());

    println!("{} Scanning installed packages...", "→".blue());

    // Get explicitly installed packages (not dependencies) via backend
    let backend = crate::backend::create_backend(&config)?;
    let installed_packages: Vec<String> = backend.get_explicit_packages()?;

    println!(
        "{} Found {} explicitly installed packages",
        "→".blue(),
        installed_packages.len().to_string().green()
    );

    // Get installed flatpaks
    let flatpak_scope = match config.flatpak_scope {
        crate::config::FlatpakScope::User => "--user",
        crate::config::FlatpakScope::System => "--system",
    };
    let installed_flatpaks = pkg_manager.get_installed_flatpaks(flatpak_scope)?;

    println!(
        "{} Found {} installed flatpaks ({} scope)",
        "→".blue(),
        installed_flatpaks.len().to_string().green(),
        if flatpak_scope == "--user" {
            "user"
        } else {
            "system"
        }
    );

    // Get all declared packages
    let declared = pkg_manager.get_declared_packages(&config)?;

    let declared_pacman_names: HashSet<String> = declared
        .iter()
        .filter(|p| matches!(p.package_type, crate::config::PackageType::Native))
        .map(|p| p.name.clone())
        .collect();

    let declared_flatpak_names: HashSet<String> = declared
        .iter()
        .filter(|p| matches!(p.package_type, crate::config::PackageType::Flatpak))
        .map(|p| p.name.clone())
        .collect();

    println!(
        "{} Found {} packages declared in config",
        "→".blue(),
        (declared_pacman_names.len() + declared_flatpak_names.len())
            .to_string()
            .green()
    );

    // Find unmanaged packages
    let unmanaged_packages: Vec<String> = installed_packages
        .iter()
        .filter(|pkg| !declared_pacman_names.contains(*pkg))
        .cloned()
        .collect();

    // Find unmanaged flatpaks
    let unmanaged_flatpaks: Vec<String> = installed_flatpaks
        .iter()
        .filter(|pkg| !declared_flatpak_names.contains(*pkg))
        .cloned()
        .collect();

    // Get dependency packages if --include-deps flag is used
    let mut dependency_packages: Vec<String> = Vec::new();
    if include_deps {
        println!(
            "{} Scanning for all installed packages (including dependencies)...",
            "→".blue()
        );

        let all_installed: Vec<String> = backend.get_all_packages()?;
        let explicit_set: HashSet<String> = installed_packages.iter().cloned().collect();
        let declared_set: HashSet<String> = declared_pacman_names.clone();

        // Dependencies are packages that are:
        // 1. Installed (all packages)
        // 2. NOT explicitly installed
        // 3. NOT already declared in config
        dependency_packages = all_installed
            .iter()
            .filter(|pkg| !explicit_set.contains(*pkg) && !declared_set.contains(*pkg))
            .cloned()
            .collect();

        println!(
            "{} Found {} dependency packages",
            "→".blue(),
            dependency_packages.len().to_string().green()
        );
    }

    if unmanaged_packages.is_empty()
        && unmanaged_flatpaks.is_empty()
        && dependency_packages.is_empty()
    {
        println!();
        println!(
            "{} All installed packages and flatpaks are already managed by dcli",
            "✓".green()
        );
        return Ok(());
    }

    if !unmanaged_packages.is_empty() {
        println!(
            "{} Found {} unmanaged packages",
            "→".blue(),
            unmanaged_packages.len().to_string().yellow()
        );
    }

    if !unmanaged_flatpaks.is_empty() {
        println!(
            "{} Found {} unmanaged flatpaks",
            "→".blue(),
            unmanaged_flatpaks.len().to_string().yellow()
        );
    }

    println!();

    // Display unmanaged packages
    println!("{}", "=== Unmanaged Packages ===".blue().bold());
    println!();
    println!("{}", "These are packages you installed manually:".dimmed());
    println!();

    // Display pacman packages
    for pkg in &unmanaged_packages {
        println!("  • {}", pkg);
    }

    // Display flatpaks with indicator
    for pkg in &unmanaged_flatpaks {
        println!("  • {} {}", pkg, "[flatpak]".cyan());
    }

    println!();

    // Dry run mode
    if dry_run {
        println!("{}", "[DRY RUN - No changes will be made]".yellow());
        println!();

        if include_deps && !dependency_packages.is_empty() {
            let deps_module_name = format!("dependencies-{}", config.host);
            println!("These changes would be made:");
            println!();
            println!(
                "  {} Explicit packages → {}",
                unmanaged_packages.len(),
                packages_file.display().to_string().cyan()
            );
            println!(
                "  {} Dependency packages → modules/{}/packages.yaml",
                dependency_packages.len(),
                deps_module_name.cyan()
            );
        } else {
            let total_count = unmanaged_packages.len() + unmanaged_flatpaks.len();
            println!("These {} items would be added to:", total_count);
            println!("  {}", packages_file.display().to_string().cyan());
        }
        println!();
        println!("{}", "What is this module?".bold());
        println!("  • A module containing your manually installed packages");
        println!("  • Enable it in your host config to use these packages");
        println!("  • Handles both pacman packages and flatpaks");
        println!("  • You can gradually move packages to other modules");
        if include_deps {
            println!();
            println!("{}", "Note about dependencies module:".bold());
            println!("  • Contains packages auto-installed as dependencies");
            println!("  • Only needed if you want to track ALL packages");
            println!("  • Usually not needed - dependencies are auto-resolved");
        }
        return Ok(());
    }

    // Safety warning
    println!("{}", "⚠️  Important Safety Information".yellow().bold());
    println!();
    println!("This command captures packages you installed manually. However:");
    println!("  • Review the list carefully before proceeding");
    println!("  • Some packages may be critical to your system");
    println!("  • Removing packages later can break your system");
    println!("  • You are responsible for managing your system");
    println!();
    println!(
        "{}",
        "The dcli author is not responsible for any system issues.".dimmed()
    );
    println!(
        "{}",
        "Always maintain backups and test changes carefully.".dimmed()
    );
    println!();

    // Confirm before modifying
    println!("{}", "What will happen:".bold());
    println!(
        "  • Create/update: {}",
        packages_file.display().to_string().cyan()
    );
    if !unmanaged_packages.is_empty() && !unmanaged_flatpaks.is_empty() {
        println!(
            "  • Add {} packages and {} flatpaks",
            unmanaged_packages.len(),
            unmanaged_flatpaks.len()
        );
    } else if !unmanaged_packages.is_empty() {
        println!("  • Add {} packages", unmanaged_packages.len());
    } else {
        println!("  • Add {} flatpaks", unmanaged_flatpaks.len());
    }

    // Add dependencies module info if flag is used
    if include_deps && !dependency_packages.is_empty() {
        let deps_module_name = format!("dependencies-{}", config.host);
        let deps_module_dir = paths.config_dir.join("modules").join(&deps_module_name);
        println!();
        println!(
            "  • Create dependencies module: {}",
            deps_module_name.cyan()
        );
        println!("    at: {}", deps_module_dir.display().to_string().cyan());
        println!("    with {} dependency packages", dependency_packages.len());
        println!();
        println!("{}", "Note: The dependencies module is optional.".dimmed());
        println!(
            "{}",
            "      Dependencies are usually auto-resolved during install.".dimmed()
        );
    }
    println!();
    print!("Proceed? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("{}", "Cancelled".yellow());
        return Ok(());
    }

    // Load or create system-packages.yaml
    let mut system_list = if packages_file.exists() {
        crate::config::load_package_list(&packages_file)?
    } else {
        crate::config::PackageList {
            description: "Packages installed manually on the system (auto-synced by dcli)"
                .to_string(),
            packages: Vec::new(),
            exclude: Vec::new(),
            conflicts: Vec::new(),
            pre_install_hook: None,
            post_install_hook: None,
            hook_behavior: "ask".to_string(),
            pre_hook_behavior: None,
            post_hook_behavior: None,
            run_hooks_as_user: RunHooksAsUser::Bool(false),
            post_disable_hook: None,
            post_disable_behavior: None,
        }
    };

    // Get existing package names
    let existing_names: HashSet<String> = system_list
        .packages
        .iter()
        .map(|entry| entry.name().to_string())
        .collect();

    // Add unmanaged pacman packages
    let mut added_count = 0;
    for pkg in &unmanaged_packages {
        if !existing_names.contains(pkg) {
            system_list
                .packages
                .push(crate::config::PackageEntry::Simple(pkg.clone()));
            added_count += 1;
        }
    }

    // Add unmanaged flatpaks
    for pkg in &unmanaged_flatpaks {
        if !existing_names.contains(pkg.as_str()) {
            system_list
                .packages
                .push(crate::config::PackageEntry::WithType {
                    name: pkg.clone(),
                    r#type: Some(crate::config::PackageType::Flatpak),
                });
            added_count += 1;
        }
    }

    // Sort packages alphabetically for easier reading
    system_list.packages.sort_by(|a, b| a.name().cmp(b.name()));

    // Save updated packages to declared-packages file
    crate::config::write_package_list_any(&packages_file, &system_list)?;

    // Create dependencies module if --include-deps flag was used
    if include_deps && !dependency_packages.is_empty() {
        let deps_module_name = format!("dependencies-{}", config.host);
        create_dependencies_module(paths, &config, &dependency_packages, true)?;
        deps_module_name
    } else {
        String::new()
    };

    println!();
    if !unmanaged_packages.is_empty() && !unmanaged_flatpaks.is_empty() {
        println!(
            "{} Added {} packages and {} flatpaks",
            "✓".green(),
            unmanaged_packages.len().to_string().green(),
            unmanaged_flatpaks.len().to_string().green()
        );
    } else {
        println!(
            "{} Added {} items",
            "✓".green(),
            added_count.to_string().green()
        );
    }

    println!();
    println!("{}", "What happened:".bold());
    println!(
        "  • {} packages written to {}",
        added_count,
        packages_file.display().to_string().cyan()
    );
    println!();
    println!("  • These packages are automatically loaded by dcli");
    println!("  • No need to enable a module — they're part of declared-packages");
    println!("  • Run 'dcli sync' to install any missing packages");
    println!("  • Gradually move packages to named modules for better organization");

    Ok(())
}

/// Create a module containing all dependency packages
fn create_dependencies_module(
    paths: &ConfigPaths,
    config: &crate::config::Config,
    dependency_packages: &[String],
    _is_lua: bool,
) -> Result<()> {
    let deps_module_name = format!("dependencies-{}", config.host);
    let deps_module_dir = paths.config_dir.join("modules").join(&deps_module_name);
    let deps_packages_file = deps_module_dir.join("packages.yaml");
    let deps_manifest_file = deps_module_dir.join("module.yaml");

    // Check if module already exists
    let module_exists = deps_packages_file.exists() && deps_manifest_file.exists();

    // Get existing packages if module exists
    let existing_count = if module_exists {
        match crate::config::load_package_list_any(&deps_packages_file) {
            Ok(existing_list) => existing_list.packages.len(),
            Err(_) => 0,
        }
    } else {
        0
    };

    // Check if packages have changed
    let packages_changed = dependency_packages.len() != existing_count;

    if module_exists && !packages_changed {
        // Module exists and package count is the same, skip
        println!();
        println!("{} Dependencies module already up to date", "✓".green());
        return Ok(());
    }

    println!();
    if module_exists {
        println!("{} Updating dependencies module...", "→".blue());
    } else {
        println!("{} Creating dependencies module...", "→".blue());
    }

    // Create module directory
    std::fs::create_dir_all(&deps_module_dir).context(format!(
        "Failed to create dependencies module directory: {:?}",
        deps_module_dir
    ))?;

    // Create packages.yaml for dependencies
    let mut deps_list = crate::config::PackageList {
        description: format!(
            "Dependency packages for {} (auto-synced by dcli merge --include-deps)",
            config.host
        ),
        packages: Vec::new(),
        exclude: Vec::new(),
        conflicts: Vec::new(),
        pre_install_hook: None,
        post_install_hook: None,
        hook_behavior: "ask".to_string(),
        pre_hook_behavior: None,
        post_hook_behavior: None,
        run_hooks_as_user: crate::config::RunHooksAsUser::Bool(false),
        post_disable_hook: None,
        post_disable_behavior: None,
    };

    // Add dependency packages
    for pkg in dependency_packages {
        deps_list
            .packages
            .push(crate::config::PackageEntry::Simple(pkg.clone()));
    }

    // Sort packages alphabetically
    deps_list.packages.sort_by(|a, b| a.name().cmp(b.name()));

    // Save packages file
    let yaml = serde_yaml::to_string(&deps_list)
        .context("Failed to serialize dependencies packages.yaml")?;
    std::fs::write(&deps_packages_file, yaml)
        .context("Failed to write dependencies packages.yaml")?;

    // Create module.yaml manifest
    let deps_manifest = crate::config::ModuleManifest {
        description: format!(
            "Dependency packages for {} (auto-synced by dcli merge --include-deps)",
            config.host
        ),
        conflicts: Vec::new(),
        pre_install_hook: None,
        post_install_hook: None,
        hook_behavior: "ask".to_string(),
        pre_hook_behavior: None,
        post_hook_behavior: None,
        run_hooks_as_user: crate::config::RunHooksAsUser::Bool(false),
        post_disable_hook: None,
        post_disable_behavior: None,
        package_files: vec!["packages.yaml".to_string()],
        dotfiles_sync: None,
        dotfiles: Vec::new(),
        author: None,
        version: None,
        category: None,
        tags: Vec::new(),
        license: None,
        upstream_url: None,
    };

    let manifest_yaml = serde_yaml::to_string(&deps_manifest)
        .context("Failed to serialize dependencies module.yaml")?;
    std::fs::write(&deps_manifest_file, manifest_yaml)
        .context("Failed to write dependencies module.yaml")?;

    if module_exists {
        println!(
            "{} Dependencies module updated at: {}",
            "✓".green(),
            deps_module_dir.display().to_string().cyan()
        );
    } else {
        println!(
            "{} Dependencies module created at: {}",
            "✓".green(),
            deps_module_dir.display().to_string().cyan()
        );
    }

    Ok(())
}

fn run_services_merge(paths: &ConfigPaths, dry_run: bool, user_scope: bool) -> Result<()> {
    use crate::config::ServiceScope;
    let scope = if user_scope {
        ServiceScope::User
    } else {
        ServiceScope::System
    };

    println!("{} Loading configuration...", "→".blue());

    let config = load_config(paths)?;

    // Get the host configuration file path
    let host_file = paths.host_packages_file(&config.host);

    println!("{} Scanning enabled services...", "→".blue());

    // Get all currently enabled services
    let all_enabled = ServiceManager::get_all_enabled_services(scope)
        .context("Failed to get enabled services")?;

    println!(
        "{} Found {} enabled services on system",
        "→".blue(),
        all_enabled.len().to_string().green()
    );

    // Filter out system-critical services (only relevant for system scope)
    let manageable_services: Vec<String> = if user_scope {
        all_enabled
    } else {
        let system_services = get_system_critical_services();
        all_enabled
            .into_iter()
            .filter(|s| !system_services.contains(&s.as_str()))
            .collect()
    };

    if user_scope {
        println!(
            "{} {} manageable user services",
            "→".blue(),
            manageable_services.len().to_string().green()
        );
    } else {
        println!(
            "{} {} manageable services (after filtering system-critical)",
            "→".blue(),
            manageable_services.len().to_string().green()
        );
    }

    // Get services already declared in config
    let declared_enabled: HashSet<String> = config.services.enabled.iter().cloned().collect();
    let declared_disabled: HashSet<String> = config.services.disabled.iter().cloned().collect();

    println!(
        "{} Found {} services declared in config ({} enabled, {} disabled)",
        "→".blue(),
        (declared_enabled.len() + declared_disabled.len())
            .to_string()
            .green(),
        declared_enabled.len(),
        declared_disabled.len()
    );

    // Find unmanaged services (enabled but not in config)
    let unmanaged: Vec<String> = manageable_services
        .iter()
        .filter(|s| !declared_enabled.contains(*s) && !declared_disabled.contains(*s))
        .cloned()
        .collect();

    if unmanaged.is_empty() {
        println!();
        println!(
            "{} All enabled services are already managed by dcli or are system-critical",
            "✓".green()
        );
        return Ok(());
    }

    println!(
        "{} Found {} unmanaged services",
        "→".blue(),
        unmanaged.len().to_string().yellow()
    );
    println!();

    // Display unmanaged services
    println!("{}", "=== Unmanaged Services ===".blue().bold());
    println!();
    println!(
        "{}",
        "These services are currently enabled but not in your dcli config:".dimmed()
    );
    println!();
    for svc in &unmanaged {
        println!("  • {}", svc);
    }
    println!();

    // Dry run mode
    if dry_run {
        println!("{}", "[DRY RUN - No changes will be made]".yellow());
        println!();
        println!("These {} services would be added to:", unmanaged.len());
        println!("  {}", host_file.display().to_string().cyan());
        println!();
        println!("{}", "What will happen:".bold());
        println!("  • Services will be added to 'services.enabled' section");
        println!("  • Your host configuration will be updated");
        println!("  • Run 'dcli sync' to keep them enabled");
        return Ok(());
    }

    // Safety warning
    println!("{}", "⚠️  Important Information".yellow().bold());
    println!();
    println!("This command captures services that are currently enabled. However:");
    println!("  • Review the list carefully before proceeding");
    println!("  • Some services may be important for your workflow");
    println!("  • System-critical services are automatically filtered out");
    println!("  • You can remove services from config later if needed");
    println!();
    println!(
        "{}",
        "The dcli author is not responsible for any system issues.".dimmed()
    );
    println!(
        "{}",
        "Always maintain backups and test changes carefully.".dimmed()
    );
    println!();

    // Confirm before modifying
    println!("{}", "What will happen:".bold());
    println!("  • Update: {}", host_file.display().to_string().cyan());
    println!("  • Add {} services to 'services.enabled'", unmanaged.len());
    println!("  • File will be loaded during sync");
    println!();
    print!("Proceed? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("{}", "Cancelled".yellow());
        return Ok(());
    }

    // Load or create host configuration
    let mut host_config = if host_file.exists() {
        if host_file.extension().and_then(|e| e.to_str()) == Some("lua") {
            crate::lua::load_lua_config(&host_file).context("Failed to parse host configuration")?
        } else {
            let content = std::fs::read_to_string(&host_file)
                .context(format!("Failed to read host file: {:?}", host_file))?;
            serde_yaml::from_str::<Config>(&content)
                .context("Failed to parse host configuration")?
        }
    } else {
        Config {
            host: config.host.clone(),
            sops_key_path: None,
            secrets: Vec::new(),
            description: format!("Configuration for {}", config.host),
            import: Vec::new(),
            enabled_modules: Vec::new(),
            packages: Vec::new(),
            exclude: Vec::new(),
            additional_packages: Vec::new(),
            #[allow(deprecated)]
            backup_tool: None,
            #[allow(deprecated)]
            snapper_config: "root".to_string(),
            flatpak_scope: crate::config::FlatpakScope::User,
            auto_prune: false,
            config_backups: crate::config::ConfigBackupsSettings::default(),
            system_backups: crate::config::SystemBackupsSettings::default(),
            services: crate::config::ServicesConfig::default(),
            enabled_service_profiles: Vec::new(),
            update_hooks: crate::config::UpdateHooksConfig::default(),
            default_apps: crate::config::DefaultAppsConfig::default(),
            theming: crate::config::ThemingConfig::default(),
            module_processing: crate::config::ModuleProcessing::Parallel,
            strict_package_order: false,
            package_manager: config.package_manager.clone(),
            editor: None,
            aur_helper: None,
            sync_sudo: false,
            auto_commit: false,
            nix: crate::config::NixConfig::default(),
        }
    };

    // Set the correct scope on the host config
    host_config.services.scope = scope;

    // Get existing enabled services
    let existing_enabled: HashSet<String> = host_config.services.enabled.iter().cloned().collect();

    // Add unmanaged services
    let mut added_count = 0;
    for svc in &unmanaged {
        if !existing_enabled.contains(svc) {
            host_config.services.enabled.push(svc.clone());
            added_count += 1;
        }
    }

    // Sort services alphabetically for easier reading
    host_config.services.enabled.sort();
    host_config.services.disabled.sort();

    // Create hosts directory if it doesn't exist
    if let Some(parent) = host_file.parent() {
        std::fs::create_dir_all(parent)
            .context(format!("Failed to create hosts directory: {:?}", parent))?;
    }

    // Save updated host configuration
    if host_file.extension().and_then(|e| e.to_str()) == Some("lua") {
        // Read existing Lua content and modify it
        let lua_content = std::fs::read_to_string(&host_file)
            .context(format!("Failed to read Lua host file: {:?}", host_file))?;

        let services_to_add: Vec<String> = unmanaged
            .iter()
            .filter(|svc| !existing_enabled.contains(*svc))
            .cloned()
            .collect();

        if !services_to_add.is_empty() {
            let modified_content =
                insert_services_into_lua(&lua_content, &services_to_add, user_scope)?;
            let backup_path = backup_and_write_lua(&host_file, &modified_content)?;

            println!();
            println!("{} Created backup: {}", "⚠".yellow(), backup_path.display());
            println!(
                "{} When ready, delete: {}",
                "ℹ".blue(),
                backup_path.display()
            );
        }
    } else {
        // Save as YAML (original behavior)
        let yaml = serde_yaml::to_string(&host_config)
            .context("Failed to serialize host configuration")?;
        std::fs::write(&host_file, yaml)
            .context(format!("Failed to write host file: {:?}", host_file))?;
    }

    println!();
    println!(
        "{} Added {} services to host configuration",
        "✓".green(),
        added_count.to_string().green()
    );
    println!();
    println!("{}", "What's next:".bold());
    println!("  • These services are now managed by dcli");
    println!("  • File: {}", host_file.display());
    println!("  • Services will remain enabled during 'dcli sync'");
    println!("  • You can move services to modules later for better organization");

    Ok(())
}

/// Get list of system-critical services that should NOT be managed by dcli
/// These are essential services that should always be left alone
fn get_system_critical_services() -> HashSet<&'static str> {
    HashSet::from([
        // Core system services
        "dbus",
        "dbus-broker",
        "systemd-journald",
        "systemd-logind",
        "systemd-udevd",
        "systemd-resolved",
        "systemd-timesyncd",
        "systemd-networkd",
        "systemd-boot-system-token",
        "systemd-tmpfiles-setup",
        "systemd-tmpfiles-clean",
        "systemd-update-utmp",
        "systemd-user-sessions",
        "systemd-vconsole-setup",
        "systemd-sysctl",
        "systemd-modules-load",
        "systemd-random-seed",
        "systemd-remount-fs",
        "systemd-binfmt",
        "systemd-firstboot",
        "systemd-fsck-root",
        "systemd-hwdb-update",
        "systemd-journal-catalog-update",
        "systemd-journal-flush",
        "systemd-machine-id-commit",
        "systemd-quotacheck",
        "systemd-repart",
        "systemd-sysusers",
        "systemd-update-done",
        // Security services
        "polkit",
        "rtkit-daemon",
        // Display and graphics
        "getty@tty1",
        "getty@tty2",
        "getty@tty3",
        "getty@tty4",
        "getty@tty5",
        "getty@tty6",
        "display-manager",
        "gdm",
        "sddm",
        "lightdm",
        // Essential boot services
        "multi-user.target",
        "graphical.target",
        "basic.target",
        "sysinit.target",
        // Kernel/firmware
        "kmod-static-nodes",
        "systemd-update-utmp-runlevel",
    ])
}

fn run_defaults_merge(paths: &ConfigPaths, dry_run: bool) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    println!("{} Loading configuration...", "→".blue());

    let config = load_config(paths)?;

    let host_file = paths.host_packages_file(&config.host);

    println!("{} Scanning current default applications...", "→".blue());

    // Define all MIME types we care about
    let mime_types_to_check = vec![
        (
            "browser",
            vec![
                "text/html",
                "x-scheme-handler/http",
                "x-scheme-handler/https",
            ],
        ),
        ("text_editor", vec!["text/plain"]),
        ("file_manager", vec!["inode/directory"]),
        ("video_player", vec!["video/mp4", "video/x-matroska"]),
        ("audio_player", vec!["audio/mpeg", "audio/x-flac"]),
        ("image_viewer", vec!["image/png", "image/jpeg"]),
        ("pdf_viewer", vec!["application/pdf"]),
    ];

    let mut discovered_apps: HashMap<String, String> = HashMap::new();
    let mut discovered_desktop_files: HashSet<String> = HashSet::new();

    // Query current defaults for each category
    for (category, mime_list) in mime_types_to_check {
        // Try the first MIME type in the list as representative
        if let Some(mime_type) = mime_list.first() {
            match DefaultsManager::get_current_default_for_mime(mime_type) {
                Ok(Some(desktop_file)) => {
                    // Verify desktop file exists
                    if DefaultsManager::desktop_file_exists(&desktop_file) {
                        discovered_apps.insert(category.to_string(), desktop_file.clone());
                        discovered_desktop_files.insert(desktop_file);
                    }
                }
                Ok(None) => {
                    // No default set for this category
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to query default for {}: {}",
                        "⚠".yellow(),
                        category,
                        e
                    );
                }
            }
        }
    }

    println!(
        "{} Found {} default applications currently set",
        "→".blue(),
        discovered_apps.len().to_string().green()
    );

    // Get already declared defaults
    let declared_apps = config.default_apps.to_apps_map();

    println!(
        "{} Found {} default apps declared in config",
        "→".blue(),
        declared_apps.len().to_string().green()
    );

    // Find unmanaged defaults
    let unmanaged: HashMap<String, String> = discovered_apps
        .iter()
        .filter(|(category, _)| !declared_apps.contains_key(*category))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if unmanaged.is_empty() {
        println!();
        println!(
            "{} All current default applications are already managed by dcli",
            "✓".green()
        );
        return Ok(());
    }

    println!(
        "{} Found {} unmanaged default applications",
        "→".blue(),
        unmanaged.len().to_string().yellow()
    );
    println!();

    // Display unmanaged defaults
    println!("{}", "=== Unmanaged Default Applications ===".blue().bold());
    println!();
    println!(
        "{}",
        "These default applications are set but not in your dcli config:".dimmed()
    );
    println!();
    for (category, desktop_file) in &unmanaged {
        println!(
            "  • {}: {}",
            category.replace("_", " "),
            desktop_file.trim_end_matches(".desktop")
        );
    }
    println!();

    // Dry run mode
    if dry_run {
        println!("{}", "[DRY RUN - No changes will be made]".yellow());
        println!();
        println!("These {} defaults would be added to:", unmanaged.len());
        println!("  {}", host_file.display().to_string().cyan());
        println!();
        println!("{}", "What will happen:".bold());
        println!("  • Defaults will be added to 'default_apps' section");
        println!("  • Your host configuration will be updated");
        println!("  • Run 'dcli sync' to maintain these settings");
        return Ok(());
    }

    // Safety information
    println!("{}", "ℹ️  Information".blue().bold());
    println!();
    println!("This command captures your current default application settings.");
    println!("  • Review the list carefully before proceeding");
    println!("  • These settings will be managed by dcli going forward");
    println!("  • You can modify them later in your host config");
    println!();

    // Confirm before modifying
    println!("{}", "What will happen:".bold());
    println!("  • Update: {}", host_file.display().to_string().cyan());
    println!("  • Add {} defaults to 'default_apps'", unmanaged.len());
    println!("  • Settings will be applied during sync");
    println!();
    print!("Proceed? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("{}", "Cancelled".yellow());
        return Ok(());
    }

    // Load or create host configuration
    let mut host_config = if host_file.exists() {
        if host_file.extension().and_then(|e| e.to_str()) == Some("lua") {
            crate::lua::load_lua_config(&host_file).context("Failed to parse host configuration")?
        } else {
            let content = std::fs::read_to_string(&host_file)
                .context(format!("Failed to read host file: {:?}", host_file))?;
            serde_yaml::from_str::<Config>(&content)
                .context("Failed to parse host configuration")?
        }
    } else {
        Config {
            host: config.host.clone(),
            sops_key_path: None,
            secrets: Vec::new(),
            description: format!("Configuration for {}", config.host),
            import: Vec::new(),
            enabled_modules: Vec::new(),
            packages: Vec::new(),
            exclude: Vec::new(),
            additional_packages: Vec::new(),
            #[allow(deprecated)]
            backup_tool: None,
            #[allow(deprecated)]
            snapper_config: "root".to_string(),
            flatpak_scope: crate::config::FlatpakScope::User,
            auto_prune: false,
            config_backups: crate::config::ConfigBackupsSettings::default(),
            system_backups: crate::config::SystemBackupsSettings::default(),
            services: crate::config::ServicesConfig::default(),
            enabled_service_profiles: Vec::new(),
            update_hooks: crate::config::UpdateHooksConfig::default(),
            default_apps: crate::config::DefaultAppsConfig::default(),
            theming: crate::config::ThemingConfig::default(),
            module_processing: crate::config::ModuleProcessing::Parallel,
            strict_package_order: false,
            package_manager: config.package_manager.clone(),
            editor: None,
            aur_helper: None,
            sync_sudo: false,
            auto_commit: false,
            nix: crate::config::NixConfig::default(),
        }
    };

    // Add unmanaged defaults
    let mut added_count = 0;
    for (category, desktop_file) in &unmanaged {
        // Strip .desktop suffix for cleaner config
        let clean_name = desktop_file.trim_end_matches(".desktop").to_string();

        match category.as_str() {
            "browser" if host_config.default_apps.browser.is_none() => {
                host_config.default_apps.browser = Some(clean_name);
                added_count += 1;
            }
            "text_editor" if host_config.default_apps.text_editor.is_none() => {
                host_config.default_apps.text_editor = Some(clean_name);
                added_count += 1;
            }
            "file_manager" if host_config.default_apps.file_manager.is_none() => {
                host_config.default_apps.file_manager = Some(clean_name);
                added_count += 1;
            }
            "terminal" if host_config.default_apps.terminal.is_none() => {
                host_config.default_apps.terminal = Some(clean_name);
                added_count += 1;
            }
            "video_player" if host_config.default_apps.video_player.is_none() => {
                host_config.default_apps.video_player = Some(clean_name);
                added_count += 1;
            }
            "audio_player" if host_config.default_apps.audio_player.is_none() => {
                host_config.default_apps.audio_player = Some(clean_name);
                added_count += 1;
            }
            "image_viewer" if host_config.default_apps.image_viewer.is_none() => {
                host_config.default_apps.image_viewer = Some(clean_name);
                added_count += 1;
            }
            "pdf_viewer" if host_config.default_apps.pdf_viewer.is_none() => {
                host_config.default_apps.pdf_viewer = Some(clean_name);
                added_count += 1;
            }
            _ => {}
        }
    }

    // Create hosts directory if it doesn't exist
    if let Some(parent) = host_file.parent() {
        std::fs::create_dir_all(parent)
            .context(format!("Failed to create hosts directory: {:?}", parent))?;
    }

    // Save updated host configuration
    if host_file.extension().and_then(|e| e.to_str()) == Some("lua") {
        // Read existing Lua content and modify it
        let lua_content = std::fs::read_to_string(&host_file)
            .context(format!("Failed to read Lua host file: {:?}", host_file))?;

        if !unmanaged.is_empty() {
            let modified_content = insert_defaults_into_lua(&lua_content, &unmanaged)?;
            let backup_path = backup_and_write_lua(&host_file, &modified_content)?;

            println!();
            println!("{} Created backup: {}", "⚠".yellow(), backup_path.display());
            println!(
                "{} When ready, delete: {}",
                "ℹ".blue(),
                backup_path.display()
            );
        }
    } else {
        // Save as YAML (original behavior)
        let yaml = serde_yaml::to_string(&host_config)
            .context("Failed to serialize host configuration")?;
        std::fs::write(&host_file, yaml)
            .context(format!("Failed to write host file: {:?}", host_file))?;
    }

    println!();
    println!(
        "{} Added {} default applications to host configuration",
        "✓".green(),
        added_count.to_string().green()
    );
    println!();
    println!("{}", "What's next:".bold());
    println!("  • These defaults are now managed by dcli");
    println!("  • File: {}", host_file.display());
    println!("  • Defaults will be maintained during 'dcli sync'");
    println!("  • Edit the config to change your default applications");

    Ok(())
}
