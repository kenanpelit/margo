//! `mdots doctor` — read-only environment health check.
//!
//! Runs a series of probes and prints a pass/warn/fail report grouped by area.
//! Exits with code 1 if any check fails; warns do not fail.
//! The command never writes to disk, never panics, and never aborts early on
//! a single failed check.

use std::path::PathBuf;

use anyhow::Result;
use colored::*;

use crate::config::{load_config, ConfigPaths, PackageType};
use crate::secrets::{
    classify_secret_status, resolve_key_path, resolve_secret_target, secret_name, sops_available,
    SecretState,
};

// ─── Check result types ────────────────────────────────────────────────────────

/// The health of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    Warn,
    Fail,
}

/// A single check result.
#[derive(Debug, Clone)]
pub struct Check {
    pub area: &'static str,
    pub name: String,
    pub status: Health,
    pub detail: String,
}

impl Check {
    fn ok(area: &'static str, name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            area,
            name: name.into(),
            status: Health::Ok,
            detail: detail.into(),
        }
    }

    fn warn(area: &'static str, name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            area,
            name: name.into(),
            status: Health::Warn,
            detail: detail.into(),
        }
    }

    fn fail(area: &'static str, name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            area,
            name: name.into(),
            status: Health::Fail,
            detail: detail.into(),
        }
    }
}

// ─── Pure aggregation (unit-tested) ───────────────────────────────────────────

/// Summarise check results.
///
/// Returns `(ok_count, warn_count, fail_count, exit_code)`.
/// `exit_code` is `1` iff at least one check has `Health::Fail`; warns are
/// non-fatal.
pub fn summarize(checks: &[Check]) -> (usize, usize, usize, i32) {
    let ok = checks.iter().filter(|c| c.status == Health::Ok).count();
    let warn = checks.iter().filter(|c| c.status == Health::Warn).count();
    let fail = checks.iter().filter(|c| c.status == Health::Fail).count();
    let exit_code = if fail > 0 { 1 } else { 0 };
    (ok, warn, fail, exit_code)
}

// ─── Individual probes ─────────────────────────────────────────────────────────

fn check_config(paths: &ConfigPaths, out: &mut Vec<Check>) {
    const AREA: &str = "Config";

    if !paths.config_dir.exists() {
        out.push(Check::fail(
            AREA,
            "config dir",
            format!(
                "{} does not exist — run `mdots init`",
                paths.config_dir.display()
            ),
        ));
        return;
    }
    out.push(Check::ok(
        AREA,
        "config dir",
        format!("{} exists", paths.config_dir.display()),
    ));

    match load_config(paths) {
        Ok(config) => {
            // Report which file resolved
            let cfg_path = crate::config::resolve_config_path(paths)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| paths.config_file.display().to_string());
            out.push(Check::ok(
                AREA,
                "config file",
                format!("{} parses (host: {})", cfg_path, config.host),
            ));
        }
        Err(e) => {
            out.push(Check::fail(
                AREA,
                "config file",
                format!("failed to load: {}", e),
            ));
        }
    }
}

fn check_backend(config_opt: Option<&crate::config::Config>, out: &mut Vec<Check>) {
    const AREA: &str = "Backend";

    let config = match config_opt {
        Some(c) => c,
        None => {
            out.push(Check::fail(
                AREA,
                "package backend",
                "skipped — config did not load",
            ));
            return;
        }
    };

    // Resolve pacman binary
    match which::which("pacman") {
        Ok(path) => out.push(Check::ok(
            AREA,
            "pacman",
            format!("found at {}", path.display()),
        )),
        Err(_) => out.push(Check::fail(
            AREA,
            "pacman",
            "pacman not found in PATH (mdots requires Arch/pacman-based distro)",
        )),
    }

    // Resolve AUR helper. `resolve_aur_helper` can return Ok(name) for an
    // auto-detected helper that is not actually on PATH, so verify with `which`
    // and downgrade to a warning when the binary is missing.
    match crate::config::resolve_aur_helper(config) {
        Ok(helper) => match which::which(&helper) {
            Ok(path) => out.push(Check::ok(
                AREA,
                "AUR helper",
                format!("{} found at {}", helper, path.display()),
            )),
            Err(_) => out.push(Check::warn(
                AREA,
                "AUR helper",
                format!(
                    "{} configured but not in PATH — AUR packages will fail",
                    helper
                ),
            )),
        },
        Err(e) => out.push(Check::warn(
            AREA,
            "AUR helper",
            format!("not resolved: {} — AUR packages will fail", e),
        )),
    }
}

