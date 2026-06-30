use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::io::{self, Write};

use crate::config::{load_config, load_module, ConfigPaths};
use crate::module::ModuleManager;

#[derive(Serialize)]
struct ModuleListOutput {
    modules: Vec<ModuleJsonInfo>,
}

#[derive(Serialize)]
struct ModuleJsonInfo {
    name: String,
    description: String,
    package_count: usize,
    conflicts: Vec<String>,
    enabled: bool,
    category: Option<String>,
}

#[derive(Serialize)]
struct ModuleActionOutput {
    success: bool,
    message: String,
    module: String,
    action: String,
}

#[derive(Serialize)]
struct ModuleBatchActionOutput {
    results: Vec<ModuleActionOutput>,
}

struct EnableOutcome {
    output: ModuleActionOutput,
    changed: bool,
}

pub fn list(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let module_manager = ModuleManager::new(paths.clone());
    let modules = module_manager.list_modules()?;

    if modules.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&ModuleListOutput { modules: vec![] })?
            );
        } else {
            println!("{}", "No modules found".yellow());
        }
        return Ok(());
    }

    if json {
        // JSON output
        let module_infos: Vec<ModuleJsonInfo> = modules
            .iter()
            .map(|module| {
                let is_enabled = config.enabled_modules.contains(&module.name);
                let category = if module.name.contains('/') {
                    Some(module.name.split('/').next().unwrap().to_string())
                } else {
                    None
                };

                ModuleJsonInfo {
                    name: module.name.clone(),
                    description: module.description.clone(),
                    package_count: module.package_count,
                    conflicts: module.conflicts.clone(),
                    enabled: is_enabled,
                    category,
                }
            })
            .collect();

        let output = ModuleListOutput {
            modules: module_infos,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Human-readable output
        println!("{}", "=== Available Modules ===".blue().bold());
        println!();

        // Group modules by category
        let mut root_modules = Vec::new();
        let mut categorized: std::collections::HashMap<String, Vec<_>> =
            std::collections::HashMap::new();

        for module in modules {
            let is_enabled = config.enabled_modules.contains(&module.name);

            if module.name.contains('/') {
                let category = module.name.split('/').next().unwrap().to_string();
                categorized
                    .entry(category)
                    .or_default()
                    .push((module, is_enabled));
            } else {
                root_modules.push((module, is_enabled));
            }
        }

        // Display root-level modules
        for (module, is_enabled) in root_modules {
            display_module(&module, is_enabled);
        }

        // Display categorized modules
        let mut categories: Vec<_> = categorized.keys().collect();
        categories.sort();

        for category in categories {
            println!("{}", format!("{}:", category).cyan().bold());
            for (module, is_enabled) in &categorized[category] {
                let short_name = module.name.split('/').next_back().unwrap();
                print!("  ");
                display_module_inline(
                    short_name,
                    &module.description,
                    module.package_count,
                    *is_enabled,
                );
            }
            println!();
        }
    }

    Ok(())
}

fn display_module(module: &crate::module::ModuleInfo, is_enabled: bool) {
    let status = if is_enabled {
        "enabled".green()
    } else {
        "disabled".yellow()
    };

    println!("  {} [{}]", module.name.blue(), status);
    if !module.description.is_empty() {
        println!("    {}", module.description);
    }
    println!("    Packages: {}", module.package_count);

    if !module.conflicts.is_empty() {
        println!(
            "    {}: {}",
            "Conflicts with".red(),
            module.conflicts.join(", ")
        );
    }

    println!();
}

fn display_module_inline(name: &str, description: &str, pkg_count: usize, is_enabled: bool) {
    let status = if is_enabled {
        "enabled".green()
    } else {
        "disabled".yellow()
    };

    println!("{} [{}]", name.blue(), status);
    if !description.is_empty() {
        println!("      {}", description);
    }
    println!("      Packages: {}", pkg_count);
    println!();
}

