//! `p <query>` — search official Arch + AUR repos.
//!
//! Subprocess-based search via `pacman -Ss` (always) and
//! `yay -Ss` / `paru -Ss` (when an AUR helper is on PATH). Each
//! hit becomes one row — activating it spawns the user's
//! terminal with `<helper> -S <pkg>` so they can confirm the
//! install interactively. We don't wrap install in a yes/no
//! popover because pacman's confirmation flow already handles
//! conflicts/deps/replacements that a generic popover can't
//! represent.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

pub struct ArchLinuxPkgsProvider {
    /// AUR helper found on PATH (`yay` / `paru` / `pamac`). When
    /// `None`, only `pacman -Ss` runs and install spawns the
    /// terminal with plain `sudo pacman -S`.
    helper: Option<String>,
    /// Terminal binary used to spawn install confirmations. We
    /// pick `$TERMINAL` env when set, else `kitty`, `alacritty`,
    /// `foot` in that order. Falls back to `kitty` and lets the
    /// shell discover failures.
    terminal: String,
}

impl ArchLinuxPkgsProvider {
    pub fn new() -> Self {
        let helper = ["yay", "paru", "pamac"]
            .iter()
            .find(|h| which_exists(h))
            .map(|h| h.to_string());
        let terminal = std::env::var("TERMINAL")
            .ok()
            .or_else(|| {
                ["kitty", "alacritty", "foot", "wezterm"]
                    .iter()
                    .find(|t| which_exists(t))
                    .map(|t| t.to_string())
            })
            .unwrap_or_else(|| "kitty".into());
        Self { helper, terminal }
    }
}

impl Default for ArchLinuxPkgsProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn which_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// One package row.
struct Hit {
    /// `repo/name` (e.g. `extra/firefox` or `aur/spotify`).
    full: String,
    /// Version string (right after the name).
    version: String,
    /// Description line (the indented second line of pacman/yay
    /// output).
    description: String,
    /// `[installed]` marker present in the output.
    installed: bool,
}

/// Parse `pacman -Ss` / `yay -Ss` style output. Each match is
/// two lines: a header (`repo/name ver [installed]`) and an
/// indented description.
fn parse_search(stdout: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut iter = stdout.lines().peekable();
    while let Some(header) = iter.next() {
        let header = header.trim_end();
        if header.is_empty() {
            continue;
        }
        // Description lines start with whitespace; if the next
        // header arrives before a description, this entry has
        // none.
        let description = match iter.peek() {
            Some(next) if next.starts_with(' ') || next.starts_with('\t') => {
                let s = iter.next().unwrap().trim().to_string();
                s
            }
            _ => String::new(),
        };
        let installed = header.contains("[installed");
        // Header form: "repo/name version <tags>"
        let mut tokens = header.split_whitespace();
        let full = tokens.next().unwrap_or("").to_string();
        let version = tokens.next().unwrap_or("").to_string();
        if !full.is_empty() {
            hits.push(Hit {
                full,
                version,
                description,
                installed,
            });
        }
    }
    hits
}

fn search_packages(query: &str, helper: Option<&str>) -> Vec<Hit> {
    let mut hits = Vec::new();
    // Prefer the AUR helper because it also searches official
    // repos in one shot; fall back to pacman when no helper is
    // installed.
    let bin = helper.unwrap_or("pacman");
    if let Ok(out) = Command::new(bin).args(["-Ss", query]).output()
        && out.status.success()
    {
        hits.extend(parse_search(&String::from_utf8_lossy(&out.stdout)));
    }
    hits
}

impl Provider for ArchLinuxPkgsProvider {
    fn name(&self) -> &str {
        "Arch packages"
    }

