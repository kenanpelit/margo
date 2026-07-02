use anyhow::{Context, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use crate::config::{load_module, Config, ConfigPaths};

/// State file tracking which dotfiles have been backed up
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DotfilesState {
    /// List of backed up dotfile paths
    #[serde(default)]
    pub backed_up: Vec<BackedUpDotfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackedUpDotfile {
    /// The target path that was backed up (e.g., ~/.config/hypr)
    pub target: String,

    /// The backup path
    pub backup: String,

    /// Module that owns this dotfile
    pub module: String,

    /// Timestamp of backup
    pub backed_up_at: String,
}

/// Runtime structure for a resolved dotfile (source and target paths)
#[derive(Debug, Clone)]
struct ResolvedDotfile {
    /// Absolute path to source file/directory in the module
    source: PathBuf,

    /// Absolute path to target location in filesystem
    target: PathBuf,

    /// Name of the module that owns this dotfile
    module_name: String,
}

/// Tracks a conflict where multiple modules want to sync to the same target
#[derive(Debug)]
struct DotfileConflict {
    /// The target path that has conflicts
    target: PathBuf,

    /// List of (module_name, source_path) tuples that conflict
    modules: Vec<(String, PathBuf)>,
}

/// Statistics for sync operation
#[derive(Debug, Default)]
struct SyncStats {
    created: usize,
    updated: usize,
    skipped: usize,
    backups: usize,
}

/// Sync dotfiles from all enabled modules using three-phase approach
pub fn sync_dotfiles(paths: &ConfigPaths, config: &Config, force: bool, json: bool) -> Result<()> {
    if !json {
        crate::ui::step("Linking", "dotfiles");
    }

    // Phase 1: Collect all dotfiles from all enabled modules
    let all_dotfiles = collect_all_dotfiles(paths, config)?;

    // Phase 2: Detect conflicts (errors if found)
    detect_conflicts(&all_dotfiles)?;

    // Phase 3: Perform sync (existing backup/symlink logic)
    perform_sync(paths, &all_dotfiles, force, json)?;

    Ok(())
}

/// Perform the actual sync operation for all dotfiles
fn perform_sync(
    paths: &ConfigPaths,
    dotfiles: &[ResolvedDotfile],
    force: bool,
    json: bool,
) -> Result<()> {
    let home_dir = std::env::var("HOME").context("HOME environment variable not set")?;
    let config_dir = PathBuf::from(&home_dir).join(".config");

    // Ensure .config directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).context("Failed to create .config directory")?;
    }

    // Load dotfiles state
    let mut state = load_dotfiles_state(paths)?;
    let mut stats = SyncStats::default();

    // Process each dotfile
    for df in dotfiles {
        sync_single_dotfile(df, &mut state, &mut stats, force, json, paths)?;
    }

    // Save updated state
    save_dotfiles_state(paths, &state)?;

    // Print summary
    if !json {
        print_sync_summary(&stats);
    }

    Ok(())
}

/// Sync a single dotfile (create symlink with backup if needed)
fn sync_single_dotfile(
    df: &ResolvedDotfile,
    state: &mut DotfilesState,
    stats: &mut SyncStats,
    force: bool,
    json: bool,
    paths: &ConfigPaths,
) -> Result<()> {
    let source = &df.source;
    let target = &df.target;
    let module_name = &df.module_name;

    // Ensure parent directory exists
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .context(format!("Failed to create parent directory: {:?}", parent))?;
        }
    }

    // Get display name for output
    let display_name = target
        .file_name()
        .unwrap_or(target.as_os_str())
        .to_str()
        .unwrap_or("?");

    // Check if already backed up
    let already_backed_up = state
        .backed_up
        .iter()
        .any(|b| b.target == target.to_string_lossy() && b.module == *module_name);

    // Handle existing target (including broken symlinks)
    // Note: target.exists() returns false for broken symlinks, so we check is_symlink() first
    if target.is_symlink() || target.exists() {
        // Check if it's already a symlink pointing to our source
        if target.is_symlink() {
            if let Ok(link_target) = fs::read_link(target) {
                if link_target == *source && !force {
                    // Already correctly linked, skip (unless force is enabled)
                    stats.skipped += 1;
                    return Ok(());
                }
            }

            // Symlink exists but points elsewhere (or force is enabled), remove it
            fs::remove_file(target)
                .context(format!("Failed to remove old symlink: {:?}", target))?;
        } else {
            // Real file/directory exists — ALWAYS back it up before replacing it.
            //
            // A previous backup record (`already_backed_up`) may point at older,
            // different content from an earlier sync. It must NEVER justify deleting
            // the data currently sitting at the target (e.g. if the user replaced our
            // symlink with a fresh real file). Otherwise a re-sync silently destroys it.
            let backup_path = create_backup_path(target);

            if !json {
                println!(
                    "  {} Backing up existing {}: {} -> {}",
                    "↑".yellow(),
                    if target.is_dir() { "directory" } else { "file" },
                    display_name,
                    backup_path.file_name().unwrap().to_str().unwrap()
                );
            }

            fs::rename(target, &backup_path).context(format!(
                "Failed to backup {:?} to {:?}",
                target, backup_path
            ))?;

            // Record backup in state
            state.backed_up.push(BackedUpDotfile {
                target: target.to_string_lossy().to_string(),
                backup: backup_path.to_string_lossy().to_string(),
                module: module_name.clone(),
                backed_up_at: chrono::Utc::now().to_rfc3339(),
            });

            stats.backups += 1;
        }
    }

    // Create symlink
    unix_fs::symlink(source, target).context(format!(
        "Failed to create symlink: {:?} -> {:?}",
        target, source
    ))?;

    if !json {
        println!(
            "  {} {} -> {}",
            "→".green(),
            display_name,
            source
                .strip_prefix(&paths.config_dir)
                .unwrap_or(source)
                .display()
        );
    }

    if already_backed_up {
        stats.updated += 1;
    } else {
        stats.created += 1;
    }

    Ok(())
}

