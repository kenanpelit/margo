use anyhow::{Context, Result};
use colored::*;
use std::process::{Command, Stdio};

use crate::config::{ConfigPaths, PackageManagerType, load_config};

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

    // List every package with the helper and pipe it into fzf. Both run as
    // argv — the configured helper name never passes through `sh -c`.
    let output = match pm_type {
        PackageManagerType::Pacman => {
            let helper = backend.package_info_command().to_string();
            let (program, args) = list_command(&helper);
            let mut list = Command::new(program)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| format!("Failed to run {helper}"))?;
            let list_out = list
                .stdout
                .take()
                .context("Failed to capture the package list")?;
            let output = Command::new("fzf")
                .args(fzf_args(&helper))
                .stdin(Stdio::from(list_out))
                .output()
                .context("Failed to run fzf")?;
            let _ = list.wait();
            output
        }
    };

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

/// Program and arguments that list every installable package name.
fn list_command(helper: &str) -> (&str, [&'static str; 2]) {
    (helper, ["-Slq", "--color=never"])
}

/// Single-quote `s` for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// fzf arguments. `--preview` is the one place fzf itself runs a shell
/// command, so the helper name is quoted there.
fn fzf_args(helper: &str) -> Vec<String> {
    let preview = format!("{} -Si {{1}} --color=never", sh_quote(helper));
    [
        "--multi",
        "--ansi",
        "--preview",
        &preview,
        "--preview-window=right:60%:wrap",
        "--header=→ Interactive package search\nℹ Use TAB to select multiple packages, ENTER to install",
        "--prompt=Search packages > ",
        "--height=100%",
        "--border=rounded",
        "--border-label= mdots search ",
        "--border-label-pos=2",
        "--color=border:blue,label:cyan",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_name_is_an_argv_element_not_shell_text() {
        // A configured aur_helper used to be spliced into `sh -c`; a value with
        // shell syntax would have run as a command.
        let evil = "yay; touch pwned";
        let (program, args) = list_command(evil);
        assert_eq!(program, evil);
        assert_eq!(args, ["-Slq", "--color=never"]);
    }

    #[test]
    fn fzf_preview_quotes_the_helper() {
        let args = fzf_args("my helper'; touch pwned #");
        let preview = args
            .iter()
            .position(|a| a == "--preview")
            .map(|i| args[i + 1].clone())
            .unwrap();
        assert_eq!(
            preview,
            r#"'my helper'\''; touch pwned #' -Si {1} --color=never"#
        );
        assert!(args.iter().any(|a| a == "--multi"));
    }

    #[test]
    fn sh_quote_plain_word() {
        assert_eq!(sh_quote("paru"), "'paru'");
    }
}