pub fn enable(
    paths: &ConfigPaths,
    module_names: &[String],
    json: bool,
    skip_sync: bool,
) -> Result<()> {
    let is_batch = module_names.len() > 1;
    let mut results = Vec::new();
    let mut changed_count = 0usize;

    if !json && is_batch {
        println!();
        println!(
            "{} Enabling {} module(s)...",
            "→".blue(),
            module_names.len()
        );
        println!();
    }

    for module_name in module_names {
        if !json && is_batch {
            println!("{} Enabling: {}", "→".blue(), module_name.green());
        }

        match enable_internal(paths, module_name, json) {
            Ok(outcome) => {
                if outcome.changed {
                    changed_count += 1;
                }
                results.push(outcome.output);
            }
            Err(err) => {
                if !json {
                    eprintln!("{} Failed to enable: {} - {}", "✗".red(), module_name, err);
                }
                results.push(ModuleActionOutput {
                    success: false,
                    message: err.to_string(),
                    module: module_name.clone(),
                    action: "enable".to_string(),
                });
            }
        }
    }

    if json {
        if is_batch {
            println!(
                "{}",
                serde_json::to_string_pretty(&ModuleBatchActionOutput { results })?
            );
        } else if let Some(result) = results.into_iter().next() {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        return Ok(());
    }

    if is_batch {
        let success_count = results.iter().filter(|r| r.success).count();
        let failed_count = results.len().saturating_sub(success_count);

        println!();
        if success_count > 0 {
            println!(
                "{} Successfully enabled {} module(s)",
                "✓".green(),
                success_count
            );
        }
        if failed_count > 0 {
            println!(
                "{} {} module(s) failed to enable",
                "✗".yellow(),
                failed_count
            );
        }
    }

    if changed_count > 0 && !skip_sync {
        prompt_for_sync(paths)?;
    }

    Ok(())
}

fn enable_internal(paths: &ConfigPaths, module_name: &str, json: bool) -> Result<EnableOutcome> {
    let spinner = if !json {
        Some(crate::progress::create_spinner("Checking module..."))
    } else {
        None
    };

    let mut config = load_config(paths)?;
    let module_manager = ModuleManager::new(paths.clone());

    // Resolve module path
    let resolved_name = module_manager.resolve_module_path(module_name)?;

    // Check if already enabled
    if config.enabled_modules.contains(&resolved_name) {
        let message = format!("Module '{}' is already enabled", resolved_name);
        if !json {
            if let Some(s) = spinner {
                s.finish_with_message(message.clone());
            }
        }
        return Ok(EnableOutcome {
            output: ModuleActionOutput {
                success: false,
                message,
                module: resolved_name,
                action: "enable".to_string(),
            },
            changed: false,
        });
    }

    // Check for conflicts
    let conflicts = module_manager.check_conflicts(&resolved_name, &config.enabled_modules)?;

    // Finish spinner before any prompts
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    if !conflicts.is_empty() {
        if json {
            return Ok(EnableOutcome {
                output: ModuleActionOutput {
                    success: false,
                    message: format!(
                        "Module '{}' conflicts with enabled module(s): {}",
                        resolved_name,
                        conflicts.join(", ")
                    ),
                    module: resolved_name,
                    action: "enable".to_string(),
                },
                changed: false,
            });
        } else {
            println!(
                "{}",
                format!(
                    "Module '{}' conflicts with enabled module(s): {}",
                    resolved_name,
                    conflicts.join(", ")
                )
                .red()
            );

            print!("Disable conflicting module(s)? [y/N] ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if input.trim().to_lowercase() == "y" {
                for conflict in conflicts {
                    config.enabled_modules.retain(|m| m != &conflict);
                    println!("{}", format!("Disabled module '{}'", conflict).yellow());
                }
            } else {
                println!("{}", "Cancelled".yellow());
                return Ok(EnableOutcome {
                    output: ModuleActionOutput {
                        success: false,
                        message: format!("Cancelled enabling module '{}'", resolved_name),
                        module: resolved_name,
                        action: "enable".to_string(),
                    },
                    changed: false,
                });
            }
        }
    }

    // Enable module
    config.enabled_modules.push(resolved_name.clone());

    // Save config
    save_config(paths, &config)?;

    let output = ModuleActionOutput {
        success: true,
        message: format!("Enabled module '{}'", resolved_name),
        module: resolved_name,
        action: "enable".to_string(),
    };

    if !json {
        println!("{}", format!("✓ {}", output.message).green());
    }

    Ok(EnableOutcome {
        output,
        changed: true,
    })
}

fn prompt_for_sync(paths: &ConfigPaths) -> Result<()> {
    println!();

    print!("Run sync now to install packages? [Y/n] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    if input.is_empty() || input == "y" || input == "yes" {
        println!();
        crate::commands::sync::run(
            paths,
            crate::commands::sync::SyncOptions {
                dry_run: false,
                prune: false,
                force: false,
                no_backup: false,
                no_hooks: false,
                force_dotfiles: false,
                json: false,
                auto_commit: false,
            },
        )?;
    } else {
        println!("Skipped sync. Run 'mdots sync' later to install packages.");
    }

    Ok(())
}

pub fn disable(paths: &ConfigPaths, module_name: &str, json: bool) -> Result<()> {
    let mut config = load_config(paths)?;
    let module_manager = ModuleManager::new(paths.clone());

    // Resolve module path
    let resolved_name = module_manager.resolve_module_path(module_name)?;

    // Check if enabled
    if !config.enabled_modules.contains(&resolved_name) {
        let message = format!("Module '{}' is not enabled", resolved_name);
        if json {
            let output = ModuleActionOutput {
                success: false,
                message,
                module: resolved_name,
                action: "disable".to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", message.yellow());
        }
        return Ok(());
    }

    // Disable module
    config.enabled_modules.retain(|m| m != &resolved_name);

    // Save config
    save_config(paths, &config)?;

    // Execute post-disable hook if present
    if !json {
        let modules_dir = paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", resolved_name));
        let module_lua = modules_dir.join(format!("{}.lua", resolved_name));
        let module_dir = modules_dir.join(&resolved_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            Some(module_dir)
        } else if module_file.exists() {
            Some(module_file)
        } else if module_lua.exists() {
            Some(module_lua)
        } else {
            // Module not found, skip hook execution
            None
        };

        if let Some(path) = module_path {
            if let Ok(module) = load_module(&path) {
                if let Some(hook_script) = module.post_disable_hook() {
                    if !hook_script.is_empty() {
                        let hook_path = if std::path::Path::new(hook_script).is_absolute() {
                            std::path::PathBuf::from(hook_script)
                        } else if module.is_directory() {
                            module.root_dir().join(hook_script)
                        } else {
                            paths.config_dir.join(hook_script)
                        };

                        if hook_path.exists() {
                            println!();
                            println!(
                                "{}",
                                format!("Running post-disable hook for '{}'...", resolved_name)
                                    .blue()
                            );
                            println!("Script: {}", hook_path.display());

                            let mut hook = std::process::Command::new("sudo");
                            hook.args(["bash", hook_path.to_str().unwrap()]);
                            let status = crate::process::status_inherited(&mut hook);

                            match status {
                                Ok(s) if s.success() => {
                                    println!(
                                        "{}",
                                        "✓ Post-disable hook executed successfully".green()
                                    );
                                }
                                _ => {
                                    eprintln!(
                                        "{} Post-disable hook failed - packages already removed",
                                        "✗".red()
                                    );
                                    eprintln!(
                                        "{} Run the script manually to complete cleanup:",
                                        "→".blue()
                                    );
                                    eprintln!("   sudo bash {}", hook_path.display());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if json {
        let output = ModuleActionOutput {
            success: true,
            message: format!("Disabled module '{}'", resolved_name),
            module: resolved_name,
            action: "disable".to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "{}",
            format!("✓ Disabled module '{}'", resolved_name).green()
        );
        println!("Run 'mdots sync --prune' to remove packages");
    }

    Ok(())
}

pub fn enable_interactive(paths: &ConfigPaths, skip_sync: bool) -> Result<()> {
    use std::process::{Command, Stdio};

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!(
            "fzf is not installed. Please install fzf to use interactive module selection."
        );
    }

    let config = load_config(paths)?;
    let module_manager = ModuleManager::new(paths.clone());
    let all_modules = module_manager.list_modules()?;

    // Filter to disabled modules
    let disabled: Vec<_> = all_modules
        .iter()
        .filter(|m| !config.enabled_modules.contains(&m.name))
        .collect();

    if disabled.is_empty() {
        println!("{}", "All modules are already enabled".yellow());
        return Ok(());
    }

    // Build preview command that shows the module manifest (YAML or Lua)
    let modules_dir = paths.modules_dir();
    let preview_cmd = format!(
        r#"[ -f '{dir}/{{1}}.yaml' ] && cat '{dir}/{{1}}.yaml' || \
[ -f '{dir}/{{1}}.lua' ] && cat '{dir}/{{1}}.lua' || \
[ -f '{dir}/{{1}}/module.yaml' ] && cat '{dir}/{{1}}/module.yaml' || \
[ -f '{dir}/{{1}}/module.lua' ] && cat '{dir}/{{1}}/module.lua' || \
echo 'Module file not found'"#,
        dir = modules_dir.display(),
    );

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--multi",
            "--preview",
            &preview_cmd,
            "--preview-window=right:40%:wrap",
            "--header=→ Select modules to enable\nℹ Use TAB to select multiple modules, ENTER to confirm",
            "--prompt=Select modules > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= mdots module enable ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run fzf")?;

    // Write module list to fzf
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for module in &disabled {
            writeln!(stdin, "{}", module.name)?;
        }
    }

    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        println!("{} Selection cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No modules selected", "✗".yellow());
        return Ok(());
    }

    let module_names: Vec<String> = selected.lines().map(ToOwned::to_owned).collect();
    enable(paths, &module_names, false, skip_sync)
}

pub fn disable_interactive(paths: &ConfigPaths) -> Result<()> {
    use std::process::{Command, Stdio};

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!(
            "fzf is not installed. Please install fzf to use interactive module selection."
        );
    }

    let config = load_config(paths)?;

    if config.enabled_modules.is_empty() {
        println!("{}", "No modules are currently enabled".yellow());
        return Ok(());
    }

    // Build preview command that shows the module manifest (YAML or Lua)
    let modules_dir = paths.modules_dir();
    let preview_cmd = format!(
        r#"[ -f '{dir}/{{1}}.yaml' ] && cat '{dir}/{{1}}.yaml' || \
[ -f '{dir}/{{1}}.lua' ] && cat '{dir}/{{1}}.lua' || \
[ -f '{dir}/{{1}}/module.yaml' ] && cat '{dir}/{{1}}/module.yaml' || \
[ -f '{dir}/{{1}}/module.lua' ] && cat '{dir}/{{1}}/module.lua' || \
echo 'Module file not found'"#,
        dir = modules_dir.display(),
    );

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--multi",
            "--preview",
            &preview_cmd,
            "--preview-window=right:40%:wrap",
            "--header=→ Select modules to disable\nℹ Use TAB to select multiple modules, ENTER to confirm",
            "--prompt=Select modules > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= mdots module disable ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run fzf")?;

    // Write enabled modules to fzf
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for module_name in &config.enabled_modules {
            writeln!(stdin, "{}", module_name)?;
        }
    }

    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        println!("{} Selection cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No modules selected", "✗".yellow());
        return Ok(());
    }

    let module_names: Vec<&str> = selected.lines().collect();

    println!();
    println!(
        "{} Disabling {} module(s)...",
        "→".blue(),
        module_names.len()
    );
    println!();

    let mut disabled_count = 0;
    let mut failed_count = 0;

    for module_name in &module_names {
        println!("{} Disabling: {}", "→".blue(), module_name.yellow());

        if let Err(e) = disable(paths, module_name, false) {
            failed_count += 1;
            eprintln!("{} Failed to disable: {} - {}", "✗".red(), module_name, e);
        } else {
            disabled_count += 1;
        }
    }

    println!();
    if disabled_count > 0 {
        println!(
            "{} Successfully disabled {} module(s)",
            "✓".green(),
            disabled_count
        );
        println!("Run 'mdots sync --prune' to remove packages");
    }
    if failed_count > 0 {
        println!(
            "{} {} module(s) failed to disable",
            "✗".yellow(),
            failed_count
        );
    }

    Ok(())
}

/// Check if a file path is a Lua config file
fn is_lua_config(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("lua")
}

fn save_config(paths: &ConfigPaths, config: &crate::config::Config) -> Result<()> {
    // Determine the correct file to save to
    // If config.yaml is a pointer (minimal), save to host file
    // Otherwise, save to config.yaml (legacy mode)
    let save_path = if let Ok(pointer_content) = std::fs::read_to_string(&paths.config_file) {
        if let Ok(pointer_config) = serde_yaml::from_str::<crate::config::Config>(&pointer_content)
        {
            // Check if it's a pointer config
            #[allow(deprecated)]
            let is_pointer = pointer_config.enabled_modules.is_empty()
                && pointer_config.packages.is_empty()
                && pointer_config.exclude.is_empty()
                && pointer_config.additional_packages.is_empty()
                && pointer_config.backup_tool.is_none()
                && pointer_config.description.is_empty()
                && pointer_config.import.is_empty();

            if is_pointer {
                // It's a pointer, save to host file
                paths.host_packages_file(&config.host)
            } else {
                // It's a full config, save to config.yaml
                paths.config_file.clone()
            }
        } else {
            // Can't parse, default to config.yaml
            paths.config_file.clone()
        }
    } else {
        // Can't read, default to config.yaml
        paths.config_file.clone()
    };

    // Check if target is a Lua file - we cannot auto-modify Lua configs
    if is_lua_config(&save_path) {
        println!(
            "{}",
            format!(
                "⚠️  Warning: Cannot automatically save changes to Lua file: {}\n\
                 Lua configs contain code and cannot be auto-modified.\n\
                 The module has been enabled/disabled in memory only.\n\
                 Please edit the file manually to persist this change.",
                save_path.display()
            )
            .yellow()
        );
        // Return Ok to allow the operation to continue, even though we can't persist
        return Ok(());
    }

    let yaml = serde_yaml::to_string(config).context("Failed to serialize config")?;
    std::fs::write(&save_path, yaml)
        .context(format!("Failed to write config file: {:?}", save_path))?;
    Ok(())
}

pub fn run_hook(paths: &ConfigPaths, module_name: &str) -> Result<()> {
    use crate::config::load_module;
    use std::process::Command;

    let config = load_config(paths)?;
    let module_manager = ModuleManager::new(paths.clone());

    // Resolve module path
    let resolved_name = module_manager.resolve_module_path(module_name)?;

    // Check if module is enabled
    if !config.enabled_modules.contains(&resolved_name) {
        println!(
            "{}",
            format!("Module '{}' is not enabled", resolved_name).yellow()
        );
        return Ok(());
    }

    // Find and load the module
    let modules_dir = paths.modules_dir();
    let module_file = modules_dir.join(format!("{}.yaml", resolved_name));
    let module_lua = modules_dir.join(format!("{}.lua", resolved_name));
    let module_dir = modules_dir.join(&resolved_name);

    let module_path = if module_dir.exists()
        && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
    {
        module_dir
    } else if module_file.exists() {
        module_file
    } else if module_lua.exists() {
        module_lua
    } else {
        anyhow::bail!("Module '{}' not found", resolved_name);
    };

    let module = load_module(&module_path)?;

    // Get the hook script
    let hook_script = match module.post_install_hook() {
        Some(script) if !script.is_empty() => script,
        _ => {
            println!(
                "{}",
                format!("Module '{}' has no post-install hook", resolved_name).yellow()
            );
            return Ok(());
        }
    };

    // Resolve hook path
    let hook_path = module.root_dir().join(hook_script);

    if !hook_path.exists() {
        anyhow::bail!("Hook script not found: {}", hook_path.display());
    }

    println!(
        "{}",
        format!("Running post-install hook for '{}'...", resolved_name).blue()
    );
    println!("Script: {}", hook_path.display());
    println!();

    // Execute the hook
    let mut hook = Command::new("sudo");
    hook.args(["bash", hook_path.to_str().unwrap()]);
    let status =
        crate::process::status_inherited(&mut hook).context("Failed to execute hook script")?;

    if status.success() {
        println!();
        println!("{}", "✓ Hook executed successfully".green());
    } else {
        anyhow::bail!("Hook script failed with exit code: {:?}", status.code());
    }

    Ok(())
}

pub fn run_hook_interactive(paths: &ConfigPaths) -> Result<()> {
    use crate::config::load_module;
    use std::process::{Command, Stdio};

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!(
            "fzf is not installed. Please install fzf to use interactive hook selection."
        );
    }

    let config = load_config(paths)?;

    if config.enabled_modules.is_empty() {
        println!("{}", "No modules are currently enabled".yellow());
        return Ok(());
    }

    // Find modules with hooks
    let modules_dir = paths.modules_dir();
    let mut modules_with_hooks = Vec::new();

    for module_name in &config.enabled_modules {
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

        match load_module(&module_path) {
            Ok(module) => {
                if let Some(hook) = module.post_install_hook() {
                    if !hook.is_empty() {
                        modules_with_hooks.push((module_name.clone(), hook.to_string()));
                    }
                }
            }
            Err(_) => continue,
        }
    }

    if modules_with_hooks.is_empty() {
        println!("{}", "No enabled modules have post-install hooks".yellow());
        return Ok(());
    }

    // Build a simple preview that just shows module and script info
    // Create a temp file with module:script mappings
    use tempfile::NamedTempFile;
    let mut temp_file = NamedTempFile::new().context("Failed to create temp file")?;
    for (name, script) in &modules_with_hooks {
        writeln!(temp_file, "{}:{}", name, script)?;
    }
    temp_file.flush()?;

    let preview_cmd = format!(
        r#"grep '^{{1}}:' {} | cut -d: -f2- | sed 's/^/Hook script: /'"#,
        temp_file.path().display()
    );

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--preview",
            &preview_cmd,
            "--preview-window=right:40%:wrap",
            "--header=→ Select module to run hook\nℹ Use arrow keys to select, ENTER to confirm",
            "--prompt=Select module > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= mdots hooks run ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
            "--no-multi",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run fzf")?;

    // Write module names to fzf
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for (module_name, _) in &modules_with_hooks {
            writeln!(stdin, "{}", module_name)?;
        }
    }

    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        println!("{} Selection cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No module selected", "✗".yellow());
        return Ok(());
    }

    println!();
    run_hook(paths, &selected)?;

    Ok(())
}

