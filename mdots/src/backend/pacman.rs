use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::process::Command;

use super::{PackageUpdate, PkgBackend};

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

    fn check_updates(&self) -> Result<Vec<PackageUpdate>> {
        let mut updates = Vec::new();
        let mut seen = HashSet::new();
        let mut add = |text: &[u8], updates: &mut Vec<PackageUpdate>| {
            for update in parse_pacman_updates(&String::from_utf8_lossy(text)) {
                if seen.insert(update.name.clone()) {
                    updates.push(update);
                }
            }
        };

        // `checkupdates` (pacman-contrib) checks against a fresh temporary copy of the sync databases, without root and without
        // touching the real ones. It exits 2 when there is nothing to update and 1 on failure (no network, say); then the
        // locally cached databases (`pacman -Qu`) are the fallback.
        let checked_official = which::which("checkupdates").is_ok()
            && Command::new("checkupdates")
                .output()
                .ok()
                .filter(|out| out.status.success() || out.status.code() == Some(2))
                .map(|out| add(&out.stdout, &mut updates))
                .is_some();
        if !checked_official {
            let output = Command::new("pacman")
                .args(["-Qu"])
                .output()
                .context("Failed to run pacman -Qu")?;
            add(&output.stdout, &mut updates);
        }

        // AUR packages are not covered by pacman/checkupdates; `-Qua` lists only those.
        if let Ok(output) = Command::new(&self.aur_helper).args(["-Qua"]).output() {
            add(&output.stdout, &mut updates);
        }

        Ok(updates)
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

/// Parses the `name current -> available` lines of `checkupdates`, `pacman -Qu` and `<aur helper> -Qua`; anything else
/// (warnings, `[ignored]` notes) is skipped.
fn parse_pacman_updates(text: &str) -> Vec<PackageUpdate> {
    text.lines()
        .filter_map(|line| {
            let (left, available) = line.trim().split_once(" -> ")?;
            let (name, current) = left.trim().split_once(' ')?;
            let available = available.split_whitespace().next()?;
            if name.is_empty() || name.contains(':') {
                return None;
            }
            Some(PackageUpdate {
                name: name.to_string(),
                current: Some(current.trim().to_string()),
                available: Some(available.to_string()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_lines() {
        let updates = parse_pacman_updates(
            "linux 6.14.1.arch1-1 -> 6.14.2.arch1-1\nfirefox 138.0-1 -> 139.0-1\n",
        );
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "linux");
        assert_eq!(updates[0].current.as_deref(), Some("6.14.1.arch1-1"));
        assert_eq!(updates[0].available.as_deref(), Some("6.14.2.arch1-1"));
        assert_eq!(updates[1].name, "firefox");
    }

    #[test]
    fn keeps_the_version_of_an_ignored_package() {
        let updates = parse_pacman_updates("linux 6.14.1-1 -> 6.14.2-1 [ignored]\n");
        assert_eq!(updates[0].available.as_deref(), Some("6.14.2-1"));
    }

    #[test]
    fn skips_lines_that_are_not_updates() {
        assert!(parse_pacman_updates("").is_empty());
        assert!(
            parse_pacman_updates("error: no updates\n:: Looking for devel upgrades...\n")
                .is_empty()
        );
        assert!(parse_pacman_updates("warning: a -> b\n").is_empty());
    }
}
