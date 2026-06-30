use anyhow::{Context, Result};
use colored::*;
use std::process::Command;

use crate::config::{load_config, ConfigPaths, PackageManagerType};

/// Run interactive package search with fzf and install selected packages
pub fn run(paths: &ConfigPaths) -> Result<()> {
    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!("fzf is not installed. Please install fzf to use the search command.");
    }

    // Load config and create backend
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;
    let pm_type = crate::config::resolve_package_manager(&config)?;

    // Build the fzf command depending on the package manager
    let fzf_cmd = match pm_type {
        PackageManagerType::Pacman => {
            let info_cmd = backend.package_info_command().to_string();
            format!(
                "{cmd} -Slq --color=never 2>/dev/null | fzf \
                --multi \
                --ansi \
                --preview '{cmd} -Si {{1}} --color=never' \
                --preview-window=right:60%:wrap \
                --header='→ Interactive package search\nℹ Use TAB to select multiple packages, ENTER to install' \
                --prompt='Search packages > ' \
                --height=100% \
                --border=rounded \
                --border-label=' mdots search ' \
                --border-label-pos=2 \
                --color=border:blue,label:cyan",
                cmd = info_cmd
            )
        }
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(fzf_cmd)
        .output()
        .context("Failed to run fzf")?;

    if !output.status.success() {
        println!("{} Search cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No packages selected", "✗".yellow());
        return Ok(());
    }

    let packages: Vec<&str> = selected.lines().collect();

    println!();
    println!("{} Installing {} package(s)...", "→".blue(), packages.len());
    println!();

    // Install each package using mdots install
    let mut installed_count = 0;
    let mut failed_count = 0;

    for package in &packages {
        println!("{} Installing: {}", "→".blue(), package.green());

        let status = Command::new("mdots")
            .args(["install", package])
            .status()
            .context(format!("Failed to install package: {}", package))?;

        if status.success() {
            installed_count += 1;
        } else {
            failed_count += 1;
            eprintln!("{} Failed to install: {}", "✗".red(), package);
            if packages.len() > 1 {
                eprintln!("{} Continuing with remaining packages...", "→".yellow());
            }
        }
    }

    println!();
    if installed_count > 0 {
        println!(
            "{} Successfully installed {} package(s)",
            "✓".green(),
            installed_count
        );
    }
    if failed_count > 0 {
        println!(
            "{} {} package(s) failed or were cancelled",
            "✗".yellow(),
            failed_count
        );
    }

    Ok(())
}