/// Create a new module with template content
pub fn create(
    paths: &ConfigPaths,
    module_path: &str,
    force_lua: bool,
    force_nix: bool,
) -> Result<()> {
    use crate::config::{is_lua_config, is_nix_config, load_config, resolve_editor};
    use std::fs;
    use std::io::{self, Write};

    if module_path.contains(' ') {
        anyhow::bail!("Module path cannot contain spaces: '{}'", module_path);
    }

    let format = if force_nix {
        "nix"
    } else if force_lua {
        "lua"
    } else if is_nix_config(paths).unwrap_or(false) {
        "nix"
    } else if is_lua_config(paths).unwrap_or(false) {
        "lua"
    } else {
        "yaml"
    };

    let extension = format;

    let modules_dir = paths.modules_dir();
    let module_file_path = if module_path.ends_with(".lua")
        || module_path.ends_with(".yaml")
        || module_path.ends_with(".nix")
    {
        modules_dir.join(module_path)
    } else {
        modules_dir.join(format!("{}.{}", module_path, extension))
    };

    if module_file_path.exists() {
        anyhow::bail!(
            "Module already exists: {}\nUse a different name or remove the existing module first.",
            module_file_path.display()
        );
    }

    println!("{}", "=== Create New Module ===".blue().bold());
    println!();
    println!("Module path: {}", module_file_path.display());
    let format_name = match format {
        "nix" => "Nix",
        "lua" => "Lua",
        _ => "YAML",
    };
    println!("Format: {}", format_name);
    println!();
    print!("Create this module? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("{}", "Cancelled".yellow());
        return Ok(());
    }

    if let Some(parent) = module_file_path.parent() {
        fs::create_dir_all(parent).context("Failed to create module directories")?;
    }

    let template = match format {
        "nix" => generate_nix_template(),
        "lua" => generate_lua_template(),
        _ => generate_yaml_template(),
    };

    fs::write(&module_file_path, template).context(format!(
        "Failed to write module file: {:?}",
        module_file_path
    ))?;

    println!();
    println!(
        "{} Created module: {}",
        "✓".green(),
        module_file_path.display()
    );

    println!();
    print!("Would you like to edit this module now? [Y/n] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    if input.is_empty() || input == "y" || input == "yes" {
        let config = load_config(paths)?;
        let editor = resolve_editor(&config)?;
        open_file_in_editor(&module_file_path, &editor)?;
    }

    println!();
    println!(
        "{}",
        "Tip: Run 'mdots module enable <module>' to enable this module.".yellow()
    );

    Ok(())
}

