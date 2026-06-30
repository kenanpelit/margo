use anyhow::{Context, Result};
use colored::*;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the dcli git repository
fn find_dcli_repo() -> Option<PathBuf> {
    // Check DCLI_REPO_PATH environment variable first
    if let Ok(path) = env::var("DCLI_REPO_PATH") {
        let repo_path = PathBuf::from(path);
        if repo_path.join(".git").exists() {
            return Some(repo_path);
        }
    }

    // Search common locations
    let home = env::var("HOME").ok()?;
    let search_paths = vec![
        format!("{}/dcli", home),
        format!("{}/projects/dcli", home),
        format!("{}/git/dcli", home),
        "/home/don/dcli".to_string(), // Hardcoded fallback
    ];

    for path in search_paths {
        let repo_path = PathBuf::from(&path);
        if repo_path.join(".git").exists() {
            return Some(repo_path);
        }
    }

    None
}

/// Get current git commit hash
fn get_current_commit(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .context("Failed to get current commit")?;

    if output.status.success() {
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    } else {
        Ok("unknown".to_string())
    }
}

/// Check if there are uncommitted changes
fn has_uncommitted_changes(repo_path: &Path) -> Result<bool> {
    let status = Command::new("git")
        .current_dir(repo_path)
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()
        .context("Failed to check git status")?;

    Ok(!status.success())
}

/// Self-update dcli from git repository
pub fn run() -> Result<()> {
    println!("{}", "=== dcli Self-Update ===".blue());
    println!();

    // Find dcli repository
    let dcli_repo = match find_dcli_repo() {
        Some(repo) => repo,
        None => {
            println!(
                "{}",
                "Could not auto-detect dcli repository location".yellow()
            );
            println!();
            println!("Please enter the path to your dcli git repository:");
            print!("Path: ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let repo_path = PathBuf::from(input.trim());

            if !repo_path.join(".git").exists() {
                anyhow::bail!("Not a git repository: {}", repo_path.display());
            }

            repo_path
        }
    };

    println!(
        "{} Found dcli repository at: {}",
        "→".blue(),
        dcli_repo.display().to_string().green()
    );
    println!();

    // Check for uncommitted changes
    if has_uncommitted_changes(&dcli_repo)? {
        println!(
            "{}",
            "Warning: You have uncommitted changes in the dcli repository".yellow()
        );

        Command::new("git")
            .current_dir(&dcli_repo)
            .args(["status", "--short"])
            .status()?;

        println!();
        print!("Continue anyway? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Update cancelled".yellow());
            return Ok(());
        }
    }

    // Show current version
    let current_commit = get_current_commit(&dcli_repo)?;
    println!("{} {}", "Current version:".blue(), current_commit);

    // Pull latest changes
    println!();
    let spinner = crate::progress::create_spinner("Pulling latest changes from git...");

    let status = Command::new("git")
        .current_dir(&dcli_repo)
        .arg("pull")
        .status()
        .context("Failed to run git pull")?;

    if !status.success() {
        spinner.finish_with_message("✗ Failed to pull updates");
        anyhow::bail!("Failed to pull updates");
    }
    spinner.finish_with_message("✓ Git pull complete");

    // Check new version
    let new_commit = get_current_commit(&dcli_repo)?;

    if current_commit == new_commit {
        println!();
        println!("{}", "✓ Already up to date!".green());
        return Ok(());
    }

    println!("{} {}", "New version:".blue(), new_commit);
    println!();

    // Check if cargo is installed
    let cargo_cmd = if let Ok(cargo_path) = which::which("cargo") {
        cargo_path.to_string_lossy().to_string()
    } else {
        println!("{}", "Error: cargo (Rust toolchain) not found".red());
        println!();
        println!("dcli self-update requires Rust to build from source.");
        println!();
        println!("Would you like to install Rust now with rustup? [Y/n]");
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
            println!();
            println!("{}", "Installing Rust with rustup...".blue());

            let rustup_status = Command::new("sh")
                .args([
                    "-c",
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                ])
                .status()
                .context("Failed to run rustup installer")?;

            if !rustup_status.success() {
                anyhow::bail!(
                    "Failed to install Rust. Please install manually: https://rustup.rs/"
                );
            }

            println!("{}", "✓ Rust installed successfully".green());
            println!();
            println!(
                "{}",
                "Please restart your terminal and run 'dcli self-update' again.".yellow()
            );
            println!("Or run: source $HOME/.cargo/env && dcli self-update");
            return Ok(());
        } else {
            println!(
                "{}",
                "Installation cancelled. Please install Rust manually: https://rustup.rs/".yellow()
            );
            return Ok(());
        }
    };

    // Build the Rust binary
    let build_spinner = crate::progress::create_spinner(
        "Building Rust binary with cargo (this may take a minute)...",
    );

    let build_status = Command::new(&cargo_cmd)
        .current_dir(&dcli_repo)
        .args(["build", "--release"])
        .status()
        .context("Failed to run cargo build")?;

    if !build_status.success() {
        build_spinner.finish_with_message("✗ Build failed");
        anyhow::bail!("Failed to build dcli binary");
    }

    build_spinner.finish_with_message("✓ Build completed successfully");

    // Install updated version
    println!();
    println!(
        "{} Installing updated dcli to /usr/local/bin...",
        "→".blue()
    );

    let binary_path = dcli_repo.join("target/release/dcli");

    if !binary_path.exists() {
        anyhow::bail!("Built binary not found at: {}", binary_path.display());
    }

    // Use install command which handles replacing running binaries better than cp
    let install_status = Command::new("sudo")
        .args([
            "install",
            "-m",
            "755",
            &binary_path.to_string_lossy(),
            "/usr/local/bin/dcli",
        ])
        .status()
        .context("Failed to install binary")?;

    if !install_status.success() {
        anyhow::bail!("Failed to install updated dcli");
    }

    println!("{} Installed to /usr/local/bin/dcli", "✓".green());

    // Success message
    println!();
    println!("{}", "=== Update Complete! ===".green());
    println!();
    println!("Changes: {} → {}", current_commit, new_commit);
    println!();
    println!("To see what changed, run:");
    println!(
        "  cd {} && git log --oneline {}..{}",
        dcli_repo.display(),
        current_commit,
        new_commit
    );
    println!();
    println!(
        "{}",
        "Note: Restart your terminal if the command doesn't work immediately.".yellow()
    );

    Ok(())
}
