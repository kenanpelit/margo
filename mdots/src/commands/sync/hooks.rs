//! Module pre/post-install hooks and the hooks state that decides whether a
//! "once" hook runs again.

use anyhow::{Context, Result, anyhow};
use colored::*;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::config::ConfigPaths;

use super::*;

/// Execute a module hook script with error handling
pub(super) fn execute_module_hook(
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

    let hook_path_str = hook_path.to_str().context("hook path is not valid UTF-8")?;

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
                    hook_path_str,
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
                .args(["--preserve-env=MDOTS_LOG_INDENT", "bash", hook_path_str])
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

pub(super) fn run_pre_install_hooks(
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

        let hook_path_str = hook_path.to_str().context("hook path is not valid UTF-8")?;
        execute_command("sudo", &["bash", hook_path_str], json, false)?;

        mark_hook_executed(paths, &format!("pre_{}", module_name), &hook_path)?;

        if !json {
            println!("{}", "✓ Hook completed successfully".green());
        }
    }

    Ok(())
}

pub(super) fn run_post_install_hooks(
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

        let hook_path_str = hook_path.to_str().context("hook path is not valid UTF-8")?;
        match execute_command("sudo", &["bash", hook_path_str], json, false) {
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

pub enum HookStatus {
    NotRun,
    Executed,
    Skipped,
    Modified, // Hook exists but script has been modified
}

/// Identity of a hook script's content, recorded in the hooks state so a
/// "once" hook is re-run only when its script changes. SHA-256, so the value is
/// stable across Rust releases (unlike `DefaultHasher`, whose algorithm is
/// explicitly unspecified).
pub(crate) fn hook_script_hash(script: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(script.as_bytes());
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The identity older mdots versions recorded: `DefaultHasher` over the
/// script `String`, as a decimal `u64`. Only used to recognise existing
/// entries so they can be migrated to [`hook_script_hash`] without re-running
/// hooks that already ran.
pub(super) fn legacy_hook_script_hash(script: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    // `String` hashes exactly like `str`, which is what the old code fed in.
    script.hash(&mut hasher);
    hasher.finish().to_string()
}

pub(super) fn load_hooks_yaml(paths: &ConfigPaths) -> Result<serde_yaml::Value> {
    if paths.hooks_state_file.exists() {
        let content = std::fs::read_to_string(&paths.hooks_state_file)?;
        Ok(serde_yaml::from_str(&content)?)
    } else {
        Ok(serde_yaml::from_str("hooks: []")?)
    }
}

/// Write the hooks state atomically: it decides whether `sudo bash` hooks
/// re-run, so a torn file must never replace a good one.
pub(super) fn save_hooks_yaml(paths: &ConfigPaths, yaml: &serde_yaml::Value) -> Result<()> {
    let content = serde_yaml::to_string(yaml)?;
    crate::atomic_write::write_atomic(&paths.hooks_state_file, &content)
}

pub fn check_hook_status(
    paths: &ConfigPaths,
    module: &str,
    hook_path: &std::path::PathBuf,
) -> Result<HookStatus> {
    if !paths.hooks_state_file.exists() {
        return Ok(HookStatus::NotRun);
    }

    let mut yaml = load_hooks_yaml(paths)?;

    // Get current hash of script
    let script_content = std::fs::read_to_string(hook_path)?;
    let current_hash = hook_script_hash(&script_content);

    // Check stored hash and status
    let mut migrate = false;
    let mut result = HookStatus::NotRun;
    if let Some(hooks) = yaml.get_mut("hooks").and_then(|v| v.as_sequence_mut()) {
        for hook in hooks.iter_mut() {
            let Some(hook_map) = hook.as_mapping_mut() else {
                continue;
            };
            let stored_module = hook_map
                .get("module")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if stored_module != module {
                continue;
            }

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

            // If executed, check if script has been modified. An entry written
            // by an older mdots carries the legacy `DefaultHasher` value:
            // accept it if it matches this script and rewrite it as SHA-256.
            result = if stored_hash == current_hash {
                HookStatus::Executed
            } else if !stored_hash.starts_with("sha256:")
                && stored_hash == legacy_hook_script_hash(&script_content)
            {
                hook_map.insert(
                    serde_yaml::Value::from("script_hash"),
                    serde_yaml::Value::from(current_hash.clone()),
                );
                migrate = true;
                HookStatus::Executed
            } else {
                HookStatus::Modified
            };
            break;
        }
    }

    if migrate {
        save_hooks_yaml(paths, &yaml)?;
    }
    Ok(result)
}

pub fn mark_hook_skipped(paths: &ConfigPaths, module: &str) -> Result<()> {
    // Initialize or load hooks state file
    let mut yaml = load_hooks_yaml(paths)?;

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

    save_hooks_yaml(paths, &yaml)
}

pub fn mark_hook_executed(
    paths: &ConfigPaths,
    module: &str,
    hook_path: &std::path::PathBuf,
) -> Result<()> {
    use chrono::Utc;

    // Get hash of script
    let script_content = std::fs::read_to_string(hook_path)?;
    let script_hash = hook_script_hash(&script_content);

    let timestamp = Utc::now().to_rfc3339();

    // Initialize or load hooks state file
    let mut yaml = load_hooks_yaml(paths)?;

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

    save_hooks_yaml(paths, &yaml)
}
