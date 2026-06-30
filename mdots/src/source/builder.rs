use anyhow::{Context, Result};
use colored::*;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{pkgbuild::generate_pkgbuild, SourceInfo};

/// Result of a build operation
pub struct BuildResult {
    pub name: String,
    pub success: bool,
    /// Path to the build directory (kept on failure for debugging)
    #[allow(dead_code)]
    pub build_dir: PathBuf,
}

/// Build and install a source package using makepkg
pub fn build_source(source: &SourceInfo, force: bool) -> Result<BuildResult> {
    let name = &source.config.name;

    // Check if already installed (skip unless force rebuild)
    if !force && is_pacman_installed(name) {
        println!(
            "  {} {} (already installed, use rebuild to force)",
            "→".cyan(),
            name.bold()
        );
        return Ok(BuildResult {
            name: name.clone(),
            success: true,
            build_dir: PathBuf::new(),
        });
    }

    println!("  {} Building {}...", "→".cyan(), name.bold());

    // Determine build directory
    let build_dir = if source.config.cache_builds {
        let cache_dir = dirs_build_cache(name)?;
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create cache dir: {}", cache_dir.display()))?;
        cache_dir
    } else {
        let tmp_dir = std::env::temp_dir().join(format!("dcli-source-{}", name));
        // Clean temp dir for fresh build
        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir)
                .with_context(|| format!("Failed to clean temp dir: {}", tmp_dir.display()))?;
        }
        std::fs::create_dir_all(&tmp_dir)
            .with_context(|| format!("Failed to create temp dir: {}", tmp_dir.display()))?;
        tmp_dir
    };

    // Write or copy PKGBUILD
    let pkgbuild_path = build_dir.join("PKGBUILD");
    if let Some(custom_pkgbuild) = source.custom_pkgbuild_path() {
        if !custom_pkgbuild.exists() {
            anyhow::bail!("Custom PKGBUILD not found: {}", custom_pkgbuild.display());
        }
        std::fs::copy(&custom_pkgbuild, &pkgbuild_path).with_context(|| {
            format!(
                "Failed to copy custom PKGBUILD from {}",
                custom_pkgbuild.display()
            )
        })?;
        println!("    Using custom PKGBUILD: {}", custom_pkgbuild.display());
    } else {
        let pkgbuild_content = generate_pkgbuild(&source.config)?;
        std::fs::write(&pkgbuild_path, &pkgbuild_content)
            .with_context(|| format!("Failed to write PKGBUILD to {}", pkgbuild_path.display()))?;
    }

    // Run makepkg -si --noconfirm --rmdeps
    let result = run_makepkg(&build_dir, force);

    match result {
        Ok(()) => {
            println!("  {} {} built and installed", "✓".green(), name.bold());
            // Clean up temp dir on success (keep cache dir)
            if !source.config.cache_builds {
                let _ = std::fs::remove_dir_all(&build_dir);
            }
            Ok(BuildResult {
                name: name.clone(),
                success: true,
                build_dir,
            })
        }
        Err(e) => {
            println!("  {} Failed to build {}: {}", "✗".red(), name.bold(), e);
            println!(
                "    Build directory kept for inspection: {}",
                build_dir.display()
            );
            Ok(BuildResult {
                name: name.clone(),
                success: false,
                build_dir,
            })
        }
    }
}

fn run_makepkg(build_dir: &Path, force: bool) -> Result<()> {
    let mut args = vec![
        "-si",         // sync deps + install
        "--noconfirm", // no prompts
        "--rmdeps",    // remove makedepends after build
    ];

    if force {
        args.push("-f"); // overwrite existing package
    }

    let status = Command::new("makepkg")
        .args(&args)
        .current_dir(build_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to run makepkg — is base-devel installed?")?;

    if !status.success() {
        anyhow::bail!("makepkg exited with code {}", status.code().unwrap_or(-1));
    }

    Ok(())
}

/// Check if a package is installed via pacman
pub fn is_pacman_installed(name: &str) -> bool {
    Command::new("pacman")
        .args(["-Qi", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Uninstall a package via pacman -R
pub fn remove_source_package(name: &str) -> Result<()> {
    let status = Command::new("sudo")
        .args(["pacman", "-R", "--noconfirm", name])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to run pacman -R")?;

    if !status.success() {
        anyhow::bail!(
            "pacman -R {} exited with code {}",
            name,
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

fn dirs_build_cache(name: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("dcli")
        .join("sources")
        .join(name))
}
