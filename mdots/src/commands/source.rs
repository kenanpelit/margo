use anyhow::Result;
use colored::*;

use crate::config::ConfigPaths;
use crate::source::{builder, discover_sources, find_source};

/// List all declared sources
pub fn list(paths: &ConfigPaths) -> Result<()> {
    let sources = discover_sources(paths)?;

    if sources.is_empty() {
        println!(
            "{}",
            "No sources found. Create a source config in ~/.config/arch-config/sources/".yellow()
        );
        return Ok(());
    }

    println!("{}", "=== Declared Sources ===".blue().bold());
    println!();

    for source in &sources {
        let installed = builder::is_pacman_installed(&source.config.name);
        let status = if installed {
            "installed".green()
        } else {
            "not installed".yellow()
        };

        let format_tag = if source.config.custom_pkgbuild.is_some() {
            " [custom PKGBUILD]".dimmed()
        } else {
            "".dimmed()
        };

        println!(
            "  {} {}  [{}]{}",
            "•".cyan(),
            source.config.name.bold(),
            status,
            format_tag
        );

        if !source.config.description.is_empty() {
            println!("    {}", source.config.description.dimmed());
        }
        println!("    {}", source.config.url.dimmed());
        if let Some(branch) = &source.config.branch {
            println!("    branch: {}", branch.dimmed());
        }
    }

    println!();
    println!("  {} source(s) declared", sources.len().to_string().bold());

    Ok(())
}

/// Build all sources (or a specific one)
pub fn build(paths: &ConfigPaths, name: Option<&str>) -> Result<()> {
    let sources = if let Some(n) = name {
        vec![find_source(paths, n)?]
    } else {
        discover_sources(paths)?
    };

    if sources.is_empty() {
        println!(
            "{}",
            "No sources found. Create a source config in ~/.config/arch-config/sources/".yellow()
        );
        return Ok(());
    }

    println!("{}", "=== Building Sources ===".blue().bold());
    println!();

    let mut failed = Vec::new();

    for source in &sources {
        let result = builder::build_source(source, false)?;
        if !result.success {
            failed.push(result.name);
        }
    }

    println!();
    if failed.is_empty() {
        println!("{}", "All sources processed successfully.".green());
    } else {
        println!(
            "{} {} source(s) failed: {}",
            "✗".red(),
            failed.len().to_string().bold(),
            failed.join(", ")
        );
        anyhow::bail!("Some builds failed");
    }

    Ok(())
}

/// Force rebuild all sources (or a specific one)
pub fn rebuild(paths: &ConfigPaths, name: Option<&str>) -> Result<()> {
    let sources = if let Some(n) = name {
        vec![find_source(paths, n)?]
    } else {
        discover_sources(paths)?
    };

    if sources.is_empty() {
        println!(
            "{}",
            "No sources found. Create a source config in ~/.config/arch-config/sources/".yellow()
        );
        return Ok(());
    }

    println!("{}", "=== Rebuilding Sources ===".blue().bold());
    println!();

    let mut failed = Vec::new();

    for source in &sources {
        let result = builder::build_source(source, true)?;
        if !result.success {
            failed.push(result.name);
        }
    }

    println!();
    if failed.is_empty() {
        println!("{}", "All sources rebuilt successfully.".green());
    } else {
        println!(
            "{} {} source(s) failed: {}",
            "✗".red(),
            failed.len().to_string().bold(),
            failed.join(", ")
        );
        anyhow::bail!("Some builds failed");
    }

    Ok(())
}

/// Remove (uninstall) a source-built package
pub fn remove(name: &str) -> Result<()> {
    if !builder::is_pacman_installed(name) {
        println!("{} '{}' is not installed", "→".yellow(), name.bold());
        return Ok(());
    }

    println!("Removing {}...", name.bold());
    builder::remove_source_package(name)?;
    println!("{} {} removed", "✓".green(), name.bold());

    Ok(())
}

/// Show status of all sources (installed vs not, build dirs)
pub fn status(paths: &ConfigPaths) -> Result<()> {
    let sources = discover_sources(paths)?;

    if sources.is_empty() {
        println!("{}", "No sources declared.".yellow());
        return Ok(());
    }

    println!("{}", "=== Source Status ===".blue().bold());
    println!();

    let installed_count = sources
        .iter()
        .filter(|s| builder::is_pacman_installed(&s.config.name))
        .count();

    for source in &sources {
        let installed = builder::is_pacman_installed(&source.config.name);
        let marker = if installed {
            "✓".green()
        } else {
            "✗".red()
        };
        println!("  {} {}", marker, source.config.name.bold());
    }

    println!();
    println!(
        "  {}/{} installed",
        installed_count.to_string().bold(),
        sources.len().to_string().bold()
    );

    Ok(())
}

/// Build sources as part of mdots sync (only uninstalled ones)
pub fn sync_sources(paths: &ConfigPaths) -> Result<()> {
    let sources = discover_sources(paths)?;

    let uninstalled: Vec<_> = sources
        .iter()
        .filter(|s| !builder::is_pacman_installed(&s.config.name))
        .collect();

    if uninstalled.is_empty() {
        return Ok(());
    }

    println!();
    println!(
        "{} {} source package(s) to build",
        "→".cyan(),
        uninstalled.len().to_string().bold()
    );

    let mut failed = Vec::new();
    for source in uninstalled {
        let result = builder::build_source(source, false)?;
        if !result.success {
            failed.push(result.name);
        }
    }

    if !failed.is_empty() {
        println!(
            "{} {} source build(s) failed: {}",
            "✗".red(),
            failed.len().to_string().bold(),
            failed.join(", ")
        );
    }

    Ok(())
}