/// Print summary of sync operation
fn print_sync_summary(stats: &SyncStats) {
    let total_processed = stats.created + stats.updated + stats.skipped + stats.backups;

    if total_processed == 0 {
        crate::ui::detail("no dotfiles configured");
        return;
    }

    if stats.created > 0 {
        crate::ui::detail(&format!("{} created", stats.created));
    }
    if stats.updated > 0 {
        crate::ui::detail(&format!("{} updated", stats.updated));
    }
    if stats.backups > 0 {
        crate::ui::detail(&format!("{} backed up", stats.backups));
    }
    if stats.skipped > 0 {
        crate::ui::detail(&format!("{} already in sync", stats.skipped));
    }
}

/// Remove dotfile symlinks for modules that are no longer enabled
pub fn prune_dotfiles(paths: &ConfigPaths, config: &Config, json: bool) -> Result<()> {
    let _home_dir = std::env::var("HOME").context("HOME environment variable not set")?;

    // Load dotfiles state
    let mut state = load_dotfiles_state(paths)?;

    // Get set of currently enabled modules
    let enabled_modules: HashSet<&str> =
        config.enabled_modules.iter().map(|s| s.as_str()).collect();

    let mut pruned_count = 0;
    let mut restored_count = 0;
    let mut kept_backups: Vec<BackedUpDotfile> = Vec::new();

    // Check each backed up dotfile
    for backed_up in &state.backed_up {
        if enabled_modules.contains(backed_up.module.as_str()) {
            // Still enabled — leave its symlink and backup record untouched.
            kept_backups.push(backed_up.clone());
            continue;
        }

        // Module no longer enabled: remove our symlink AND restore the user's
        // original file, which `sync_single_dotfile` renamed aside to `backup`
        // before linking. The old behaviour dropped the backup record without
        // restoring — the original was left orphaned at the backup path and the
        // live location empty: silent data loss. Restore it, and only drop the
        // record once the original is safely back (or there is genuinely
        // nothing on disk to restore).
        let target = PathBuf::from(&backed_up.target);
        let backup = PathBuf::from(&backed_up.backup);

        // 1. Remove our symlink if it is still the thing at `target`
        //    (`is_symlink` also catches a dangling link whose source is gone).
        if target.is_symlink() {
            fs::remove_file(&target).context(format!("Failed to remove symlink: {:?}", target))?;

            if !json {
                println!(
                    "  {} Removed symlink: {}",
                    "✗".yellow(),
                    target.file_name().unwrap().to_str().unwrap()
                );
            }

            pruned_count += 1;
        }

        // 2. Restore the original from its backup. Guard hard against
        //    clobbering: only when the backup still exists and the target slot
        //    is now free. If the user has since placed a real file at `target`,
        //    never overwrite it — keep the backup and its record so nothing is
        //    silently orphaned.
        if backup.exists() {
            if target.exists() {
                // A real file/dir the user created now occupies the slot —
                // leave it, and keep the record so the backup stays tracked.
                if !json {
                    eprintln!(
                        "  {} {} exists; kept original backup at {}",
                        "!".yellow(),
                        backed_up.target,
                        backed_up.backup
                    );
                }
                kept_backups.push(backed_up.clone());
                continue;
            }
            match fs::rename(&backup, &target) {
                Ok(()) => {
                    if !json {
                        println!("  {} Restored original: {}", "↩".green(), backed_up.target);
                    }
                    restored_count += 1;
                }
                Err(e) => {
                    // Couldn't restore — KEEP the record so the backup stays
                    // tracked and a later prune can retry, rather than
                    // orphaning the user's original.
                    if !json {
                        eprintln!(
                            "  {} Failed to restore {} from {}: {}",
                            "✗".red(),
                            backed_up.target,
                            backed_up.backup,
                            e
                        );
                    }
                    kept_backups.push(backed_up.clone());
                    continue;
                }
            }
        }

        // Original restored, or nothing on disk to restore — safe to drop the
        // record by not pushing it into `kept_backups`.
    }

    // Update state
    state.backed_up = kept_backups;
    save_dotfiles_state(paths, &state)?;

    if !json {
        if pruned_count > 0 {
            println!(
                "  {} {} dotfile symlink{} pruned",
                "✓".green(),
                pruned_count,
                if pruned_count == 1 { "" } else { "s" }
            );
        }
        if restored_count > 0 {
            println!(
                "  {} {} original{} restored",
                "✓".green(),
                restored_count,
                if restored_count == 1 { "" } else { "s" }
            );
        }
    }

    Ok(())
}