    fn category(&self) -> &str {
        "Search"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "p"
            || q.starts_with("p ")
            || q == "pacman"
            || q.starts_with("pacman ")
            || q == "aur"
            || q.starts_with("aur ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "archpkgs:palette".into(),
            name: "p <query>".into(),
            description: format!(
                "Search Arch + AUR ({}install via terminal)",
                if self.helper.is_some() {
                    "AUR helper found — "
                } else {
                    ""
                }
            ),
            icon: "package-x-generic-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Arch packages".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        // Strip prefix to get the actual search term.
        let term = if let Some(rest) = q.strip_prefix("pacman ") {
            rest
        } else if let Some(rest) = q.strip_prefix("aur ") {
            rest
        } else if let Some(rest) = q.strip_prefix("p ") {
            rest
        } else {
            return Vec::new();
        };
        let term = term.trim();
        // Searches with <3 chars produce too much noise from
        // pacman — wait for a real query.
        if term.len() < 3 {
            return Vec::new();
        }

        let hits = search_packages(term, self.helper.as_deref());
        let install_cmd = self.helper.clone().unwrap_or_else(|| "pacman".into());
        let terminal = self.terminal.clone();

        hits.into_iter()
            .take(80)
            .enumerate()
            .map(|(idx, hit)| {
                let pkg = hit.full.clone();
                let pkg_short = pkg.split('/').next_back().unwrap_or(&pkg).to_string();
                let term_clone = terminal.clone();
                let install_cmd_clone = install_cmd.clone();
                let pkg_label = pkg.clone();
                let label = if hit.installed {
                    format!("{}  {}  [installed]", hit.full, hit.version)
                } else {
                    format!("{}  {}", hit.full, hit.version)
                };
                let description = if hit.description.is_empty() {
                    "Press Enter to install in a terminal".into()
                } else {
                    hit.description.clone()
                };
                LauncherItem {
                    id: format!("archpkgs:{}", hit.full),
                    name: label,
                    description,
                    icon: if hit.installed {
                        "emblem-default-symbolic".into()
                    } else {
                        "package-x-generic-symbolic".into()
                    },
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "Arch packages".into(),
                    usage_key: Some(format!("archpkgs:{}", hit.full)),
                    on_activate: Rc::new(move || {
                        spawn_install(&term_clone, &install_cmd_clone, &pkg_short);
                        toast("Installing", pkg_label.clone());
                    }),
                }
            })
            .collect()
    }
}

/// Spawn `<terminal> -e <helper> -S <pkg>` so the user sees the
/// confirmation prompt. When using bare pacman we wrap with
/// `sudo`; AUR helpers handle their own privilege escalation.
fn spawn_install(terminal: &str, helper: &str, pkg: &str) {
    let inner: Vec<String> = if helper == "pacman" {
        vec!["sudo".into(), "pacman".into(), "-S".into(), pkg.into()]
    } else {
        vec![helper.into(), "-S".into(), pkg.into()]
    };

    // `kitty` / `alacritty` / `foot` all accept `<term> -e <cmd>
    // <args...>`. wezterm wants `start --`, so handle it
    // specially.
    let result = if terminal == "wezterm" {
        let mut cmd = Command::new(terminal);
        cmd.arg("start").arg("--");
        for arg in &inner {
            cmd.arg(arg);
        }
        cmd.spawn()
    } else {
        let mut cmd = Command::new(terminal);
        cmd.arg("-e");
        for arg in &inner {
            cmd.arg(arg);
        }
        cmd.spawn()
    };
    if let Err(err) = result {
        tracing::warn!(?err, terminal, pkg, "archpkgs install spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = ArchLinuxPkgsProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn p_prefix_requires_min_3_chars() {
        let p = ArchLinuxPkgsProvider::new();
        assert!(p.search("p ab").is_empty());
        assert!(p.search("p").is_empty());
    }

    #[test]
    fn parse_search_handles_pacman_output() {
        let sample = "extra/firefox 145.0.1-1\n    Standalone web browser from mozilla.org\nextra/firefox-i18n-tr 145.0.1-1\n    Turkish language pack for Firefox\n";
        let hits = parse_search(sample);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].full, "extra/firefox");
        assert_eq!(hits[0].version, "145.0.1-1");
        assert!(!hits[0].installed);
        assert!(hits[0].description.contains("Standalone"));
    }

    #[test]
    fn parse_search_detects_installed_marker() {
        let sample = "extra/firefox 145.0.1-1 [installed]\n    Browser\n";
        let hits = parse_search(sample);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].installed);
    }
}