fn check_flatpak(config_opt: Option<&crate::config::Config>, out: &mut Vec<Check>) {
    const AREA: &str = "Flatpak";

    let config = match config_opt {
        Some(c) => c,
        None => return, // silently skip; config failure already reported
    };

    // Collect all packages, including from additional_packages
    let has_flatpak = config
        .packages
        .iter()
        .chain(config.additional_packages.iter())
        .any(|p| p.package_type() == PackageType::Flatpak);

    if !has_flatpak {
        // No flatpak packages declared — check not needed
        out.push(Check::ok(AREA, "flatpak", "no flatpak packages declared"));
        return;
    }

    match which::which("flatpak") {
        Ok(path) => out.push(Check::ok(
            AREA,
            "flatpak",
            format!("found at {} (flatpak packages declared)", path.display()),
        )),
        Err(_) => out.push(Check::warn(
            AREA,
            "flatpak",
            "flatpak packages declared but `flatpak` is not in PATH",
        )),
    }
}

fn check_secrets(
    paths: &ConfigPaths,
    config_opt: Option<&crate::config::Config>,
    out: &mut Vec<Check>,
) {
    const AREA: &str = "Secrets";

    let config = match config_opt {
        Some(c) => c,
        None => return,
    };

    if config.secrets.is_empty() {
        out.push(Check::ok(AREA, "secrets", "no secrets declared"));
        return;
    }

    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            out.push(Check::fail(
                AREA,
                "secrets",
                "HOME environment variable not set",
            ));
            return;
        }
    };

    let repo_root = &paths.config_dir;
    // When no `sops_key_path` is configured, sops falls back to its default
    // location (~/.config/sops/age/keys.txt — the same path `secrets keygen`
    // writes). Probe that rather than assuming the key is present.
    let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home)
        .unwrap_or_else(|| home.join(".config/sops/age/keys.txt"));
    let sops = sops_available();
    let key_available = key_path.exists();

    for entry in &config.secrets {
        let name = secret_name(entry);
        let source_exists = repo_root.join(&entry.source).exists();
        let target_exists = match resolve_secret_target(&entry.target, &home, repo_root) {
            Ok(t) => t.exists(),
            Err(_) => false,
        };

        let state = classify_secret_status(sops, source_exists, key_available, target_exists);

        let check = match state {
            SecretState::Decrypted => Check::ok(AREA, name, "decrypted and present on disk"),
            SecretState::Pending => Check::warn(
                AREA,
                name,
                "all prerequisites met but not yet decrypted — run `mdots secrets sync`",
            ),
            SecretState::SopsMissing => {
                Check::fail(AREA, name, "sops is not installed (install `sops`)")
            }
            SecretState::SourceMissing => Check::fail(
                AREA,
                name,
                format!("encrypted source not found: {}", entry.source),
            ),
            SecretState::KeyMissing => Check::fail(
                AREA,
                name,
                format!("age key not found at {}", key_path.display()),
            ),
        };
        out.push(check);
    }
}

