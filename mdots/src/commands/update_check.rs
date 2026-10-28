//! `mdots update --check`: what an update would bring in, before anything is installed. Pacman (via `checkupdates`, else the
//! cached databases), the AUR helper (`-Qua`) and Flatpak (`flatpak remote-ls --updates`, in the configured scope). All queries
//! are read-only; a source that cannot be checked is reported as a warning instead of failing the whole check.

use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::backend::{PackageUpdate, PkgBackend};
use crate::config::{Config, FlatpakScope};

/// One source of updates in the report.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateSection {
    pub label: String,
    pub updates: Vec<PackageUpdate>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

impl UpdateSection {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            updates: Vec::new(),
            warnings: Vec::new(),
            notes: Vec::new(),
        }
    }
}

/// The whole check: the JSON form (`--json`) is this plus `total_updates`.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckReport {
    pub package_manager: UpdateSection,
    pub flatpak: UpdateSection,
}

impl UpdateCheckReport {
    pub fn total_updates(&self) -> usize {
        self.package_manager.updates.len() + self.flatpak.updates.len()
    }

    fn has_warnings(&self) -> bool {
        !self.package_manager.warnings.is_empty() || !self.flatpak.warnings.is_empty()
    }
}

/// Collects the available updates from the native package manager (with the AUR) and Flatpak.
pub fn collect(config: &Config, backend: &dyn PkgBackend) -> UpdateCheckReport {
    UpdateCheckReport {
        package_manager: check_package_manager(backend),
        flatpak: check_flatpak(config),
    }
}

