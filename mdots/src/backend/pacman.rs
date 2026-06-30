use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Command;

use super::PkgBackend;

/// Pacman/AUR helper backend for Arch-based distros
pub struct PacmanBackend {
    aur_helper: String,
}

impl PacmanBackend {
    pub fn new(aur_helper: String) -> Self {
        Self { aur_helper }
    }
}

impl PkgBackend for PacmanBackend {
    fn install_packages_batch(&self, packages: &[&str]) -> Result<bool> {
        let status = Command::new(&self.aur_helper)
            .args(["-S", "--needed", "--noconfirm"])
            .args(packages)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to install packages")?;

        Ok(status.success())
    }

    fn install_interactive(&self, package: &str) -> Result<bool> {
        let status = Command::new(&self.aur_helper)
            .args(["-S", package])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context(format!("Failed to install package: {}", package))?;

        Ok(status.success())
    }

    fn remove_packages_batch(&self, packages: &[&str]) -> Result<bool> {
        let status = Command::new(&self.aur_helper)
            .args(["-R", "--noconfirm"])
            .args(packages)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to remove packages")?;

        Ok(status.success())
    }

    fn remove_interactive(&self, package: &str) -> Result<bool> {
        let status = Command::new(&self.aur_helper)
            .args(["-R", package])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context(format!("Failed to remove package: {}", package))?;

        Ok(status.success())
    }

    fn refresh_db(&self) -> Result<()> {
        let status = Command::new("sudo")
            .args(["pacman", "-Sy"])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to refresh package database")?;

        if !status.success() {
            anyhow::bail!("Package database refresh failed");
        }

        Ok(())
    }

    fn system_update(&self, devel: bool) -> Result<bool> {
        let mut args = vec!["-Syu"];
        if devel {
            args.push("--devel");
        }

        let status = Command::new(&self.aur_helper)
            .args(&args)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context(format!(
                "Failed to run {} {}",
                self.aur_helper,
                args.join(" ")
            ))?;

        Ok(status.success())
    }

    fn get_installed_packages(&self) -> Result<HashMap<String, String>> {
        let output = Command::new("pacman")
            .args(["-Q"])
            .output()
            .context("Failed to run pacman -Q")?;

        if !output.status.success() {
            anyhow::bail!("pacman -Q failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut packages = HashMap::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 {
                packages.insert(parts[0].to_string(), parts[1].to_string());
            }
        }

        Ok(packages)
    }

    fn get_explicit_packages(&self) -> Result<Vec<String>> {
        let output = Command::new("pacman")
            .args(["-Qeq"])
            .output()
            .context("Failed to run pacman -Qeq")?;

        if !output.status.success() {
            anyhow::bail!("pacman -Qeq failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|s| s.to_string()).collect())
    }

    fn get_all_packages(&self) -> Result<Vec<String>> {
        let output = Command::new("pacman")
            .args(["-Qq"])
            .output()
            .context("Failed to run pacman -Qq")?;

        if !output.status.success() {
            anyhow::bail!("pacman -Qq failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|s| s.to_string()).collect())
    }

    fn get_package_version(&self, package: &str) -> Result<Option<String>> {
        let output = Command::new("pacman").args(["-Q", package]).output();

        match output {
            Ok(out) if out.status.success() => {
                let result = String::from_utf8_lossy(&out.stdout);
                let parts: Vec<&str> = result.split_whitespace().collect();
                if parts.len() >= 2 {
                    Ok(Some(parts[1].to_string()))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn is_available(&self, package: &str) -> bool {
        Command::new("pacman")
            .args(["-Si", package])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn check_package_exists(&self, package: &str) -> bool {
        // Check official repos first
        if self.is_available(package) {
            return true;
        }

        // Check via AUR helper
        Command::new("timeout")
            .args(["5", &self.aur_helper, "-Si", package])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn package_info_command(&self) -> &str {
        // Returns the base command; callers append the package name
        // For fzf preview: "{aur_helper} -Si {1} --color=never"
        &self.aur_helper
    }
}
