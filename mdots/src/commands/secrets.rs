//! `mdots secrets` — manage SOPS/age-encrypted secrets.
//!
//! Decryption logic lives in `crate::secrets`; this module is the CLI surface:
//! `status`, `sync`, `edit`, `list`, and `keygen`.

use anyhow::{bail, Context, Result};
use colored::*;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use crate::config::{load_config, ConfigPaths};
use crate::secrets::{
    classify_secret_status, compute_orphans, load_secrets_state, resolve_key_path,
    resolve_secret_target, secret_name, sops_available, SecretState,
};

fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var("HOME").context("HOME environment variable not set")?,
    ))
}

/// The conventional age key location SOPS falls back to when no path is set.
fn default_age_key_path(home: &std::path::Path) -> PathBuf {
    home.join(".config/sops/age/keys.txt")
}

/// `mdots secrets status` — read-only health report for declared secrets.
pub fn status(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let home = home_dir()?;
    let repo_root = &paths.config_dir;
    let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home);
    let sops = sops_available();
    let key_available = key_path.as_ref().map(|k| k.exists()).unwrap_or(true);

    // Resolve each declared secret to (name, target, state-label).
    let mut declared_targets: Vec<PathBuf> = Vec::new();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for entry in &config.secrets {
        let name = secret_name(entry);
        let (target_display, label) = match resolve_secret_target(&entry.target, &home, repo_root) {
            Err(e) => (entry.target.clone(), format!("invalid: {}", e)),
            Ok(target) => {
                declared_targets.push(target.clone());
                let source_exists = repo_root.join(&entry.source).exists();
                let target_exists = target.exists();
                let state =
                    classify_secret_status(sops, source_exists, key_available, target_exists);
                let label = match state {
                    SecretState::SopsMissing => "sops not installed".to_string(),
                    SecretState::SourceMissing => "source not found".to_string(),
                    SecretState::KeyMissing => "key missing".to_string(),
                    SecretState::Pending => "pending (not yet decrypted)".to_string(),
                    SecretState::Decrypted => {
                        let mode = std::fs::metadata(&target)
                            .map(|m| m.permissions().mode() & 0o777)
                            .unwrap_or(0);
                        format!("decrypted ({:04o})", mode)
                    }
                };
                (target.display().to_string(), label)
            }
        };
        rows.push((name, target_display, label));
    }

    // Orphans: previously written targets that are no longer declared.
    let prior = load_secrets_state(&paths.state_dir).unwrap_or_default();
    let prior_targets: Vec<PathBuf> = prior.decrypted_targets.iter().map(PathBuf::from).collect();
    let orphans = compute_orphans(&prior_targets, &declared_targets);

    if json {
        let secrets: Vec<_> = rows
            .iter()
            .map(|(name, target, status)| {
                serde_json::json!({ "name": name, "target": target, "status": status })
            })
            .collect();
        let out = serde_json::json!({
            "sops_available": sops,
            "key_available": key_available,
            "secrets": secrets,
            "orphaned": orphans.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("{}", "=== Secrets ===".blue().bold());
    if rows.is_empty() {
        println!("  No secrets declared.");
    }
    for (name, target, label) in &rows {
        let mark = if label.starts_with("decrypted") {
            "✓".green()
        } else if label.starts_with("pending") {
            "•".blue()
        } else {
            "✗".red()
        };
        println!("  {} {}  {}  [{}]", mark, name, target.dimmed(), label);
    }
    for orphan in &orphans {
        println!(
            "  {} {}  [orphaned — run `mdots secrets sync --prune` to remove]",
            "⚠".yellow(),
            orphan.display().to_string().dimmed()
        );
    }
    Ok(())
}

/// `mdots secrets sync` — decrypt declared secrets without a full system sync.
pub fn sync(paths: &ConfigPaths, dry_run: bool, prune: bool, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    crate::secrets::sync_secrets(paths, &config, dry_run, prune, json)
}

/// `mdots secrets list` — list declared secrets (config only, no filesystem).
pub fn list(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;

    if json {
        let secrets: Vec<_> = config
            .secrets
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": secret_name(e),
                    "source": e.source,
                    "target": e.target,
                    "mode": e.mode.clone().unwrap_or_else(|| "0600".to_string()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&secrets)?);
        return Ok(());
    }

    if config.secrets.is_empty() {
        println!("No secrets declared.");
        return Ok(());
    }
    println!("{}", "Declared secrets:".blue().bold());
    for e in &config.secrets {
        println!(
            "  {}  {} -> {} [{}]",
            secret_name(e).bold(),
            e.source,
            e.target,
            e.mode.clone().unwrap_or_else(|| "0600".to_string())
        );
    }
    Ok(())
}

/// `mdots secrets edit <name>` — open the encrypted source in `sops`.
pub fn edit(paths: &ConfigPaths, name: &str) -> Result<()> {
    let config = load_config(paths)?;
    let entry = config
        .secrets
        .iter()
        .find(|e| secret_name(e) == name)
        .ok_or_else(|| anyhow::anyhow!("no secret named {:?} (see `mdots secrets list`)", name))?;

    if !sops_available() {
        bail!("sops is not installed — cannot edit secrets");
    }

    let source = paths.config_dir.join(&entry.source);
    let home = home_dir()?;
    let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home);

    let mut cmd = Command::new("sops");
    cmd.arg(&source);
    if let Some(key) = &key_path {
        cmd.env("SOPS_AGE_KEY_FILE", key);
    }
    let status = crate::process::status_inherited(&mut cmd)
        .with_context(|| format!("launching sops to edit {}", source.display()))?;
    if !status.success() {
        bail!("sops exited without saving changes");
    }
    Ok(())
}

/// `mdots secrets keygen` — generate an age key if one does not exist.
pub fn keygen(paths: &ConfigPaths) -> Result<()> {
    let config = load_config(paths)?;
    let home = home_dir()?;
    let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home)
        .unwrap_or_else(|| default_age_key_path(&home));

    if which::which("age-keygen").is_err() {
        bail!("age-keygen not found — install the `age` package (pacman -S age)");
    }

    if key_path.exists() {
        println!(
            "{} age key already exists at {}",
            "✓".green(),
            key_path.display()
        );
        print_recipient(&key_path)?;
        return Ok(());
    }

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating key directory {}", parent.display()))?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).ok();
    }

    let out = Command::new("age-keygen")
        .arg("-o")
        .arg(&key_path)
        .output()
        .context("running age-keygen")?;
    if !out.status.success() {
        bail!(
            "age-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // age-keygen writes 0600, but enforce it regardless.
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .context("tightening key file permissions")?;

    println!(
        "{} generated age key at {}",
        "✓".green(),
        key_path.display()
    );
    print_recipient(&key_path)?;
    println!();
    println!(
        "Add the public recipient above to your {} so `sops` can encrypt to it, e.g.:",
        ".sops.yaml".cyan()
    );
    println!("  creation_rules:");
    println!("    - age: <recipient-above>");
    Ok(())
}

/// Print the public recipient (`age1…`) derived from a key file.
fn print_recipient(key_path: &std::path::Path) -> Result<()> {
    let out = Command::new("age-keygen")
        .arg("-y")
        .arg(key_path)
        .output()
        .context("deriving public recipient with age-keygen -y")?;
    if out.status.success() {
        let recipient = String::from_utf8_lossy(&out.stdout);
        println!("  public recipient: {}", recipient.trim().green());
    }
    Ok(())
}