/// Generate YAML module template
fn generate_yaml_template() -> String {
    r#"# Module configuration
# See mdots documentation for full syntax reference

description: "Brief description of what this module provides"

packages:
  - package1
  - package2

# Optional: Conflicting modules
# conflicts:
#   - other-module

# Optional: Hook scripts
# pre_install_hook: scripts/pre-install.sh
# post_install_hook: scripts/post-install.sh
# post_disable_hook: scripts/cleanup.sh

# Optional: Hook behavior (ask, always, once, skip, never)
# hook_behavior: ask
"#
    .to_string()
}

/// Generate Lua module template
fn generate_nix_template() -> String {
    r#"{ system, pkgs }:

{
  description = "Brief description of what this module provides";

  packages = [
    "package1"
    "package2"
  ];

  # Optional: Flatpak packages
  # flatpak_packages = [
  #   "com.spotify.Client"
  # ];

  # Optional: Nix packages (requires home-manager)
  # nix_packages = with pkgs; [
  #   ripgrep
  #   fd
  # ];

  # Optional: System services to enable/disable
  # services = {
  #   enabled = [ "example.service" ];
  #   disabled = [];
  #   scope = "system";
  # };

  # Optional: Conflicting modules
  # conflicts = [ "other-module" ];

  # Optional: Hook scripts
  # pre_install_hook = "scripts/pre-install.sh";
  # post_install_hook = "scripts/post-install.sh";

  # Optional: Hook behavior (ask, always, once, skip)
  # hook_behavior = "ask";
}
"#
    .to_string()
}