/// Expand tilde (~) in paths to home directory
fn expand_tilde(path: &str, home_dir: &str) -> Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        Ok(PathBuf::from(home_dir).join(rest))
    } else if path == "~" {
        Ok(PathBuf::from(home_dir))
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Collect dotfiles in automatic mode (legacy behavior - scan dotfiles/ directory)
fn collect_automatic_dotfiles(
    module_root: &Path,
    module_name: &str,
) -> Result<Vec<ResolvedDotfile>> {
    let mut dotfiles = Vec::new();
    let dotfiles_dir = module_root.join("dotfiles");

    if !dotfiles_dir.exists() {
        return Ok(dotfiles);
    }

    let home_dir = std::env::var("HOME").context("HOME not set")?;
    let config_dir = PathBuf::from(&home_dir).join(".config");

    for entry in fs::read_dir(&dotfiles_dir).context(format!(
        "Failed to read dotfiles directory: {:?}",
        dotfiles_dir
    ))? {
        let entry = entry?;
        let path = entry.path();

        // Support both files and directories in automatic mode
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        dotfiles.push(ResolvedDotfile {
            source: path,
            target: config_dir.join(&name),
            module_name: module_name.to_string(),
        });
    }

    Ok(dotfiles)
}

/// Collect dotfiles from explicit dotfiles.yaml configuration
fn collect_explicit_dotfiles(
    module_root: &Path,
    module_name: &str,
    entries: &[crate::config::DotfileEntry],
) -> Result<Vec<ResolvedDotfile>> {
    let mut dotfiles = Vec::new();
    let home_dir = std::env::var("HOME").context("HOME not set")?;

    for entry in entries {
        // Direct access to source and target fields
        let source_rel = PathBuf::from(&entry.source);
        let target = expand_tilde(&entry.target, &home_dir)?;
        let source_abs = module_root.join(&source_rel);

        // Validate source exists
        if !source_abs.exists() {
            anyhow::bail!(
                "Dotfile source does not exist: {:?} (from module {})",
                source_abs,
                module_name
            );
        }

        dotfiles.push(ResolvedDotfile {
            source: source_abs,
            target,
            module_name: module_name.to_string(),
        });
    }

    Ok(dotfiles)
}

/// Collect all dotfiles from all enabled modules
fn collect_all_dotfiles(paths: &ConfigPaths, config: &Config) -> Result<Vec<ResolvedDotfile>> {
    use crate::config::ModuleStructure;
    use std::collections::HashSet;

    let mut all_dotfiles = Vec::new();
    let modules_dir = paths.modules_dir();

    for module_name in &config.enabled_modules {
        // Load the module
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            module_dir
        } else if module_file.exists() {
            // Legacy modules don't have dotfiles
            continue;
        } else {
            continue;
        };

        let module = match load_module(&module_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Only directory modules can have dotfiles
        if !module.is_directory() {
            continue;
        }

        let module_root = module.root_dir();

        // Process dotfiles configuration from module.yaml
        if let ModuleStructure::Directory(dir_module) = &module {
            // Collect automatic dotfiles if enabled
            let automatic_dotfiles = if dir_module.manifest.dotfiles_sync == Some(true) {
                collect_automatic_dotfiles(&module_root, module_name)?
            } else {
                Vec::new()
            };

            // Collect explicit dotfiles if defined
            let explicit_dotfiles = if !dir_module.manifest.dotfiles.is_empty() {
                collect_explicit_dotfiles(&module_root, module_name, &dir_module.manifest.dotfiles)?
            } else {
                Vec::new()
            };

            // HYBRID MODE: Explicit takes precedence over automatic
            // Build set of explicit targets for O(1) lookup
            let explicit_targets: HashSet<PathBuf> = explicit_dotfiles
                .iter()
                .map(|df| df.target.clone())
                .collect();

            // Add explicit dotfiles first
            all_dotfiles.extend(explicit_dotfiles);

            // Add automatic dotfiles only if target not already in explicit
            for auto_df in automatic_dotfiles {
                if !explicit_targets.contains(&auto_df.target) {
                    all_dotfiles.push(auto_df);
                }
            }
        }
    }

    Ok(all_dotfiles)
}

/// Detect conflicts where multiple modules want to sync to the same target
fn detect_conflicts(dotfiles: &[ResolvedDotfile]) -> Result<()> {
    use std::collections::HashMap;

    let mut target_map: HashMap<PathBuf, Vec<(String, PathBuf)>> = HashMap::new();

    // Build map of target → [(module, source), ...]
    for df in dotfiles {
        target_map
            .entry(df.target.clone())
            .or_default()
            .push((df.module_name.clone(), df.source.clone()));
    }

    // Find conflicts (target with multiple sources)
    let conflicts: Vec<DotfileConflict> = target_map
        .into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(target, modules)| DotfileConflict { target, modules })
        .collect();

    if !conflicts.is_empty() {
        // Format error message with colored output
        eprintln!("{}", "Dotfile conflicts detected:".red().bold());
        eprintln!();

        for conflict in &conflicts {
            eprintln!(
                "  {}: {}",
                "Target".bold(),
                conflict.target.display().to_string().red()
            );
            eprintln!("  {}:", "Conflicting modules".bold());
            for (module, source) in &conflict.modules {
                eprintln!("    - {} (source: {})", module.yellow(), source.display());
            }
            eprintln!();
        }

        eprintln!(
            "{}: Update dotfiles.yaml in conflicting modules to use different target paths.",
            "Resolution".green().bold()
        );

        anyhow::bail!("Dotfile conflicts must be resolved before syncing");
    }

    Ok(())
}

