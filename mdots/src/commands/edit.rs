use anyhow::{Context, Result};
use colored::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

use crate::config::{load_config, resolve_editor, ConfigPaths};

/// Discover all config files (YAML or Lua) in arch-config directory
fn discover_config_files(paths: &ConfigPaths) -> Result<Vec<PathBuf>> {
    let mut config_files = Vec::new();

    // Walk the config directory recursively
    for entry in WalkDir::new(&paths.config_dir)
        .max_depth(10) // Reasonable limit to prevent infinite loops
        .follow_links(false) // Don't follow symlinks to avoid loops
        .into_iter()
        .filter_entry(|e| {
            // Exclude state directory and its contents
            // Exclude .git directory
            // Exclude scripts directory (not config files)
            let path = e.path();
            let file_name = path.file_name().unwrap_or_default();

            // Skip excluded directories
            if path.is_dir() {
                let name_str = file_name.to_string_lossy();
                if name_str == "state"
                    || name_str == ".git"
                    || name_str == "scripts"
                    || name_str == "wallpapers" // User assets, not config
                    || name_str == "dotfiles"
                // User dotfiles, not dcli config
                {
                    return false;
                }
            }

            true
        })
    {
        let entry = entry?;
        let path = entry.path();

        // Only include YAML or Lua files
        if path.is_file()
            && path
                .extension()
                .map(|s| s == "yaml" || s == "yml" || s == "lua")
                .unwrap_or(false)
        {
            config_files.push(path.to_path_buf());
        }
    }

    // Sort for consistent ordering
    config_files.sort();

    if config_files.is_empty() {
        anyhow::bail!(
            "No config files (YAML or Lua) found in {}",
            paths.config_dir.display()
        );
    }

    Ok(config_files)
}

/// Convert absolute path to relative path from config_dir for display
fn make_relative_display_path(path: &Path, config_dir: &PathBuf) -> String {
    path.strip_prefix(config_dir)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Run interactive file selector with fzf
fn select_file_with_fzf(config_files: &[PathBuf], config_dir: &PathBuf) -> Result<PathBuf> {
    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!("fzf is not installed. Please install fzf to use the edit command.");
    }

    // Build preview command
    // Try bat with fallback (bat is prettier but not always installed)
    let preview_cmd = if which::which("bat").is_ok() {
        format!(
            "bat --style=numbers --color=always --line-range=:100 '{}/{{}}'",
            config_dir.display()
        )
    } else {
        format!("head -n 100 '{}/{{}}'", config_dir.display())
    };

    // Spawn fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--preview",
            &preview_cmd,
            "--preview-window=right:60%:wrap",
            "--header=→ Select a file to edit (ESC to cancel)\nℹ Use arrow keys to select, ENTER to open",
            "--prompt=Search files > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= dcli edit ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
            "--no-multi", // Single selection only
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn fzf")?;

    // Write file list to fzf (relative paths for cleaner display)
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for file in config_files {
            let display_path = make_relative_display_path(file, config_dir);
            writeln!(stdin, "{}", display_path)?;
        }
    }

    // Wait for user selection
    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        anyhow::bail!("File selection cancelled");
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        anyhow::bail!("No file selected");
    }

    // Convert relative path back to absolute
    let selected_path = config_dir.join(&selected);

    // Validate the file still exists (paranoid check)
    if !selected_path.exists() {
        anyhow::bail!("Selected file not found: {}", selected_path.display());
    }

    Ok(selected_path)
}

/// Open a file in the configured editor
fn open_in_editor(file_path: &PathBuf, editor: &str) -> Result<()> {
    println!();
    println!("{} Opening: {}", "→".blue(), file_path.display());
    println!("{} Editor: {}", "→".blue(), editor);
    println!();

    // Parse editor command (handle args like "code --wait")
    let editor_parts: Vec<&str> = editor.split_whitespace().collect();
    let (editor_cmd, editor_args) = editor_parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Invalid editor command"))?;

    // Check if editor exists in PATH
    if which::which(editor_cmd).is_err() {
        anyhow::bail!(
            "Editor '{}' not found in PATH. Please check your editor configuration.",
            editor_cmd
        );
    }

    // Spawn editor with full terminal control
    let mut editor = Command::new(editor_cmd);
    editor.args(editor_args).arg(file_path);
    let status = crate::process::status_inherited(&mut editor)
        .context(format!("Failed to execute editor: {}", editor_cmd))?;

    if !status.success() {
        anyhow::bail!(
            "Editor exited with non-zero status: {:?}",
            status.code().unwrap_or(-1)
        );
    }

    println!();
    println!("{} File closed", "✓".green());

    Ok(())
}

/// Run the edit command - interactive file selector and editor
pub fn run(paths: &ConfigPaths) -> Result<()> {
    // Load config to resolve editor
    let config = load_config(paths)?;
    let editor = resolve_editor(&config)?;

    // Discover config files
    let config_files = discover_config_files(paths)?;

    println!("{}", "=== Edit Configuration Files ===".blue().bold());
    println!();
    println!("Found {} config files", config_files.len());
    println!("Editor: {}", editor.cyan());
    println!();

    // Select file interactively
    let selected_file = select_file_with_fzf(&config_files, &paths.config_dir)?;

    // Open in editor
    open_in_editor(&selected_file, &editor)?;

    // Optional: Suggest validation after edit
    println!();
    println!(
        "{}",
        "Tip: Run 'dcli validate' to check for configuration errors.".yellow()
    );

    Ok(())
}