/// Prints the report as text, or as JSON for scripts (`mdots update --check --json`).
pub fn print_report(report: &UpdateCheckReport, json: bool) -> Result<()> {
    if json {
        #[derive(Serialize)]
        struct JsonReport<'a> {
            package_manager: &'a UpdateSection,
            flatpak: &'a UpdateSection,
            total_updates: usize,
        }
        let output = JsonReport {
            package_manager: &report.package_manager,
            flatpak: &report.flatpak,
            total_updates: report.total_updates(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!();
    println!("{}", "=== Update Check ===".blue().bold());
    print_section(&report.package_manager);
    print_section(&report.flatpak);

    println!();
    let total = report.total_updates();
    if total == 0 {
        if report.has_warnings() {
            println!(
                "{}",
                "⚠ No updates found in the sources that could be checked".yellow()
            );
        } else {
            println!("{}", "✓ No updates found".green());
        }
    } else {
        let plural = if total == 1 { "" } else { "s" };
        println!(
            "{}",
            format!("Total: {} update{} available", total, plural)
                .yellow()
                .bold()
        );
    }
    Ok(())
}

fn print_section(section: &UpdateSection) {
    println!();
    println!("{}:", section.label.as_str().cyan().bold());
    if section.updates.is_empty() {
        let line = if section.warnings.is_empty() {
            "no updates found".dimmed()
        } else {
            "check incomplete".yellow()
        };
        println!("  {}", line);
    } else {
        let width = section
            .updates
            .iter()
            .map(|u| u.name.len())
            .max()
            .unwrap_or(0)
            .min(48);
        for update in &section.updates {
            let version = version_text(update);
            if version.is_empty() {
                println!("  {}", update.name);
            } else {
                println!("  {:<width$}  {}", update.name, version, width = width);
            }
        }
    }
    for warning in &section.warnings {
        println!("  {} {}", "!".yellow(), warning.as_str().yellow());
    }
    for note in &section.notes {
        println!("  {} {}", "→".blue(), note.as_str().dimmed());
    }
}

/// `1.0 -> 1.1`, or what is known of it.
fn version_text(update: &PackageUpdate) -> String {
    match (&update.current, &update.available) {
        (Some(current), Some(available)) => format!("{} -> {}", current, available),
        (None, Some(available)) => format!("-> {}", available),
        (Some(current), None) => current.clone(),
        (None, None) => String::new(),
    }
}

fn check_package_manager(backend: &dyn PkgBackend) -> UpdateSection {
    let mut section = UpdateSection::new("Package manager (pacman/AUR)");
    match backend.check_updates() {
        Ok(updates) => section.updates = updates,
        Err(error) => section
            .warnings
            .push(format!("Update check failed: {}", error)),
    }
    if which::which("checkupdates").is_err() {
        section.notes.push(
            "checkupdates not found; used the cached pacman databases. Install pacman-contrib for a fresh check."
                .to_string(),
        );
    }
    section
}

fn check_flatpak(config: &Config) -> UpdateSection {
    let scope = config.flatpak_scope.as_arg();
    let scope_name = match config.flatpak_scope {
        FlatpakScope::User => "user",
        FlatpakScope::System => "system",
    };
    let mut section = UpdateSection::new(format!("Flatpak ({})", scope_name));
    if which::which("flatpak").is_err() {
        section
            .notes
            .push("Skipped: Flatpak is not installed.".to_string());
        return section;
    }
    match check_flatpak_updates(scope) {
        Ok(updates) => section.updates = updates,
        Err(error) => section
            .warnings
            .push(format!("Flatpak update check failed: {}", error)),
    }
    section
}

/// The Flatpak refs with an update in `scope` (`--user` / `--system`), with the installed version where Flatpak knows it.
fn check_flatpak_updates(scope: &str) -> Result<Vec<PackageUpdate>> {
    let installed = flatpak_installed_versions(scope).unwrap_or_default();
    let output = Command::new("flatpak")
        .args([
            "remote-ls",
            "--updates",
            "--json",
            "--columns=ref,version,origin",
            scope,
        ])
        .output()
        .context("Failed to run flatpak remote-ls --updates")?;
    if !output.status.success() {
        anyhow::bail!(
            "flatpak remote-ls --updates failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_flatpak_updates(&String::from_utf8_lossy(&output.stdout), &installed)
}

fn flatpak_installed_versions(scope: &str) -> Result<HashMap<String, String>> {
    let output = Command::new("flatpak")
        .args(["list", "--json", "--columns=ref,version", scope])
        .output()
        .context("Failed to run flatpak list")?;
    if !output.status.success() {
        return Ok(HashMap::new());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Failed to parse flatpak list JSON")?;
    let mut versions = HashMap::new();
    for entry in value.as_array().into_iter().flatten() {
        let Some(reference) = entry.get("ref").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(version) = non_empty(entry.get("version")) {
            versions.insert(normalize_flatpak_ref(reference), version);
        }
    }
    Ok(versions)
}

/// Flatpak prints `""` for a ref without a version (most runtimes): treat that as unknown.
fn non_empty(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn parse_flatpak_updates(
    text: &str,
    installed: &HashMap<String, String>,
) -> Result<Vec<PackageUpdate>> {
    // `flatpak remote-ls --updates --json` prints nothing (not `[]`) when there are no updates.
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(text).context("Failed to parse flatpak remote-ls JSON")?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    let entries = value
        .as_array()
        .context("Unexpected flatpak remote-ls JSON shape")?;

    let mut updates = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(reference) = entry.get("ref").and_then(|v| v.as_str()) else {
            continue;
        };
        let normalized = normalize_flatpak_ref(reference);
        if !seen.insert(normalized.clone()) {
            continue;
        }
        updates.push(PackageUpdate {
            name: flatpak_ref_name(reference),
            current: installed.get(&normalized).cloned(),
            available: non_empty(entry.get("version")),
        });
    }
    Ok(updates)
}

/// `flatpak list` refs omit the `app/` / `runtime/` prefix that `remote-ls` prints: strip it so the two compare.
fn normalize_flatpak_ref(reference: &str) -> String {
    match reference.split_once('/') {
        Some(("app" | "runtime", rest)) => rest.to_string(),
        _ => reference.to_string(),
    }
}

/// The app or runtime ID of a ref (`app/org.foo.Bar/x86_64/stable` -> `org.foo.Bar`).
fn flatpak_ref_name(reference: &str) -> String {
    normalize_flatpak_ref(reference)
        .split('/')
        .next()
        .unwrap_or(reference)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flatpak_updates_with_the_installed_version() {
        let mut installed = HashMap::new();
        installed.insert(
            "com.spotify.Client/x86_64/stable".to_string(),
            "1.2.3".to_string(),
        );
        let updates = parse_flatpak_updates(
            r#"[{"ref":"app/com.spotify.Client/x86_64/stable","version":"1.2.4","origin":"flathub"},
                {"ref":"runtime/org.gnome.Platform/x86_64/50","version":"","origin":"flathub"}]"#,
            &installed,
        )
        .unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "com.spotify.Client");
        assert_eq!(updates[0].current.as_deref(), Some("1.2.3"));
        assert_eq!(updates[0].available.as_deref(), Some("1.2.4"));
        // A runtime without a version: unknown, not an empty string.
        assert_eq!(updates[1].name, "org.gnome.Platform");
        assert_eq!(updates[1].available, None);
        assert_eq!(version_text(&updates[1]), "");
    }

    #[test]
    fn empty_flatpak_output_means_no_updates() {
        assert!(parse_flatpak_updates("", &HashMap::new())
            .unwrap()
            .is_empty());
        assert!(parse_flatpak_updates("  \n", &HashMap::new())
            .unwrap()
            .is_empty());
        assert!(parse_flatpak_updates("null", &HashMap::new())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn normalizes_refs_from_list_and_remote_ls() {
        assert_eq!(
            normalize_flatpak_ref("app/org.foo.Bar/x86_64/stable"),
            "org.foo.Bar/x86_64/stable"
        );
        assert_eq!(
            normalize_flatpak_ref("org.foo.Bar/x86_64/stable"),
            "org.foo.Bar/x86_64/stable"
        );
        assert_eq!(
            flatpak_ref_name("runtime/org.gnome.Platform/x86_64/50"),
            "org.gnome.Platform"
        );
        assert_eq!(flatpak_ref_name("org.foo.Bar/x86_64/stable"), "org.foo.Bar");
    }

    #[test]
    fn counts_every_source_in_the_total() {
        let update = PackageUpdate {
            name: "a".into(),
            current: None,
            available: None,
        };
        let mut report = UpdateCheckReport {
            package_manager: UpdateSection::new("pm"),
            flatpak: UpdateSection::new("fp"),
        };
        assert_eq!(report.total_updates(), 0);
        report.package_manager.updates.push(update.clone());
        report.flatpak.updates.push(update);
        assert_eq!(report.total_updates(), 2);
    }
}