fn check_lua(
    paths: &ConfigPaths,
    config_opt: Option<&crate::config::Config>,
    out: &mut Vec<Check>,
) {
    const AREA: &str = "Lua";

    let config = match config_opt {
        Some(c) => c,
        None => return,
    };

    let modules_dir = paths.modules_dir();
    let mut lua_checked = 0u32;

    for module_name in &config.enabled_modules {
        let lua_path = modules_dir.join(format!("{}.lua", module_name));
        // Also check directory-based modules (module_name/module.lua)
        let lua_dir_path = modules_dir.join(module_name).join("module.lua");

        let effective_path = if lua_path.exists() {
            lua_path
        } else if lua_dir_path.exists() {
            lua_dir_path
        } else {
            // Not a Lua module — skip (YAML/Nix modules are irrelevant here)
            continue;
        };

        lua_checked += 1;
        let result = crate::lua::validate_lua_module_detailed(&effective_path);

        if result.valid {
            out.push(Check::ok(
                AREA,
                format!("module/{}", module_name),
                format!("{} evaluates OK", effective_path.display()),
            ));
        } else {
            let msgs: Vec<String> = result.errors.iter().map(|e| e.message.clone()).collect();
            out.push(Check::fail(
                AREA,
                format!("module/{}", module_name),
                format!("{} — {}", effective_path.display(), msgs.join("; ")),
            ));
        }
    }

    if lua_checked == 0 {
        out.push(Check::ok(
            AREA,
            "Lua modules",
            "no enabled Lua modules to check",
        ));
    }
}

fn check_nix(config_opt: Option<&crate::config::Config>, out: &mut Vec<Check>) {
    const AREA: &str = "Nix";

    let config = match config_opt {
        Some(c) => c,
        None => return,
    };

    if !config.nix.home_manager_enabled {
        out.push(Check::ok(
            AREA,
            "nix",
            "home_manager_enabled is false — skipping nix checks",
        ));
        return;
    }

    match which::which("nix") {
        Ok(path) => out.push(Check::ok(
            AREA,
            "nix binary",
            format!("found at {}", path.display()),
        )),
        Err(_) => out.push(Check::warn(
            AREA,
            "nix binary",
            "home_manager_enabled = true but `nix` is not in PATH",
        )),
    }

    match which::which("home-manager") {
        Ok(path) => out.push(Check::ok(
            AREA,
            "home-manager",
            format!("found at {}", path.display()),
        )),
        Err(_) => out.push(Check::warn(
            AREA,
            "home-manager",
            "home_manager_enabled = true but `home-manager` is not in PATH",
        )),
    }
}

// ─── Output ───────────────────────────────────────────────────────────────────

