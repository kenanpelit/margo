use anyhow::Result;
use colored::*;
use serde_json::json;
use std::collections::HashMap;
use std::process::Command;
use walkdir::WalkDir;

use crate::config::{load_config, ConfigPaths};
use crate::lua::service_profile::load_service_profile;
use crate::package::PackageManager;

pub fn run(paths: &ConfigPaths, check_packages: bool, json: bool) -> Result<()> {
    run_internal(paths, check_packages, json, false)
}

pub fn run_quiet(paths: &ConfigPaths, check_packages: bool, json: bool) -> Result<()> {
    run_internal(paths, check_packages, json, true)
}

fn run_internal(paths: &ConfigPaths, check_packages: bool, json: bool, quiet: bool) -> Result<()> {
    if !json && !quiet {
        println!("{}", "=== Validating mdots Config ===".blue().bold());
        println!();
    }

    let mut errors = 0;
    let mut warnings = 0;
    let mut checks = Vec::new();

    // 1. Check if mdots config directory exists
    if !json && !quiet {
        println!("{} Checking mdots config directory...", "→".blue());
    }
    if !paths.config_dir.exists() {
        if json {
            checks.push(json!({
                "check": "config_dir",
                "status": "error",
                "message": format!("mdots config directory not found: {}", paths.config_dir.display())
            }));
        } else {
            println!(
                "  {} mdots config directory not found: {}",
                "✗".red(),
                paths.config_dir.display()
            );
            println!("    Run 'mdots init' to create it");
        }
        anyhow::bail!("mdots config directory not found");
    }
    if !json && !quiet {
        println!("  {} Directory exists", "✓".green());
    }
    checks.push(json!({
        "check": "config_dir",
        "status": "ok"
    }));

    // 2. Check config (config.yaml, config.lua, or config.nix)
    let lua_config_file = paths.config_dir.join("config.lua");
    let nix_config_file = paths.config_dir.join("config.nix");
    let config_path = if lua_config_file.exists() {
        lua_config_file
    } else if nix_config_file.exists() {
        nix_config_file
    } else {
        paths.config_file.clone()
    };
    let has_config = config_path.exists();
    if !json && !quiet {
        println!("{} Checking config file...", "→".blue());
    }
    if !has_config {
        errors += 1;
        if json {
            checks.push(json!({
                "check": "config",
                "status": "error",
                "message": "config.yaml, config.lua, or config.nix not found"
            }));
        } else {
            println!(
                "  {} config file not found (config.yaml, config.lua, or config.nix)",
                "✗".red()
            );
        }
    } else {
        match load_config(paths) {
            Ok(config) => {
                if !json && !quiet {
                    println!(
                        "  {} Valid config syntax ({})",
                        "✓".green(),
                        config_path.file_name().unwrap().to_string_lossy()
                    );
                }

                if config.host.is_empty() {
                    warnings += 1;
                    if json {
                        checks.push(json!({
                            "check": "config_host",
                            "status": "warning",
                            "message": "Missing 'host' field in config.yaml"
                        }));
                    } else {
                        println!("  {} Missing 'host' field in config.yaml", "⚠".yellow());
                    }
                } else {
                    if !json && !quiet {
                        println!("  {} Host configured: {}", "✓".green(), config.host);
                    }
                    checks.push(json!({
                        "check": "config_host",
                        "status": "ok",
                        "host": config.host
                    }));
                }

                if !json && !quiet {
                    println!("  {} enabled_modules is valid array", "✓".green());
                }
                checks.push(json!({
                    "check": "config_modules",
                    "status": "ok"
                }));
            }
            Err(e) => {
                errors += 1;
                if json {
                    checks.push(json!({
                        "check": "config_yaml",
                        "status": "error",
                        "message": format!("config has invalid syntax: {}", e)
                    }));
                } else {
                    println!("  {} config has invalid syntax: {}", "✗".red(), e);
                }
            }
        }
    }

    // 3. Check packages directory structure (old structure - now optional)
    if !json && !quiet {
        println!("{} Checking packages directory structure...", "→".blue());
    }
    if !paths.packages_dir.exists() {
        // This is OK for new structure - just inform user
        if !json && !quiet {
            println!(
                "  {} packages directory not found (using new structure)",
                "✓".green()
            );
        }
    } else {
        if !json && !quiet {
            println!("  {} packages directory exists", "✓".green());
        }

        // Check for base.yaml
        let base_file = paths.packages_dir.join("base.yaml");
        if base_file.exists() {
            match std::fs::read_to_string(&base_file) {
                Ok(content) => {
                    if serde_yaml::from_str::<serde_yaml::Value>(&content).is_ok() {
                        if !json && !quiet {
                            println!("  {} base.yaml is valid", "✓".green());
                        }
                        checks.push(json!({
                            "check": "base_yaml",
                            "status": "ok"
                        }));
                    } else {
                        errors += 1;
                        if json {
                            checks.push(json!({
                                "check": "base_yaml",
                                "status": "error",
                                "message": "base.yaml has invalid YAML syntax"
                            }));
                        } else {
                            println!("  {} base.yaml has invalid YAML syntax", "✗".red());
                        }
                    }
                }
                Err(e) => {
                    errors += 1;
                    if json {
                        checks.push(json!({
                            "check": "base_yaml",
                            "status": "error",
                            "message": format!("Failed to read base.yaml: {}", e)
                        }));
                    } else {
                        println!("  {} Failed to read base.yaml: {}", "✗".red(), e);
                    }
                }
            }
        } else {
            warnings += 1;
            if json {
                checks.push(json!({
                    "check": "base_yaml",
                    "status": "warning",
                    "message": "base.yaml not found (optional)"
                }));
            } else {
                println!("  {} base.yaml not found (optional)", "⚠".yellow());
            }
        }

        // Check hosts directory
        let hosts_dir = paths.hosts_dir();
        if hosts_dir.exists() {
            if !json && !quiet {
                println!("  {} hosts directory exists", "✓".green());
            }
            checks.push(json!({
                "check": "hosts_dir",
                "status": "ok"
            }));
        } else {
            warnings += 1;
            if json {
                checks.push(json!({
                    "check": "hosts_dir",
                    "status": "warning",
                    "message": "hosts directory not found"
                }));
            } else {
                println!("  {} hosts directory not found", "⚠".yellow());
            }
        }
    }

    // 4. Check modules directory structure
    if !json && !quiet {
        println!("{} Checking modules directory...", "→".blue());
    }
    let modules_dir = paths.modules_dir();
    if !modules_dir.exists() {
        errors += 1;
        if json {
            checks.push(json!({
                "check": "modules_dir",
                "status": "error",
                "message": "modules directory not found"
            }));
        } else {
            println!("  {} modules directory not found", "✗".red());
        }
    } else {
        if !json && !quiet {
            println!("  {} modules directory exists", "✓".green());
        }

        // Track module names for duplicates
        let mut module_names: HashMap<String, String> = HashMap::new();
        let mut module_count = 0;

        // Find all YAML and Lua files in modules
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str());
            let is_yaml = ext == Some("yaml");
            let is_lua = ext == Some("lua");
            let is_nix = ext == Some("nix");

            if !is_yaml && !is_lua && !is_nix {
                continue;
            }

            module_count += 1;

            let rel_path = path.strip_prefix(&modules_dir).unwrap();
            let module_name = rel_path
                .to_string_lossy()
                .trim_end_matches(".yaml")
                .trim_end_matches(".lua")
                .to_string();
            let base_name = path.file_stem().unwrap().to_string_lossy().to_string();

            // Skip module.yaml files - they're part of directory modules, not standalone modules
            if base_name == "module" {
                continue;
            }

            // Skip any YAML/Lua/Nix files that are inside a directory module
            // (i.e., if there's a module.yaml, module.lua, or module.nix in the same directory)
            if let Some(parent) = path.parent() {
                if parent != modules_dir
                    && (parent.join("module.yaml").exists()
                        || parent.join("module.lua").exists()
                        || parent.join("module.nix").exists())
                {
                    continue;
                }
            }

            if is_yaml {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        if serde_yaml::from_str::<serde_yaml::Value>(&content).is_err() {
                            errors += 1;
                            if !json && !quiet {
                                println!("  {} Invalid YAML syntax: {}", "✗".red(), module_name);
                            }
                        }
                    }
                    Err(e) => {
                        errors += 1;
                        if !json && !quiet {
                            println!("  {} Failed to read {}: {}", "✗".red(), module_name, e);
                        }
                    }
                }
            } else if is_lua {
                let lua_result = crate::lua::validate_lua_module_detailed(path);

                if lua_result.valid {
                    if !json && !quiet {
                        println!("  {} Valid Lua module: {}", "✓".green(), module_name);
                    }
                    for warning in &lua_result.warnings {
                        warnings += 1;
                        if !json && !quiet {
                            println!("    {} {}", "⚠".yellow(), warning);
                        }
                    }
                } else {
                    errors += lua_result.errors.len();
                    warnings += lua_result.warnings.len();
                    if !json && !quiet {
                        println!("  {} Invalid Lua module: {}", "✗".red(), module_name);
                        for error in &lua_result.errors {
                            println!("    {} {}", "✗".red(), error.message);
                            if let Some(line) = error.line {
                                println!("      Line: {}", line);
                            }
                            if let Some(hint) = &error.hint {
                                println!("      {}: {}", "HINT".cyan(), hint);
                            }
                        }
                        for warning in &lua_result.warnings {
                            println!("    {} {}", "⚠".yellow(), warning);
                        }
                    }
                }
            } else if is_nix {
                let nix_result = crate::nix_eval::validate_nix_module_detailed(path);

                if nix_result.valid {
                    if !json && !quiet {
                        println!("  {} Valid Nix module: {}", "✓".green(), module_name);
                    }
                    for warning in &nix_result.warnings {
                        warnings += 1;
                        if !json && !quiet {
                            println!("    {} {}", "⚠".yellow(), warning);
                        }
                    }
                } else {
                    errors += nix_result.errors.len();
                    warnings += nix_result.warnings.len();
                    if !json && !quiet {
                        println!("  {} Invalid Nix module: {}", "✗".red(), module_name);
                        for error in &nix_result.errors {
                            println!("    {} {}", "✗".red(), error);
                        }
                        for warning in &nix_result.warnings {
                            println!("    {} {}", "⚠".yellow(), warning);
                        }
                    }
                }
            }

            // Check for duplicate base names
            if let Some(existing) = module_names.get(&base_name) {
                errors += 1;
                if !json && !quiet {
                    println!(
                        "  {} Duplicate module base name: '{}'",
                        "✗".red(),
                        base_name
                    );
                    println!("      Conflicts between:");
                    println!("        - {}", existing);
                    println!("        - {}", module_name);
                    println!(
                        "      Users won't know which one to enable with 'mdots module enable {}'",
                        base_name
                    );
                }
            } else {
                module_names.insert(base_name, module_name.clone());
            }

            // Check nesting depth
            let depth = module_name.matches('/').count();
            if depth > 1 {
                warnings += 1;
                if !json && !quiet {
                    println!(
                        "  {} Module nested deeper than recommended (max 1 level): {}",
                        "⚠".yellow(),
                        module_name
                    );
                }
            }
        }

        if !json && !quiet {
            println!("  {} Found {} module(s)", "✓".green(), module_count);
        }
        checks.push(json!({
            "check": "modules_count",
            "status": "ok",
            "count": module_count
        }));

        // Check for naming conflicts (file + directory with same name)
        for entry in WalkDir::new(&modules_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_dir() {
                continue;
            }

            let dir_path = entry.path();
            if dir_path == modules_dir {
                continue;
            }

            let dir_name = dir_path.file_name().unwrap().to_string_lossy();
            let yaml_conflict = modules_dir.join(format!("{}.yaml", dir_name));
            let lua_conflict = modules_dir.join(format!("{}.lua", dir_name));

            if yaml_conflict.exists() || lua_conflict.exists() {
                errors += 1;
                if !json && !quiet {
                    let conflict_type = if yaml_conflict.exists() {
                        "yaml"
                    } else {
                        "lua"
                    };
                    println!(
                        "  {} Naming conflict: Both '{}.{}' and '{}/' directory exist",
                        "✗".red(),
                        dir_name,
                        conflict_type,
                        dir_name
                    );
                    println!("      This creates ambiguity - rename one of them");
                }
            }
        }
    }

    // 5. Validate enabled modules exist
    if !json && !quiet {
        println!("{} Checking enabled modules...", "→".blue());
    }
    if has_config {
        if let Ok(config) = load_config(paths) {
            if config.enabled_modules.is_empty() {
                if !json && !quiet {
                    println!("  {} No modules enabled", "✓".green());
                }
            } else {
                let mut all_exist = true;
                for module in &config.enabled_modules {
                    let module_yaml = modules_dir.join(format!("{}.yaml", module));
                    let module_lua = modules_dir.join(format!("{}.lua", module));
                    let module_nix = modules_dir.join(format!("{}.nix", module));
                    let module_dir = modules_dir.join(module);

                    let exists = module_yaml.exists()
                        || module_lua.exists()
                        || module_nix.exists()
                        || (module_dir.exists()
                            && (module_dir.join("module.yaml").exists()
                                || module_dir.join("module.lua").exists()
                                || module_dir.join("module.nix").exists()));

                    if !exists {
                        errors += 1;
                        all_exist = false;
                        if !json && !quiet {
                            println!("  {} Enabled module not found: {}", "✗".red(), module);
                            println!(
                                "      Expected: {}, {}, {}, {}/module.yaml, {}/module.lua, or {}/module.nix",
                                module_yaml.display(),
                                module_lua.display(),
                                module_nix.display(),
                                module_dir.display(),
                                module_dir.display(),
                                module_dir.display()
                            );
                        }
                    }
                }

                if all_exist && !json {
                    println!(
                        "  {} All {} enabled module(s) exist",
                        "✓".green(),
                        config.enabled_modules.len()
                    );
                }
            }
        }
    }

    // 6. Validate services configuration
    if !json && !quiet {
        println!("{} Checking services configuration...", "→".blue());
    }
    if has_config {
        if let Ok(config) = load_config(paths) {
            let total_services = config.services.enabled.len() + config.services.disabled.len();

            if total_services == 0 {
                if !json && !quiet {
                    println!("  {} No services configured", "✓".green());
                }
            } else {
                // Validate that systemctl is available
                let systemctl_available = Command::new("which")
                    .arg("systemctl")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);

                if !systemctl_available {
                    errors += 1;
                    if !json && !quiet {
                        println!(
                            "  {} systemctl not found but services are configured",
                            "✗".red()
                        );
                        println!("      Services feature requires systemd");
                    }
                } else {
                    // Validate service names (basic check - must not be empty)
                    let mut invalid_services = Vec::new();

                    for service in config
                        .services
                        .enabled
                        .iter()
                        .chain(config.services.disabled.iter())
                    {
                        if service.trim().is_empty() {
                            invalid_services.push(service.clone());
                        }
                    }

                    if !invalid_services.is_empty() {
                        errors += invalid_services.len();
                        if !json && !quiet {
                            println!(
                                "  {} Found {} invalid service name(s):",
                                "✗".red(),
                                invalid_services.len()
                            );
                            for svc in &invalid_services {
                                println!("      - '{}'", svc);
                            }
                        }
                    } else {
                        if !json && !quiet {
                            println!(
                                "  {} {} service(s) configured ({} enabled, {} disabled)",
                                "✓".green(),
                                total_services,
                                config.services.enabled.len(),
                                config.services.disabled.len()
                            );
                        }
                        checks.push(json!({
                            "check": "services",
                            "status": "ok",
                            "enabled": config.services.enabled.len(),
                            "disabled": config.services.disabled.len()
                        }));
                    }
                }
            }
        }
    }

    // 7. Validate default applications configuration
    if !json && !quiet {
        println!("{} Checking default applications...", "→".blue());
    }
    if has_config {
        if let Ok(config) = load_config(paths) {
            let apps_map = config.default_apps.to_apps_map();
            let total_defaults = apps_map.len() + config.default_apps.mime_types.len();

            if total_defaults == 0 {
                if !json && !quiet {
                    println!("  {} No default applications configured", "✓".green());
                }
            } else {
                // Validate that xdg-mime is available
                let xdg_mime_available = Command::new("which")
                    .arg("xdg-mime")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);

                if !xdg_mime_available {
                    warnings += 1;
                    if !json && !quiet {
                        println!(
                            "  {} xdg-mime not found but default apps are configured",
                            "⚠".yellow()
                        );
                        println!("      Install xdg-utils to enable default applications feature");
                    }
                } else {
                    // Validate .desktop files exist
                    let mut invalid_apps = Vec::new();

                    for (app_type, desktop_file) in &apps_map {
                        let resolved = if desktop_file.ends_with(".desktop") {
                            desktop_file.clone()
                        } else {
                            format!("{}.desktop", desktop_file)
                        };

                        // Check if .desktop file exists in standard locations
                        let desktop_exists = [
                            format!("/usr/share/applications/{}", resolved),
                            format!("/usr/local/share/applications/{}", resolved),
                            format!(
                                "{}/.local/share/applications/{}",
                                std::env::var("HOME").unwrap_or_default(),
                                resolved
                            ),
                        ]
                        .iter()
                        .any(|path| std::path::Path::new(path).exists());

                        if !desktop_exists {
                            invalid_apps.push((app_type.clone(), desktop_file.clone()));
                        }
                    }

                    if !invalid_apps.is_empty() {
                        errors += invalid_apps.len();
                        if !json && !quiet {
                            println!(
                                "  {} Found {} invalid .desktop file(s):",
                                "✗".red(),
                                invalid_apps.len()
                            );
                            for (app_type, desktop_file) in &invalid_apps {
                                println!("      - {} -> {}", app_type, desktop_file);
                            }
                            println!(
                                "      These .desktop files don't exist in standard locations"
                            );
                        }
                    } else {
                        if !json && !quiet {
                            println!(
                                "  {} {} default application(s) configured",
                                "✓".green(),
                                total_defaults
                            );
                        }
                        checks.push(json!({
                            "check": "default_apps",
                            "status": "ok",
                            "count": total_defaults
                        }));
                    }
                }
            }
        }
    }

    // 8. Check state directory
    if !json && !quiet {
        println!("{} Checking state directory...", "→".blue());
    }
    if !paths.state_dir.exists() {
        warnings += 1;
        if json {
            checks.push(json!({
                "check": "state_dir",
                "status": "warning",
                "message": "state directory not found (will be created on first sync)"
            }));
        } else {
            println!(
                "  {} state directory not found (will be created on first sync)",
                "⚠".yellow()
            );
        }
    } else {
        if !json && !quiet {
            println!("  {} state directory exists", "✓".green());
        }

        let gitignore = paths.state_dir.join(".gitignore");
        if !gitignore.exists() {
            warnings += 1;
            if !json && !quiet {
                println!(
                    "  {} state/.gitignore not found (state files may be committed to git)",
                    "⚠".yellow()
                );
            }
        }
    }

    // 9. Validate package existence (optional)
    if check_packages {
        if !json && !quiet {
            println!(
                "{} Checking if packages exist in repositories...",
                "→".blue()
            );
        }

        if let Ok(config) = load_config(paths) {
            let backend = crate::backend::create_backend(&config).ok();
            if let Some(backend) = backend {
                let pkg_manager = PackageManager::new(paths.clone());
                if let Ok(declared) = pkg_manager.get_declared_packages(&config) {
                    let native_packages: Vec<_> = declared
                        .iter()
                        .filter(|p| matches!(p.package_type, crate::config::PackageType::Native))
                        .map(|p| p.name.as_str())
                        .collect();

                    if native_packages.is_empty() {
                        if !json && !quiet {
                            println!("  {} No packages to validate", "✓".green());
                        }
                    } else {
                        if !json && !quiet {
                            println!(
                                "  {} Checking {} package(s) (this may take a moment)...",
                                "ℹ".blue(),
                                native_packages.len()
                            );
                        }

                        let invalid = check_packages_with_backend(&native_packages, &*backend)?;

                        if invalid.is_empty() {
                            if !json && !quiet {
                                println!(
                                    "  {} All {} package(s) exist in repositories",
                                    "✓".green(),
                                    native_packages.len()
                                );
                            }
                            checks.push(json!({
                                "check": "packages_exist",
                                "status": "ok",
                                "count": native_packages.len()
                            }));
                        } else {
                            errors += invalid.len();
                            if json {
                                checks.push(json!({
                                    "check": "packages_exist",
                                    "status": "error",
                                    "invalid_packages": invalid
                                }));
                            } else {
                                println!(
                                    "  {} Found {} invalid package name(s):",
                                    "✗".red(),
                                    invalid.len()
                                );
                                for pkg in &invalid {
                                    println!("      - {}", pkg);
                                }
                                println!(
                                    "  {} These packages will fail during 'mdots sync'",
                                    "ℹ".blue()
                                );
                            }
                        }
                    }
                }
            } else {
                warnings += 1;
                if !json && !quiet {
                    println!(
                        "  {} Could not create package backend - skipping package validation",
                        "⚠".yellow()
                    );
                }
            }
        }
    }

    // 7. Validate service profiles
    let services_dir = paths.services_dir();
    if services_dir.exists() {
        if !json && !quiet {
            println!("{} Checking service profiles...", "→".blue());
        }

        let mut profile_count = 0;
        let mut profile_errors = 0;

        for entry in WalkDir::new(&services_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            if path.is_dir() {
                continue;
            }

            profile_count += 1;
            let relative_name = path
                .strip_prefix(&services_dir)
                .unwrap_or(path)
                .with_extension("")
                .to_string_lossy()
                .to_string();

            match load_service_profile(path) {
                Ok(_) => {
                    if !json && !quiet {
                        println!("  {} {}", "✓".green(), relative_name);
                    }
                }
                Err(e) => {
                    profile_errors += 1;
                    errors += 1;
                    if json {
                        checks.push(json!({
                            "check": "service_profile",
                            "status": "error",
                            "profile": relative_name,
                            "message": format!("{}", e)
                        }));
                    } else {
                        println!("  {} {} - {}", "✗".red(), relative_name, e);
                    }
                }
            }
        }

        if profile_count == 0 {
            if !json && !quiet {
                println!("  {} No service profiles found", "ℹ".blue());
            }
        } else if profile_errors == 0 {
            if !json && !quiet {
                println!(
                    "  {} All {} service profile(s) valid",
                    "✓".green(),
                    profile_count
                );
            }
            checks.push(json!({
                "check": "service_profiles",
                "status": "ok",
                "count": profile_count
            }));
        }
    }

    // Summary
    if json {
        println!(
            "{}",
            json!({
                "valid": errors == 0,
                "errors": errors,
                "warnings": warnings,
                "checks": checks
            })
        );
    } else if quiet {
        // Quiet mode - only show brief result
        if errors == 0 && warnings == 0 {
            println!("{}", "✓ Configuration is valid".green());
        } else if errors == 0 {
            println!("{}", "⚠ Configuration is valid with warnings".yellow());
        } else {
            println!("{}", "✗ Configuration has errors".red());
            anyhow::bail!("Validation failed");
        }
    } else {
        println!();
        println!("{}", "=== Validation Summary ===".blue().bold());
        println!();

        if errors == 0 && warnings == 0 {
            println!("{}", "✓ Configuration is valid!".green());
            println!();
            println!("No errors or warnings found.");
        } else if errors == 0 {
            println!("{}", "⚠ Configuration is valid with warnings".yellow());
            println!();
            println!("Warnings: {}", warnings);
            println!();
            println!(
                "Your configuration will work, but you may want to address the warnings above."
            );
        } else {
            println!("{}", "✗ Configuration has errors".red());
            println!();
            println!("Errors: {}", errors);
            println!("Warnings: {}", warnings);
            println!();
            println!("Please fix the errors above before running 'mdots sync'.");
            anyhow::bail!("Validation failed");
        }
    }

    Ok(())
}

/// Check packages using the backend's check_package_exists method
fn check_packages_with_backend(
    packages: &[&str],
    backend: &dyn crate::backend::PkgBackend,
) -> Result<Vec<String>> {
    let mut invalid = Vec::new();

    for &pkg in packages {
        if backend.check_package_exists(pkg) {
            continue;
        }

        invalid.push(pkg.to_string());
    }

    Ok(invalid)
}