/// Create a backup path with timestamp
fn create_backup_path(original: &Path) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let parent = original.parent().unwrap_or(Path::new("."));
    let name = original.file_name().unwrap().to_str().unwrap();
    parent.join(format!("{}.backup.{}", name, timestamp))
}

/// Load dotfiles state from file
fn load_dotfiles_state(paths: &ConfigPaths) -> Result<DotfilesState> {
    let state_file = paths.state_dir.join("dotfiles-state.yaml");

    if !state_file.exists() {
        return Ok(DotfilesState::default());
    }

    let content = fs::read_to_string(&state_file).context("Failed to read dotfiles state file")?;

    let state: DotfilesState =
        serde_yaml::from_str(&content).context("Failed to parse dotfiles state file")?;

    Ok(state)
}

/// Save dotfiles state to file
fn save_dotfiles_state(paths: &ConfigPaths, state: &DotfilesState) -> Result<()> {
    // Ensure state directory exists
    fs::create_dir_all(&paths.state_dir).context("Failed to create state directory")?;

    let state_file = paths.state_dir.join("dotfiles-state.yaml");

    let yaml = serde_yaml::to_string(state).context("Failed to serialize dotfiles state")?;

    fs::write(&state_file, yaml).context("Failed to write dotfiles state file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal ConfigPaths rooted at `root` (only config_dir/state_dir matter here).
    fn make_paths(root: &Path) -> ConfigPaths {
        ConfigPaths {
            config_dir: root.to_path_buf(),
            config_file: root.join("config.yaml"),
            packages_dir: root.join("packages"),
            state_dir: root.join("state"),
            state_file: root.join("state/state.yaml"),
            hooks_state_file: root.join("state/hooks.yaml"),
            services_state_file: root.join("state/services.yaml"),
            defaults_state_file: root.join("state/defaults.yaml"),
            theming_state_file: root.join("state/theming.yaml"),
            config_backups_dir: root.join("config-backups"),
        }
    }

    /// Find a `<name>.backup.*` sibling of `target` and return its path, if any.
    fn find_backup(target: &Path) -> Option<PathBuf> {
        let parent = target.parent()?;
        let prefix = format!("{}.backup.", target.file_name()?.to_str()?);
        fs::read_dir(parent)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false)
            })
    }

    /// Regression test for the dotfiles data-loss bug:
    /// If a managed symlink is later replaced by a REAL directory holding new user data,
    /// a re-sync must NOT delete that data just because an OLD backup record exists.
    /// It must back up the current real content before replacing it with the symlink.
    #[test]
    fn test_resync_real_dir_after_symlink_replaced_preserves_data() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let paths = make_paths(root);

        // Module source content (what the symlink should point to)
        let source = root.join("module/hypr");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("source.conf"), "from module").unwrap();

        // Target: a REAL user directory containing important, current data
        let target = root.join("home/.config/hypr");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("important.conf"), "SECRET_DATA").unwrap();

        // State already has a (stale) backup record from a previous sync.
        let mut state = DotfilesState {
            backed_up: vec![BackedUpDotfile {
                target: target.to_string_lossy().to_string(),
                backup: root
                    .join("home/.config/hypr.backup.old")
                    .to_string_lossy()
                    .to_string(),
                module: "hypr".to_string(),
                backed_up_at: "2020-01-01T00:00:00Z".to_string(),
            }],
        };

        let df = ResolvedDotfile {
            source: source.clone(),
            target: target.clone(),
            module_name: "hypr".to_string(),
        };
        let mut stats = SyncStats::default();

        sync_single_dotfile(&df, &mut state, &mut stats, false, true, &paths).unwrap();

        // The target must now be our symlink...
        assert!(
            target.is_symlink(),
            "target should be replaced by a symlink"
        );

        // ...and the user's current data must NOT be lost: a fresh backup must hold it.
        let backup = find_backup(&target).expect("a fresh backup of the real directory must exist");
        let preserved = fs::read_to_string(backup.join("important.conf"))
            .expect("backed-up important.conf must exist");
        assert_eq!(
            preserved, "SECRET_DATA",
            "user data must be preserved in the backup"
        );
    }

    /// Regression test for the dotfiles data-loss bug in `prune_dotfiles`:
    /// disabling a module must RESTORE the user's original file (which sync
    /// renamed aside to a backup), not just delete the symlink and orphan the
    /// backup. The old code left the live location empty and dropped the
    /// record — from the user's view, the original config was gone.
    #[test]
    fn test_prune_restores_original_when_module_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let paths = make_paths(root);

        // The user's original file at the target location.
        let target = root.join("home/.config/hypr/hyprland.conf");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "USER_ORIGINAL").unwrap();

        // Module source the symlink will point at.
        let source = root.join("module/hyprland.conf");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "from module").unwrap();

        // Sync backs up the original and symlinks the target (module "hypr").
        let df = ResolvedDotfile {
            source: source.clone(),
            target: target.clone(),
            module_name: "hypr".to_string(),
        };
        let mut state = DotfilesState {
            backed_up: Vec::new(),
        };
        let mut stats = SyncStats::default();
        sync_single_dotfile(&df, &mut state, &mut stats, false, true, &paths).unwrap();
        assert!(target.is_symlink(), "sync should have symlinked the target");
        assert!(
            find_backup(&target).is_some(),
            "sync should have backed up the original"
        );
        save_dotfiles_state(&paths, &state).unwrap();

        // The module is now disabled (empty enabled_modules) → prune must
        // restore the original, not leave it orphaned.
        let config: Config = serde_yaml::from_str("host: testhost\n").unwrap();
        prune_dotfiles(&paths, &config, true).unwrap();

        assert!(
            !target.is_symlink(),
            "prune should have removed our symlink"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "USER_ORIGINAL",
            "the user's original file must be restored at the target — not lost"
        );
        assert!(
            find_backup(&target).is_none(),
            "the backup must be consumed by the restore, not orphaned on disk"
        );
        let after = load_dotfiles_state(&paths).unwrap();
        assert!(
            after.backed_up.is_empty(),
            "the backup record must be dropped only after a successful restore"
        );
    }

    /// A real file the user placed at the target after the symlink was removed
    /// must never be clobbered by the restore, and its backup record is kept so
    /// the original isn't silently orphaned.
    #[test]
    fn test_prune_does_not_clobber_user_replaced_target() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let paths = make_paths(root);

        let target = root.join("home/.config/foo.conf");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let backup = root.join("home/.config/foo.conf.backup.old");
        fs::write(&backup, "ORIGINAL").unwrap();
        // The user has since created a fresh real file at the target.
        fs::write(&target, "USER_REPLACED").unwrap();

        let state = DotfilesState {
            backed_up: vec![BackedUpDotfile {
                target: target.to_string_lossy().to_string(),
                backup: backup.to_string_lossy().to_string(),
                module: "foo".to_string(),
                backed_up_at: "2020-01-01T00:00:00Z".to_string(),
            }],
        };
        save_dotfiles_state(&paths, &state).unwrap();

        let config: Config = serde_yaml::from_str("host: testhost\n").unwrap();
        prune_dotfiles(&paths, &config, true).unwrap();

        // The user's replacement file is untouched...
        assert_eq!(fs::read_to_string(&target).unwrap(), "USER_REPLACED");
        // ...the backup is left in place...
        assert!(
            backup.exists(),
            "backup must not be dropped when it can't be restored"
        );
        // ...and its record is kept so the original isn't orphaned/forgotten.
        let after = load_dotfiles_state(&paths).unwrap();
        assert_eq!(after.backed_up.len(), 1, "record must be kept, not dropped");
    }
}
