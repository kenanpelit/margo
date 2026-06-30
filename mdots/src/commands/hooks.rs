use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, Write};

use crate::config::{load_config, load_module, ConfigPaths};

#[derive(Serialize)]
struct HooksListOutput {
    hooks: Vec<HookStatus>,
}

#[derive(Serialize)]
struct HookStatus {
    module: String,
    hook_type: String, // "pre-install" or "post-install"
    script: Option<String>,
    status: String, // "executed", "skipped", "not_run"
    #[serde(skip_serializing_if = "Option::is_none")]
    executed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    script_hash: Option<String>,
}

/// List all hooks and their execution status
pub fn list(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let mut hooks = Vec::new();

    // Load hooks state
    let hooks_state = load_hooks_state(paths)?;

    // Check all enabled modules
    for module_name in &config.enabled_modules {
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

        match load_module(&module_path) {
            Ok(module) => {
                // Check for pre-install hook
                if let Some(hook_script) = module.pre_install_hook() {
                    // Skip empty hook paths
                    if !hook_script.is_empty() {
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

                        // Only add if path exists and is not a directory
                        if hook_path.exists() && !hook_path.is_dir() {
                            let state_key = format!("pre_{}", module_name);
                            let status_info = hooks_state.get(&state_key);

                            hooks.push(HookStatus {
                                module: module_name.clone(),
                                hook_type: "pre-install".to_string(),
                                script: Some(hook_path.display().to_string()),
                                status: status_info
                                    .map(|s| s.status.clone())
                                    .unwrap_or_else(|| "not_run".to_string()),
                                executed_at: status_info.and_then(|s| s.executed_at.clone()),
                                script_hash: status_info.and_then(|s| s.script_hash.clone()),
                            });
                        }
                    }
                }

                // Check for post-install hook
                if let Some(hook_script) = module.post_install_hook() {
                    // Skip empty hook paths
                    if !hook_script.is_empty() {
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

                        // Only add if path exists and is not a directory
                        if hook_path.exists() && !hook_path.is_dir() {
                            let status_info = hooks_state.get(module_name);

                            hooks.push(HookStatus {
                                module: module_name.clone(),
                                hook_type: "post-install".to_string(),
                                script: Some(hook_path.display().to_string()),
                                status: status_info
                                    .map(|s| s.status.clone())
                                    .unwrap_or_else(|| "not_run".to_string()),
                                executed_at: status_info.and_then(|s| s.executed_at.clone()),
                                script_hash: status_info.and_then(|s| s.script_hash.clone()),
                            });
                        }
                    }
                }

                // Check for disable hook
                if let Some(hook_script) = module.post_disable_hook() {
                    // Skip empty hook paths
                    if !hook_script.is_empty() {
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

                        // Only add if path exists and is not a directory
                        if hook_path.exists() && !hook_path.is_dir() {
                            let state_key = format!("disable_{}", module_name);
                            let status_info = hooks_state.get(&state_key);

                            hooks.push(HookStatus {
                                module: module_name.clone(),
                                hook_type: "disable".to_string(),
                                script: Some(hook_path.display().to_string()),
                                status: status_info
                                    .map(|s| s.status.clone())
                                    .unwrap_or_else(|| "not_run".to_string()),
                                executed_at: status_info.and_then(|s| s.executed_at.clone()),
                                script_hash: status_info.and_then(|s| s.script_hash.clone()),
                            });
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to load module '{}': {}", module_name, e);
            }
        }
    }

    // Check for update hooks
    if let Some(pre_script) = &config.update_hooks.pre_update {
        let hook_path = if std::path::Path::new(pre_script).is_absolute() {
            std::path::PathBuf::from(pre_script)
        } else {
            paths.config_dir.join(pre_script)
        };

        if hook_path.exists() && !hook_path.is_dir() {
            let status_info = hooks_state.get("update_pre");
            hooks.push(HookStatus {
                module: "global".to_string(),
                hook_type: "pre-update".to_string(),
                script: Some(hook_path.display().to_string()),
                status: status_info
                    .map(|s| s.status.clone())
                    .unwrap_or_else(|| "not_run".to_string()),
                executed_at: status_info.and_then(|s| s.executed_at.clone()),
                script_hash: status_info.and_then(|s| s.script_hash.clone()),
            });
        }
    }

    if let Some(post_script) = &config.update_hooks.post_update {
        let hook_path = if std::path::Path::new(post_script).is_absolute() {
            std::path::PathBuf::from(post_script)
        } else {
            paths.config_dir.join(post_script)
        };

        if hook_path.exists() && !hook_path.is_dir() {
            let status_info = hooks_state.get("update_post");
            hooks.push(HookStatus {
                module: "global".to_string(),
                hook_type: "post-update".to_string(),
                script: Some(hook_path.display().to_string()),
                status: status_info
                    .map(|s| s.status.clone())
                    .unwrap_or_else(|| "not_run".to_string()),
                executed_at: status_info.and_then(|s| s.executed_at.clone()),
                script_hash: status_info.and_then(|s| s.script_hash.clone()),
            });
        }
    }

    if json {
        let output = HooksListOutput { hooks };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if hooks.is_empty() {
            println!("{}", "No hooks found in enabled modules".yellow());
        } else {
            println!("{}", "=== Hooks Status ===".blue().bold());
            println!();

            for hook in &hooks {
                let status_display = match hook.status.as_str() {
                    "executed" => "✓ Executed".green(),
                    "skipped" => "⊘ Skipped".yellow(),
                    "not_run" => "○ Not Run".white(),
                    _ => "? Unknown".red(),
                };

                let type_label = match hook.hook_type.as_str() {
                    "pre-install" => "Pre ".yellow(),
                    "post-install" => "Post".cyan(),
                    "disable" => "Dis ".red(),
                    "pre-update" => "PreU".magenta(),
                    "post-update" => "PstU".magenta(),
                    _ => "    ".white(),
                };

                println!("{} {} {}", status_display, type_label, hook.module.cyan());

                if let Some(script) = &hook.script {
                    println!("  Script: {}", script.dimmed());
                }

                if let Some(executed_at) = &hook.executed_at {
                    println!("  Last run: {}", executed_at.dimmed());
                }

                println!();
            }
        }
    }

    Ok(())
}

/// Reset a hook to "not run" state (will run on next sync)
pub fn reset(paths: &ConfigPaths, module: &str, pre: bool, disable: bool) -> Result<()> {
    let mut hooks_state = load_hooks_state(paths)?;

    // Determine state key and description based on flags
    let (state_key, hook_desc) = if module == "update_pre" || module == "update_post" {
        (
            module.to_string(),
            format!("{} hook", module.replace('_', "-")),
        )
    } else if pre {
        (
            format!("pre_{}", module),
            format!("pre-install hook for module '{}'", module),
        )
    } else if disable {
        (
            format!("disable_{}", module),
            format!("disable hook for module '{}'", module),
        )
    } else {
        (
            module.to_string(),
            format!("post-install hook for module '{}'", module),
        )
    };

    if !hooks_state.contains_key(&state_key) {
        println!(
            "{} {} is already in 'not run' state",
            "→".blue(),
            hook_desc.yellow()
        );
        return Ok(());
    }

    hooks_state.remove(&state_key);
    save_hooks_state(paths, &hooks_state)?;

    println!(
        "{} Reset {} - will run on next invocation",
        "✓".green(),
        hook_desc.yellow()
    );

    Ok(())
}

/// Skip a hook permanently (mark as "don't run")
pub fn skip(paths: &ConfigPaths, module: &str, pre: bool, disable: bool) -> Result<()> {
    let mut hooks_state = load_hooks_state(paths)?;

    // Handle update hooks specially
    if module == "update_pre" || module == "update_post" {
        let config = load_config(paths)?;

        // Verify the hook exists in config
        let hook_exists = if module == "update_pre" {
            config.update_hooks.pre_update.is_some()
        } else {
            config.update_hooks.post_update.is_some()
        };

        if !hook_exists {
            anyhow::bail!("{} hook is not configured", module.replace('_', "-"));
        }

        hooks_state.insert(
            module.to_string(),
            HookStateEntry {
                status: "skipped".to_string(),
                script_hash: None,
                executed_at: None,
            },
        );

        save_hooks_state(paths, &hooks_state)?;

        println!(
            "{} Marked {} hook as skipped - will not run during updates",
            "✓".green(),
            module.replace('_', "-").yellow()
        );

        return Ok(());
    }

    // Handle module hooks
    let config = load_config(paths)?;
    if !config.enabled_modules.contains(&module.to_string()) {
        anyhow::bail!("Module '{}' is not enabled", module);
    }

    // Verify module has a hook
    let modules_dir = paths.modules_dir();
    let module_file = modules_dir.join(format!("{}.yaml", module));
    let module_lua = modules_dir.join(format!("{}.lua", module));
    let module_dir = modules_dir.join(module);

    let module_path = if module_dir.exists()
        && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
    {
        module_dir
    } else if module_file.exists() {
        module_file
    } else if module_lua.exists() {
        module_lua
    } else {
        anyhow::bail!("Module '{}' not found", module);
    };

    let loaded_module = load_module(&module_path)?;

    // Determine which hook to skip based on the flags
    let (hook_exists, hook_type, state_key) = if pre {
        (
            loaded_module.pre_install_hook().is_some(),
            "pre-install",
            format!("pre_{}", module),
        )
    } else if disable {
        (
            loaded_module.post_disable_hook().is_some(),
            "disable",
            format!("disable_{}", module),
        )
    } else {
        (
            loaded_module.post_install_hook().is_some(),
            "post-install",
            module.to_string(),
        )
    };

    if !hook_exists {
        anyhow::bail!("Module '{}' does not have a {} hook", module, hook_type);
    }

    hooks_state.insert(
        state_key,
        HookStateEntry {
            status: "skipped".to_string(),
            script_hash: None,
            executed_at: None,
        },
    );

    save_hooks_state(paths, &hooks_state)?;

    println!(
        "{} Marked {} hook for module '{}' as skipped - will not run during sync",
        "✓".green(),
        hook_type.yellow(),
        module.yellow()
    );

    Ok(())
}

/// Manually run a module's hook
pub fn run(paths: &ConfigPaths, module: &str, pre: bool, disable: bool) -> Result<()> {
    let config = load_config(paths)?;

    if !config.enabled_modules.contains(&module.to_string()) {
        anyhow::bail!("Module '{}' is not enabled", module);
    }

    let modules_dir = paths.modules_dir();
    let module_file = modules_dir.join(format!("{}.yaml", module));
    let module_lua = modules_dir.join(format!("{}.lua", module));
    let module_dir = modules_dir.join(module);

    let module_path = if module_dir.exists()
        && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
    {
        module_dir
    } else if module_file.exists() {
        module_file
    } else if module_lua.exists() {
        module_lua
    } else {
        anyhow::bail!("Module '{}' not found", module);
    };

    let loaded_module = load_module(&module_path)?;

    // Determine which hook to run based on the flags
    let (hook_path, hook_type, state_key) = if pre {
        match loaded_module.pre_install_hook() {
            Some(path) => {
                let hook_path = if std::path::Path::new(path).is_absolute() {
                    std::path::PathBuf::from(path)
                } else if loaded_module.is_directory() {
                    loaded_module.root_dir().join(path)
                } else {
                    paths.config_dir.join(path)
                };
                (hook_path, "pre-install", format!("pre_{}", module))
            }
            None => {
                anyhow::bail!("Module '{}' does not have a pre-install hook", module);
            }
        }
    } else if disable {
        match loaded_module.post_disable_hook() {
            Some(path) => {
                let hook_path = if std::path::Path::new(path).is_absolute() {
                    std::path::PathBuf::from(path)
                } else if loaded_module.is_directory() {
                    loaded_module.root_dir().join(path)
                } else {
                    paths.config_dir.join(path)
                };
                (hook_path, "disable", format!("disable_{}", module))
            }
            None => {
                anyhow::bail!("Module '{}' does not have a disable hook", module);
            }
        }
    } else {
        match loaded_module.post_install_hook() {
            Some(path) => {
                let hook_path = if std::path::Path::new(path).is_absolute() {
                    std::path::PathBuf::from(path)
                } else if loaded_module.is_directory() {
                    loaded_module.root_dir().join(path)
                } else {
                    paths.config_dir.join(path)
                };
                (hook_path, "post-install", module.to_string())
            }
            None => {
                anyhow::bail!("Module '{}' does not have a post-install hook", module);
            }
        }
    };

    if !hook_path.exists() {
        anyhow::bail!("Hook script not found: {}", hook_path.display());
    }

    println!(
        "{} Running {} hook for module '{}'...",
        "→".blue(),
        hook_type.yellow(),
        module.cyan()
    );
    println!("  Script: {}", hook_path.display().to_string().dimmed());
    println!();

    // Ask for confirmation
    print!("Continue? [Y/n] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().is_empty() && !input.trim().eq_ignore_ascii_case("y") {
        println!("{}", "Cancelled".yellow());
        return Ok(());
    }

    // Execute the hook
    let status = std::process::Command::new("sudo")
        .arg("bash")
        .arg(&hook_path)
        .current_dir(&paths.config_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to execute hook script")?;

    if !status.success() {
        anyhow::bail!("Hook script failed with exit code: {:?}", status.code());
    }

    println!();
    println!(
        "{} {} hook executed successfully",
        "✓".green(),
        hook_type.yellow()
    );

    // Mark as executed (update state)
    crate::commands::sync::mark_hook_executed(paths, &state_key, &hook_path)?;

    Ok(())
}

// Helper types and functions

#[derive(Clone)]
struct HookStateEntry {
    status: String,
    script_hash: Option<String>,
    executed_at: Option<String>,
}

fn load_hooks_state(paths: &ConfigPaths) -> Result<HashMap<String, HookStateEntry>> {
    if !paths.hooks_state_file.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(&paths.hooks_state_file)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;

    let mut hooks_state = HashMap::new();

    if let Some(hooks) = yaml.get("hooks").and_then(|v| v.as_sequence()) {
        for hook in hooks {
            if let Some(hook_map) = hook.as_mapping() {
                let module = hook_map
                    .get("module")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let status = hook_map
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("executed")
                    .to_string();

                let script_hash = hook_map
                    .get("script_hash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let executed_at = hook_map
                    .get("executed_at")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if !module.is_empty() {
                    hooks_state.insert(
                        module,
                        HookStateEntry {
                            status,
                            script_hash,
                            executed_at,
                        },
                    );
                }
            }
        }
    }

    Ok(hooks_state)
}

fn save_hooks_state(
    paths: &ConfigPaths,
    hooks_state: &HashMap<String, HookStateEntry>,
) -> Result<()> {
    let mut hooks_vec = Vec::new();

    for (module, entry) in hooks_state {
        let mut hook_entry = serde_json::json!({
            "module": module,
            "status": entry.status,
        });

        if let Some(hash) = &entry.script_hash {
            hook_entry["script_hash"] = serde_json::Value::String(hash.clone());
        }

        if let Some(executed_at) = &entry.executed_at {
            hook_entry["executed_at"] = serde_json::Value::String(executed_at.clone());
        }

        hooks_vec.push(hook_entry);
    }

    let yaml_value = serde_json::json!({
        "hooks": hooks_vec
    });

    std::fs::create_dir_all(&paths.state_dir)?;
    let content = serde_yaml::to_string(&yaml_value)?;
    std::fs::write(&paths.hooks_state_file, content)?;

    Ok(())
}