fn print_report(checks: &[Check]) {
    // Determine column widths for alignment
    let name_width = checks
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(10)
        .min(40);

    let mut current_area: Option<&str> = None;

    for check in checks {
        if current_area != Some(check.area) {
            if current_area.is_some() {
                println!();
            }
            println!("{}", format!("[{}]", check.area).bold().blue());
            current_area = Some(check.area);
        }

        let (marker, detail_colored) = match check.status {
            Health::Ok => ("✓".green(), check.detail.normal()),
            Health::Warn => ("!".yellow(), check.detail.yellow()),
            Health::Fail => ("✗".red(), check.detail.red()),
        };

        println!(
            "  {} {:<width$}  {}",
            marker,
            check.name,
            detail_colored,
            width = name_width,
        );
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

/// Run every probe and return the collected check results, in report order,
/// without printing anything. Shared by the CLI [`run`] and the TUI doctor
/// overlay so both see exactly the same checks.
pub fn gather_checks(paths: &ConfigPaths) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();

    // 1. Config — must come first so subsequent checks can reuse the loaded Config
    check_config(paths, &mut checks);

    // Attempt to load the config for subsequent probes; failure is already
    // recorded as a Fail check above.
    let config_opt = load_config(paths).ok();

    // 2. Backend
    check_backend(config_opt.as_ref(), &mut checks);

    // 3. Flatpak
    check_flatpak(config_opt.as_ref(), &mut checks);

    // 4. Secrets
    check_secrets(paths, config_opt.as_ref(), &mut checks);

    // 5. Lua manifests
    check_lua(paths, config_opt.as_ref(), &mut checks);

    // 6. Nix / home-manager
    check_nix(config_opt.as_ref(), &mut checks);

    checks
}

/// Run all health checks, print the report, and return the exit code.
///
/// Returns `Ok(0)` when all checks pass (or only warn), `Ok(1)` when at least
/// one check fails. The caller is responsible for calling
/// `std::process::exit(code)` when appropriate.
pub fn run(paths: &ConfigPaths) -> Result<i32> {
    println!("{}", "=== mdots doctor ===".bold().blue());
    println!();

    let checks = gather_checks(paths);

    // Print grouped report
    print_report(&checks);

    // Summary line
    let (ok, warn, fail, exit_code) = summarize(&checks);
    println!();
    let summary = format!("Summary: {} passed, {} warned, {} failed", ok, warn, fail);
    if fail > 0 {
        println!("{}", summary.red().bold());
    } else if warn > 0 {
        println!("{}", summary.yellow().bold());
    } else {
        println!("{}", summary.green().bold());
    }

    Ok(exit_code)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(area: &'static str) -> Check {
        Check::ok(area, "test", "all good")
    }
    fn warn(area: &'static str) -> Check {
        Check::warn(area, "test", "something questionable")
    }
    fn fail(area: &'static str) -> Check {
        Check::fail(area, "test", "broken")
    }

    #[test]
    fn summarize_all_ok_returns_exit_zero() {
        let checks = vec![ok("A"), ok("B"), ok("C")];
        let (ok_c, warn_c, fail_c, code) = summarize(&checks);
        assert_eq!(ok_c, 3);
        assert_eq!(warn_c, 0);
        assert_eq!(fail_c, 0);
        assert_eq!(code, 0, "all-ok must exit 0");
    }

    #[test]
    fn summarize_any_fail_returns_exit_one() {
        let checks = vec![ok("A"), warn("B"), fail("C")];
        let (ok_c, warn_c, fail_c, code) = summarize(&checks);
        assert_eq!(ok_c, 1);
        assert_eq!(warn_c, 1);
        assert_eq!(fail_c, 1);
        assert_eq!(code, 1, "any fail must exit 1");
    }

    #[test]
    fn summarize_warns_only_does_not_fail() {
        let checks = vec![ok("A"), warn("B"), warn("C")];
        let (ok_c, warn_c, fail_c, code) = summarize(&checks);
        assert_eq!(ok_c, 1);
        assert_eq!(warn_c, 2);
        assert_eq!(fail_c, 0);
        assert_eq!(code, 0, "warns only must NOT exit 1");
    }

    #[test]
    fn summarize_empty_checks_is_ok() {
        let (ok_c, warn_c, fail_c, code) = summarize(&[]);
        assert_eq!(ok_c, 0);
        assert_eq!(warn_c, 0);
        assert_eq!(fail_c, 0);
        assert_eq!(code, 0);
    }

    #[test]
    fn summarize_multiple_fails_counted_correctly() {
        let checks = vec![fail("A"), fail("B"), ok("C")];
        let (_, _, fail_c, code) = summarize(&checks);
        assert_eq!(fail_c, 2);
        assert_eq!(code, 1);
    }

    #[test]
    fn summarize_counts_each_severity_bucket_independently() {
        // A mixed vector with all three severities must count each bucket
        // separately and still fail overall because a Fail is present.
        let checks = vec![ok("A"), ok("B"), warn("C"), fail("D")];
        let (ok_c, warn_c, fail_c, code) = summarize(&checks);
        assert_eq!(ok_c, 2, "two Ok checks");
        assert_eq!(warn_c, 1, "one Warn check");
        assert_eq!(fail_c, 1, "one Fail check");
        assert_eq!(code, 1, "presence of a Fail must exit 1");
    }
}
