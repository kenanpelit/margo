//! Sync bookkeeping on disk: package state (drives `--prune`), modules state,
//! and the sync-in-progress marker.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::{ConfigPaths, PackageType, load_config};
use crate::package::Package;

/// State tracking for sequential module processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ModulesState {
    #[serde(default = "Utc::now")]
    pub(super) last_updated: DateTime<Utc>,

    #[serde(default)]
    pub(super) completed_modules: HashSet<String>,
}

impl Default for ModulesState {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            completed_modules: HashSet::new(),
        }
    }
}

pub(super) fn load_modules_state(state_file: &Path) -> Result<ModulesState> {
    if !state_file.exists() {
        return Ok(ModulesState::default());
    }

    let content =
        std::fs::read_to_string(state_file).context("Failed to read modules state file")?;

    let state: ModulesState =
        yaml_serde::from_str(&content).context("Failed to parse modules state YAML")?;

    Ok(state)
}

pub(super) fn save_modules_state(state_file: &Path, state: &ModulesState) -> Result<()> {
    if let Some(parent) = state_file.parent() {
        std::fs::create_dir_all(parent).context("Failed to create state directory")?;
    }

    let yaml = yaml_serde::to_string(state).context("Failed to serialize modules state")?;

    crate::atomic_write::write_atomic(state_file, &yaml)
        .context("Failed to write modules state file")?;

    Ok(())
}

// ===== Sequential Module Processing Helper Functions =====

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct StatePackage {
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) pkg_type: Option<String>,
}

#[derive(Debug)]
pub(super) struct StateFile {
    pub(super) packages: Vec<StatePackage>,
}

pub(super) fn load_state_file(paths: &ConfigPaths) -> Result<StateFile> {
    use yaml_serde::Value;

    let content =
        std::fs::read_to_string(&paths.state_file).context("Failed to read state file")?;

    let yaml: Value = yaml_serde::from_str(&content).context("Failed to parse state file")?;

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

/// State for `--prune`: the packages mdots recorded as managed. A missing file
/// means nothing is tracked yet (first sync) and is fine; a file that exists but
/// can't be read or parsed is an error — silently treating it as empty would
/// make `--prune` a no-op without telling anyone.
pub(super) fn load_prune_state(paths: &ConfigPaths) -> Result<StateFile> {
    if !paths.state_file.exists() {
        return Ok(StateFile {
            packages: Vec::new(),
        });
    }
    load_state_file(paths).with_context(|| {
        format!(
            "package state file {} is unreadable; refusing to prune. Fix it, or delete it \
             (mdots then forgets which packages it installed) and re-run `mdots sync`",
            paths.state_file.display()
        )
    })
}

/// Render the package state file. `version_of` looks up the installed
/// version; packages without one are recorded without a `version:` key.
pub(super) fn render_package_state(
    declared: &[Package],
    mut version_of: impl FnMut(&Package) -> Option<String>,
) -> String {
    use chrono::Utc;
    use std::fmt::Write as _;

    let mut out = String::new();
    out.push_str("# Auto-generated state file - tracks mdots-managed packages\n");
    let _ = writeln!(out, "# Generated: {}", Utc::now().to_rfc3339());
    out.push_str("\npackages:\n");
    for pkg in declared {
        let _ = writeln!(out, "  - name: {}", pkg.name);
        if let Some(ver) = version_of(pkg) {
            let _ = writeln!(out, "    version: \"{}\"", ver);
        }
        let type_str = match pkg.package_type {
            PackageType::Flatpak => "flatpak",
            PackageType::Native => "native",
            PackageType::Nix => "nix",
        };
        let _ = writeln!(out, "    type: {}", type_str);
    }
    out
}

/// Write the rendered package state atomically: this file drives `--prune`,
/// so a crash mid-write must leave the previous version, not a torn one.
pub(super) fn write_package_state(paths: &ConfigPaths, contents: &str) -> Result<()> {
    crate::atomic_write::write_atomic(&paths.state_file, contents).context(format!(
        "Failed to write state file: {:?}",
        paths.state_file
    ))
}

pub(super) fn update_state_file(paths: &ConfigPaths, declared: &[Package]) -> Result<()> {
    // Get current versions (don't fail if we can't get one).
    let contents = render_package_state(declared, |pkg| {
        if pkg.package_type == PackageType::Flatpak {
            get_flatpak_version(&pkg.name).unwrap_or(None)
        } else {
            get_package_version(&pkg.name, paths).unwrap_or(None)
        }
    });
    write_package_state(paths, &contents)
}

/// Path of the "a sync is mid-flight" marker inside the state directory.
pub(super) fn sync_marker_path(state_dir: &Path) -> PathBuf {
    state_dir.join("sync-in-progress")
}

/// Drop a marker recording that a system-mutating sync has begun. It is removed
/// only on a clean finish ([`clear_sync_marker`]); if the process dies or errors
/// out mid-sync the marker survives, so the next run can warn the user that the
/// system may be partially applied. Best-effort but creates the state dir first.
pub(super) fn begin_sync_marker(state_dir: &Path) -> std::io::Result<()> {
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
pub(super) fn clear_sync_marker(state_dir: &Path) {
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
pub(super) fn commit_package_state(
    paths: &ConfigPaths,
    declared: &[Package],
    json: bool,
) -> Result<()> {
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

pub(super) fn get_package_version(package: &str, paths: &ConfigPaths) -> Result<Option<String>> {
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;
    backend.get_package_version(package)
}

pub(super) fn get_flatpak_version(package: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("flatpak")
        .args(["info", package])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let result = String::from_utf8_lossy(&out.stdout);
            for line in result.lines() {
                if line.starts_with("Version:")
                    && let Some(ver) = line.split(':').nth(1)
                {
                    return Ok(Some(ver.trim().to_string()));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}