fn generate_lua_template() -> String {
    r#"-- Module configuration
-- See mdots documentation for full API reference

local packages = {
    "package1",
    "package2",
}

return {
    description = "Brief description of what this module provides",
    packages = packages,

    -- Optional: Conflicting modules
    -- conflicts = {"other-module"},

    -- Optional: Hook scripts
    -- pre_install_hook = "scripts/pre-install.sh",
    -- post_install_hook = "scripts/post-install.sh",
    -- post_disable_hook = "scripts/cleanup.sh",

    -- Optional: Hook behavior (ask, always, once, skip, never)
    -- hook_behavior = "ask",
}
"#
    .to_string()
}

/// Open a file in the configured editor
fn open_file_in_editor(file_path: &std::path::Path, editor: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    println!();
    println!("{} Opening: {}", "→".blue(), file_path.display());
    println!("{} Editor: {}", "→".blue(), editor);
    println!();

    // Parse editor command (handle args like "code --wait")
    let editor_parts: Vec<&str> = editor.split_whitespace().collect();
    let (editor_cmd, editor_args) = editor_parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Invalid editor command"))?;

    // Check if editor exists in PATH
    if which::which(editor_cmd).is_err() {
        anyhow::bail!(
            "Editor '{}' not found in PATH. Please check your editor configuration.",
            editor_cmd
        );
    }

    // Spawn editor with full terminal control
    let status = Command::new(editor_cmd)
        .args(editor_args)
        .arg(file_path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context(format!("Failed to execute editor: {}", editor_cmd))?;

    if !status.success() {
        anyhow::bail!(
            "Editor exited with non-zero status: {:?}",
            status.code().unwrap_or(-1)
        );
    }

    println!();
    println!("{} File closed", "✓".green());

    Ok(())
}
