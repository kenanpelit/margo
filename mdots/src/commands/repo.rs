use anyhow::{Context, Result};
use colored::*;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use crate::config::ConfigPaths;

/// Initialize git repository for arch-config
pub fn init(paths: &ConfigPaths) -> Result<()> {
    let config_dir = &paths.config_dir;
    env::set_current_dir(config_dir)
        .with_context(|| format!("Failed to change to directory: {}", config_dir.display()))?;

    // Check if already a git repo
    if is_git_repo() {
        println!("{}", "This directory is already a git repository.".yellow());
        let remote = get_remote_url().unwrap_or_else(|| "No remote configured".to_string());
        println!("Remote: {}", remote);

        print!("Reinitialize? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    println!("{}", "=== Git Repository Setup ===".blue());
    println!();

    // Ask if user wants to set up git
    print!("Version control your arch-config with git? [Y/n] ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("n") {
        println!("{}", "Skipped git setup".yellow());
        return Ok(());
    }

    // Ask for platform
    println!();
    println!("Which platform are you using?");
    println!("  1) GitHub");
    println!("  2) GitLab");
    println!("  3) Other");
    print!("Choice [1-3]: ");
    io::stdout().flush()?;

    let mut platform_choice = String::new();
    io::stdin().read_line(&mut platform_choice)?;

    let platform = match platform_choice.trim() {
        "1" => "GitHub",
        "2" => "GitLab",
        "3" => "Other",
        _ => "GitHub",
    };

    // Ask if repo is created
    println!();
    print!(
        "Have you already created a repository on {}? [y/N] ",
        platform
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        println!();
        println!("{}", "Please create a repository:".blue());
        match platform {
            "GitHub" => {
                println!("  → https://github.com/new");
                println!("  • Name it something like 'arch-config' or 'my-arch-config'");
                println!("  • Make it Private (recommended) or Public");
                println!("  • Don't initialize with README");
            }
            "GitLab" => {
                println!(
                    "  → Create a repo on your Git host (GitHub, GitLab, …) and add it as 'origin'"
                );
                println!("  • Name it something like 'arch-config' or 'my-arch-config'");
                println!("  • Set visibility as desired");
                println!("  • Don't initialize with README");
            }
            _ => {
                println!("  • Create an empty repository on your git platform");
            }
        }
        println!();
        print!("Press Enter when done...");
        io::stdout().flush()?;
        let mut _input = String::new();
        io::stdin().read_line(&mut _input)?;
    }

    // Get repository URL
    println!();
    println!("{}", "Enter your repository URL".blue());
    println!("Examples:");
    println!("  HTTPS: https://github.com/username/arch-config.git");
    println!("  SSH:   git@github.com:username/arch-config.git");
    print!("URL: ");
    io::stdout().flush()?;

    let mut repo_url = String::new();
    io::stdin().read_line(&mut repo_url)?;
    let repo_url = repo_url.trim();

    if !validate_git_url(repo_url) {
        anyhow::bail!("Invalid URL format");
    }

    // Get git user info
    println!();
    print!("Git username: ");
    io::stdout().flush()?;
    let mut git_user = String::new();
    io::stdin().read_line(&mut git_user)?;
    let git_user = git_user.trim();

    print!("Git email: ");
    io::stdout().flush()?;
    let mut git_email = String::new();
    io::stdin().read_line(&mut git_email)?;
    let git_email = git_email.trim();

    println!();
    println!("{}", "Setting up repository...".blue());
    println!();

    // Initialize git if not already
    if !Path::new(".git").exists() {
        println!("{} Initializing git repository...", "→".blue());
        run_git_command(&["init"])?;
    }

    // Add config.yaml to .gitignore
    println!("{} Adding config.yaml to .gitignore...", "→".blue());
    let gitignore_path = Path::new(".gitignore");
    let mut gitignore_content = if gitignore_path.exists() {
        fs::read_to_string(gitignore_path)?
    } else {
        String::new()
    };

    if !gitignore_content.lines().any(|line| line == "config.yaml") {
        if !gitignore_content.is_empty() && !gitignore_content.ends_with('\n') {
            gitignore_content.push('\n');
        }
        gitignore_content.push_str("config.yaml\n");
        fs::write(gitignore_path, gitignore_content)?;
    }

    // Configure git user
    println!("{} Configuring git user...", "→".blue());
    run_git_command(&["config", "user.name", git_user])?;
    run_git_command(&["config", "user.email", git_email])?;

    // Add files
    println!("{} Staging files...", "→".blue());
    run_git_command(&["add", "."])?;

    // Create initial commit
    println!("{} Creating initial commit...", "→".blue());
    let hostname = get_hostname();
    let commit_msg = format!("Initial arch-config setup from {}", hostname);
    let _ = run_git_command(&["commit", "-m", &commit_msg]);

    // Rename branch to main (handles older git versions that default to master)
    println!("{} Ensuring branch is named 'main'...", "→".blue());
    let _ = run_git_command(&["branch", "-M", "main"]);

    // Add remote
    println!("{} Adding remote origin...", "→".blue());
    if get_remote_url().is_some() {
        run_git_command(&["remote", "set-url", "origin", repo_url])?;
    } else {
        run_git_command(&["remote", "add", "origin", repo_url])?;
    }

    // Push to remote
    println!("{} Pushing to remote...", "→".blue());
    match run_git_command(&["push", "-u", "origin", "main"]) {
        Ok(_) => {
            println!();
            println!("{}", "✓ Repository set up successfully!".green());
            println!();
            println!("Your arch-config is now version controlled at:");
            println!("  {}", repo_url);
        }
        Err(_) => {
            println!();
            println!(
                "{}",
                "Repository configured locally, but push failed.".yellow()
            );
            println!(
                "Fix authentication, then run: cd {} && git push -u origin main",
                config_dir.display()
            );
        }
    }

    Ok(())
}

/// Clone existing arch-config repository
pub fn clone(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Clone arch-config Repository ===".blue());
    println!();

    let config_dir = &paths.config_dir;

    // Check if arch-config exists
    if config_dir.exists() {
        println!("{}", "arch-config directory already exists".yellow());
        print!("Backup and replace? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    // Get repository URL
    println!("Enter your arch-config repository URL:");
    println!("Examples:");
    println!("  HTTPS: https://github.com/username/arch-config.git");
    println!("  SSH:   git@github.com:username/arch-config.git");
    print!("URL: ");
    io::stdout().flush()?;

    let mut repo_url = String::new();
    io::stdin().read_line(&mut repo_url)?;
    let repo_url = repo_url.trim();

    if !validate_git_url(repo_url) {
        anyhow::bail!("Invalid URL format");
    }

    println!();
    println!("{}", "Cloning repository...".blue());
    println!();

    // Backup existing if present
    let backup_dir = if config_dir.exists() {
        let backup_path = format!(
            "{}.backup.{}",
            config_dir.display(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
        );
        println!("{} Backing up existing arch-config to:", "→".blue());
        println!("  {}", backup_path);
        fs::rename(config_dir, &backup_path)?;
        Some(backup_path)
    } else {
        None
    };

    // Clone repository
    let spinner = crate::progress::create_spinner(&format!("Cloning from {}...", repo_url));
    let result = Command::new("git")
        .args(["clone", repo_url, &config_dir.display().to_string()])
        .status();

    match result {
        Ok(status) if status.success() => {
            spinner.finish_with_message("✓ Repository cloned successfully");
        }
        _ => {
            spinner.finish_with_message("✗ Clone failed");
            // Restore backup if clone failed
            if let Some(backup) = backup_dir {
                fs::rename(&backup, config_dir)?;
            }
            anyhow::bail!("Failed to clone repository");
        }
    }

    env::set_current_dir(config_dir)?;

    // Auto-detect hostname
    let current_hostname = get_hostname();
    println!();
    println!(
        "{} Configuring for host: {}",
        "→".blue(),
        current_hostname.green()
    );

    // Create config.yaml if it doesn't exist
    let config_file = Path::new("config.yaml");
    if !config_file.exists() {
        println!("{} Creating config.yaml (not found in repo)...", "→".blue());
        let config_content = format!(
            r#"# Main configuration for mdots declarative package management
# Edit this file to customize your system configuration

# Hostname of this machine
host: {}

# List of enabled modules
# Enable modules with: mdots module enable <module-name>
enabled_modules: []

# Additional packages not in any module
additional_packages: []

# Automatically remove unmanaged packages during sync
auto_prune: false

# Flatpak installation scope: "user" (default) or "system"
flatpak_scope: user
"#,
            current_hostname
        );
        fs::write(config_file, config_content)?;
        println!("{} Created config.yaml", "✓".green());
    } else {
        // Update existing config.yaml with current hostname
        // For now, just notify the user
        println!(
            "{} config.yaml exists - please update host field to: {}",
            "→".blue(),
            current_hostname
        );
    }

    // Check if host file exists (check new location first, then old location)
    let new_host_file = format!("hosts/{}.yaml", current_hostname);
    let new_host_path = Path::new(&new_host_file);
    let old_host_file = format!("packages/hosts/{}.yaml", current_hostname);
    let old_host_path = Path::new(&old_host_file);

    let (host_file, host_exists) = if new_host_path.exists() {
        (new_host_file, true)
    } else if old_host_path.exists() {
        // Found in old location, we'll use this existing file
        (old_host_file, true)
    } else {
        // Doesn't exist, create in new location
        (new_host_file, false)
    };

    let host_path = Path::new(&host_file);

    if host_exists {
        println!(
            "{} Host configuration already exists: {}",
            "✓".yellow(),
            host_file
        );
        println!("{} Using existing configuration", "→".blue());
    } else {
        // Ask for description
        println!();
        print!("Describe this machine (optional, e.g., 'Gaming Desktop'): ");
        io::stdout().flush()?;
        let mut machine_desc = String::new();
        io::stdin().read_line(&mut machine_desc)?;
        let machine_desc = machine_desc.trim();
        let machine_desc = if machine_desc.is_empty() {
            current_hostname.clone()
        } else {
            machine_desc.to_string()
        };

        println!("{} Creating host-specific configuration...", "→".blue());

        // Ensure hosts directory exists (new location)
        fs::create_dir_all("hosts")?;

        let host_content = format!(
            r#"# Host-specific packages for {}
# {}

description: Packages specific to {}

# Packages to install only on this host
packages: []

# Packages to exclude from base or modules on this host
exclude: []
"#,
            current_hostname, machine_desc, current_hostname
        );
        fs::write(host_path, host_content)?;
    }

    // Commit changes
    println!("{} Committing host configuration...", "→".blue());
    run_git_command(&["add", "."])?;
    let commit_msg = format!("Add {} host configuration", current_hostname);
    let _ = run_git_command(&["commit", "-m", &commit_msg]);

    // Push changes
    println!("{} Pushing changes...", "→".blue());
    match run_git_command(&["push"]) {
        Ok(_) => {
            println!();
            println!(
                "{}",
                "✓ arch-config cloned and configured successfully!".green()
            );
            println!();
            println!("Next steps:");
            println!("  1. Review: {}", host_file);
            println!("  2. Run: mdots module list");
            println!("  3. Run: mdots sync");
        }
        Err(_) => {
            println!();
            println!("{}", "Cloned successfully, but push failed.".yellow());
            println!("Fix authentication, then run: mdots repo push");
        }
    }

    Ok(())
}

/// Push changes to remote repository
pub fn push(paths: &ConfigPaths) -> Result<()> {
    let config_dir = &paths.config_dir;
    env::set_current_dir(config_dir)?;

    if !is_git_repo() {
        println!("{}", "Not a git repository".red());
        println!("Run 'mdots repo init' first");
        anyhow::bail!("Not a git repository");
    }

    println!("{}", "Pushing arch-config changes...".blue());
    println!();

    // Check for uncommitted changes
    let has_changes = !Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()?
        .success();

    if has_changes {
        println!("{} Changes detected", "→".blue());
        run_git_command(&["status", "--short"])?;
        println!();

        // Get commit message
        println!();
        println!("Commit message (or press Enter for default):");
        print!("> ");
        io::stdout().flush()?;

        let mut commit_msg = String::new();
        io::stdin().read_line(&mut commit_msg)?;
        let commit_msg = commit_msg.trim();

        let commit_msg = if commit_msg.is_empty() {
            let hostname = get_hostname();
            let now = chrono::Local::now();
            format!(
                "Update arch-config from {} - {}",
                hostname,
                now.format("%Y-%m-%d %H:%M")
            )
        } else {
            commit_msg.to_string()
        };

        println!();
        println!("{} Staging changes...", "→".blue());
        run_git_command(&["add", "."])?;

        println!("{} Committing...", "→".blue());
        run_git_command(&["commit", "-m", &commit_msg])?;
    } else {
        println!("{}", "No changes to commit".green());
    }

    // Check if there are commits to push
    let commits_ahead = get_commits_ahead()?;

    if commits_ahead == 0 {
        println!("{}", "Already up to date with remote".green());
        return Ok(());
    } else {
        println!("{} {} commit(s) to push", "→".blue(), commits_ahead);
    }

    // Push to remote
    let spinner = crate::progress::create_spinner("Pushing to remote...");
    match run_git_command(&["push"]) {
        Ok(_) => {
            spinner.finish_with_message("✓ Changes pushed successfully");
        }
        Err(e) => {
            spinner.finish_with_message("✗ Push failed");
            return Err(e);
        }
    }

    Ok(())
}

/// Pull updates from remote repository
pub fn pull(paths: &ConfigPaths) -> Result<()> {
    let config_dir = &paths.config_dir;
    env::set_current_dir(config_dir)?;

    if !is_git_repo() {
        println!("{}", "Not a git repository".red());
        println!("Run 'mdots repo init' or 'mdots repo clone' first");
        anyhow::bail!("Not a git repository");
    }

    println!("{}", "Pulling updates from remote...".blue());
    println!();

    let spinner = crate::progress::create_spinner("Fetching changes...");
    match run_git_command(&["pull"]) {
        Ok(_) => {
            spinner.finish_with_message("✓ Updates pulled successfully");
            println!();
            println!("Run 'mdots sync' to install any new packages");
        }
        Err(e) => {
            spinner.finish_with_message("✗ Pull failed");
            println!();
            return Err(e);
        }
    }

    Ok(())
}

/// Show repository status
pub fn status(paths: &ConfigPaths) -> Result<()> {
    let config_dir = &paths.config_dir;
    env::set_current_dir(config_dir)?;

    if !is_git_repo() {
        println!("{}", "Not a git repository".red());
        println!("Run 'mdots repo init' or 'mdots repo clone' first");
        anyhow::bail!("Not a git repository");
    }

    println!("{}", "Repository Status:".blue());
    println!();

    // Show remote
    let remote_url = get_remote_url().unwrap_or_else(|| "No remote configured".to_string());
    println!("Remote: {}", remote_url);

    // Show current branch
    let branch = get_current_branch().unwrap_or_else(|| "unknown".to_string());
    println!("Branch: {}", branch);
    println!();

    // Show git status
    run_git_command(&["status"])?;

    println!();
    println!(
        "{} Use 'mdots repo push' to save your changes",
        "Tip:".blue()
    );

    Ok(())
}

// Helper functions

fn is_git_repo() -> bool {
    Path::new(".git").exists()
}

fn get_remote_url() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        None
    }
}

fn get_current_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        None
    }
}

fn get_commits_ahead() -> Result<usize> {
    let output = Command::new("git")
        .args(["rev-list", "--count", "@{u}..HEAD"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let count_str = String::from_utf8(output.stdout)?;
            Ok(count_str.trim().parse().unwrap_or(0))
        }
        _ => Ok(0),
    }
}

fn validate_git_url(url: &str) -> bool {
    url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("git@")
        || url.starts_with("ssh://")
}

fn run_git_command(args: &[&str]) -> Result<()> {
    let status = Command::new("git").args(args).status()?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Git command failed: git {}", args.join(" "))
    }
}

fn get_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}
