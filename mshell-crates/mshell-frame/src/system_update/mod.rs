//! Shared system-update probing core, used by the SystemUpdate bar
//! pill and its panel menu. Ports the noctalia `arch-updater`: it
//! counts and lists pending updates split by source — official repo,
//! AUR, and Flatpak — each gated by a Settings toggle.
//!
//! Arch is the primary target (checkupdates + an AUR helper +
//! flatpak). On non-Arch systems the repo source degrades to a
//! count-only probe via dnf / apt (no per-package versions).

pub(crate) mod news;

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields,
};
use reactive_graph::traits::GetUntracked;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Where a pending update comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Source {
    Repo,
    Aur,
    Flatpak,
}

impl Source {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Source::Repo => "Official repos",
            Source::Aur => "AUR",
            Source::Flatpak => "Flatpak",
        }
    }

    #[allow(dead_code)] // used by the panel menu (Stage 2)
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Source::Repo => "package-x-generic-symbolic",
            Source::Aur => "system-software-install-symbolic",
            Source::Flatpak => "flatpak-symbolic",
        }
    }
}

/// One pending package update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UpdateEntry {
    pub(crate) source: Source,
    pub(crate) name: String,
    /// Currently-installed version (`None` when the probe can't
    /// report it, e.g. dnf/apt fallback).
    pub(crate) old_version: Option<String>,
    pub(crate) new_version: Option<String>,
}

/// Result of a probe across all enabled sources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UpdateReport {
    pub(crate) entries: Vec<UpdateEntry>,
    /// `Some` when a probe failed outright — surfaced in the pill
    /// tooltip + the panel header. Per-source soft failures (a
    /// missing AUR helper, no flatpak) are silent: that source just
    /// contributes nothing.
    pub(crate) error: Option<String>,
}

impl UpdateReport {
    pub(crate) fn total(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn count(&self, source: Source) -> usize {
        self.entries.iter().filter(|e| e.source == source).count()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── Disk cache (so a restart inside the interval doesn't re-probe) ─

#[derive(Serialize, Deserialize)]
struct UpdateCache {
    /// Unix seconds of the last successful probe.
    checked_at: u64,
    report: UpdateReport,
}

/// `~/.cache/mshell/system_update.json` (honours `$XDG_CACHE_HOME`).
fn cache_path() -> std::path::PathBuf {
    let base = margo_fs::xdg::cache_home();
    base.join("mshell").join("system_update.json")
}

/// Seconds since the Unix epoch (0 on a clock error).
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The cached report + the Unix-seconds timestamp of when it was taken,
/// if a cache file exists and parses.
pub(crate) fn load_cache() -> Option<(u64, UpdateReport)> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    let c: UpdateCache = serde_json::from_str(&raw).ok()?;
    Some((c.checked_at, c.report))
}

/// Persist the report stamped with the current time (best-effort).
pub(crate) fn save_cache(report: &UpdateReport) {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let cache = UpdateCache {
        checked_at: now_secs(),
        report: report.clone(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = std::fs::write(&path, json);
    }
}

/// Which sources to probe — mirrors the config toggles.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeConfig {
    pub(crate) repo: bool,
    pub(crate) aur: bool,
    pub(crate) flatpak: bool,
}

impl ProbeConfig {
    /// Read the live `bars.widgets.system_update` toggles. Each
    /// reactive Subfield accessor consumes `self`, so re-walk the
    /// chain per field.
    pub(crate) fn from_config() -> Self {
        Self {
            repo: config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_repo()
                .get_untracked(),
            aur: config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_aur()
                .get_untracked(),
            flatpak: config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_flatpak()
                .get_untracked(),
        }
    }
}

/// Probe every enabled source and merge into one report. Sources run
/// sequentially (they're short and avoid hammering mirrors in
/// parallel). A hard failure on the repo probe sets `error`; AUR /
/// flatpak failures degrade silently.
pub(crate) async fn probe(cfg: ProbeConfig) -> UpdateReport {
    let mut report = UpdateReport::default();

    if cfg.repo {
        match probe_repo().await {
            Ok(mut entries) => report.entries.append(&mut entries),
            Err(e) => report.error = Some(e),
        }
    }
    if cfg.aur
        && let Ok(mut entries) = probe_aur().await
    {
        report.entries.append(&mut entries);
    }
    if cfg.flatpak
        && let Ok(mut entries) = probe_flatpak().await
    {
        report.entries.append(&mut entries);
    }

    report
}

// ── Repo (Arch checkupdates, else dnf / apt) ────────────────────

async fn probe_repo() -> Result<Vec<UpdateEntry>, String> {
    if which("checkupdates").await {
        // pacman-contrib: refreshes a fake DB under /tmp (no sudo).
        // Output: `name oldver -> newver`. Exit 2 = up to date.
        let out = run("checkupdates", &[]).await?;
        return match out.code {
            0 => Ok(parse_arrow_list(&out.stdout, Source::Repo)),
            2 => Ok(Vec::new()),
            c => Err(probe_err("checkupdates", c, &out.stderr)),
        };
    }
    if which("dnf").await {
        // exit 100 = updates available, 0 = none.
        let out = run("dnf", &["check-update", "--refresh", "-q"]).await?;
        return match out.code {
            0 => Ok(Vec::new()),
            100 => Ok(parse_dnf_check_update(&out.stdout)),
            c => Err(probe_err("dnf", c, &out.stderr)),
        };
    }
    if which("apt").await {
        let out = run("apt", &["list", "--upgradable"]).await?;
        if out.code != 0 {
            return Err(probe_err("apt", out.code, &out.stderr));
        }
        return Ok(parse_apt_upgradable(&out.stdout));
    }
    // No repo backend — not an error, just nothing to report.
    Ok(Vec::new())
}

// ── AUR (paru / yay) ────────────────────────────────────────────

async fn probe_aur() -> Result<Vec<UpdateEntry>, String> {
    let helper = if which("paru").await {
        "paru"
    } else if which("yay").await {
        "yay"
    } else {
        return Ok(Vec::new()); // no helper → silently skip AUR
    };
    // `-Qua`: AUR-only upgrade list, `name oldver -> newver`.
    // Exit 1 = nothing to upgrade.
    let out = run(helper, &["-Qua"]).await?;
    match out.code {
        0 => Ok(parse_arrow_list(&out.stdout, Source::Aur)),
        1 => Ok(Vec::new()),
        _ => Ok(parse_arrow_list(&out.stdout, Source::Aur)), // best-effort
    }
}

// ── Flatpak ─────────────────────────────────────────────────────

async fn probe_flatpak() -> Result<Vec<UpdateEntry>, String> {
    if !which("flatpak").await {
        return Ok(Vec::new());
    }
    // Join the updatable remote list (appid, name, NEW version) with
    // the installed list (appid, OLD version) on the app id, yielding
    // `appid<TAB>name<TAB>new<TAB>old` per line.
    let script = "flatpak update --no-deploy --noninteractive >/dev/null 2>&1; \
         join -t'\t' -j1 \
           <(flatpak remote-ls --updates --columns=application,name,version 2>/dev/null | sort -t'\t' -k1,1) \
           <(flatpak list --columns=application,version 2>/dev/null | sort -t'\t' -k1,1)";
    let out = run("sh", &["-c", script]).await?;
    Ok(parse_flatpak_join(&out.stdout))
}

/// Parse the `appid<TAB>name<TAB>new<TAB>old` lines of the flatpak join.
fn parse_flatpak_join(stdout: &str) -> Vec<UpdateEntry> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut f = line.split('\t');
            let _appid = f.next();
            let name = f.next().unwrap_or("").trim();
            let new = f
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let old = f
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            UpdateEntry {
                source: Source::Flatpak,
                name: if name.is_empty() {
                    _appid.unwrap_or("").to_string()
                } else {
                    name.to_string()
                },
                old_version: old,
                new_version: new,
            }
        })
        .collect()
}

/// Package names from `dnf check-update -q` (exit 100): one
/// `name.arch  version  repo` line per update; indented lines are
/// continuations. The trailing `Obsoleting Packages` section repeats
/// packages from the main list, so parsing stops at its header.
fn parse_dnf_check_update(stdout: &str) -> Vec<UpdateEntry> {
    stdout
        .lines()
        .take_while(|l| l.trim() != "Obsoleting Packages")
        .filter(|l| !l.trim().is_empty() && !l.starts_with(' '))
        .filter_map(|l| l.split_whitespace().next())
        .map(|name| UpdateEntry {
            source: Source::Repo,
            name: name.to_string(),
            old_version: None,
            new_version: None,
        })
        .collect()
}

/// Package names from `apt list --upgradable`
/// (`name/suite version arch [upgradable from: old]`).
fn parse_apt_upgradable(stdout: &str) -> Vec<UpdateEntry> {
    stdout
        .lines()
        .filter(|l| l.contains("[upgradable from:"))
        .filter_map(|l| l.split('/').next())
        .map(|name| UpdateEntry {
            source: Source::Repo,
            name: name.to_string(),
            old_version: None,
            new_version: None,
        })
        .collect()
}

// ── Parsing + process helpers ───────────────────────────────────

/// Parse `name oldver -> newver` lines (pacman / AUR helper format).
fn parse_arrow_list(stdout: &str, source: Source) -> Vec<UpdateEntry> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        // IgnorePkg entries (`name old -> new [ignored]`) won't be
        // upgraded by `-Syu`, so they aren't pending updates.
        .filter(|l| !l.trim_end().ends_with("[ignored]"))
        .map(|line| {
            let (left, new) = match line.split_once(" -> ") {
                Some((l, r)) => (l.trim(), Some(r.trim().to_string())),
                None => (line.trim(), None),
            };
            let mut parts = left.split_whitespace();
            let name = parts.next().unwrap_or("").to_string();
            let old = parts.next().map(|s| s.to_string());
            UpdateEntry {
                source,
                name,
                old_version: old,
                new_version: new,
            }
        })
        .filter(|e| !e.name.is_empty())
        .collect()
}

struct CmdOut {
    code: i32,
    stdout: String,
    stderr: String,
}

async fn run(cmd: &str, args: &[&str]) -> Result<CmdOut, String> {
    match tokio::process::Command::new(cmd).args(args).output().await {
        Ok(o) => Ok(CmdOut {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        }),
        Err(e) => Err(format!("{cmd} spawn: {e}")),
    }
}

fn probe_err(cmd: &str, code: i32, stderr: &str) -> String {
    let msg = if stderr.is_empty() {
        format!("{cmd}: exit {code}")
    } else {
        format!("{cmd} exit {code}: {stderr}")
    };
    warn!(error = %msg, "system_update: probe failed");
    msg
}

pub(crate) async fn which(binary: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Upgrade: open a terminal running the full upgrade ───────────

/// Spawn the user's terminal running the upgrade for every enabled
/// source: an AUR helper (covers repo + AUR) or `sudo pacman -Syu`
/// / dnf / apt, then `flatpak update` when flatpak is enabled.
pub(crate) async fn launch_terminal_upgrade(cfg: ProbeConfig) {
    let Some(term) = detect_terminal().await else {
        warn!("system_update: no terminal emulator on PATH; upgrade ignored");
        return;
    };

    let mut steps: Vec<String> = Vec::new();
    if cfg.repo || cfg.aur {
        if which("paru").await {
            steps.push("paru -Syu".into());
        } else if which("yay").await {
            steps.push("yay -Syu".into());
        } else if which("pacman").await {
            steps.push("sudo pacman -Syu".into());
        } else if which("dnf").await {
            steps.push("sudo dnf upgrade".into());
        } else if which("apt").await {
            steps.push("sudo apt upgrade".into());
        }
    }
    if cfg.flatpak && which("flatpak").await {
        steps.push("flatpak update".into());
    }
    if steps.is_empty() {
        warn!("system_update: nothing to upgrade (no backend / all sources off)");
        return;
    }

    let script = format!(
        "{}; echo; echo \"[mshell] done — press Enter to close.\"; read",
        steps.join("; ")
    );
    let args = terminal_argv(term, script);
    if let Err(e) = tokio::process::Command::new(term).args(&args).spawn() {
        warn!(error = %e, term, "system_update: terminal spawn failed");
    }
}

/// Arguments that make `term` run `sh -c script`.
fn terminal_argv(term: &str, script: String) -> Vec<String> {
    match term {
        "kitty" => vec!["--".into(), "sh".into(), "-c".into(), script],
        "foot" => vec!["sh".into(), "-c".into(), script],
        "gnome-terminal" => vec!["--".into(), "sh".into(), "-c".into(), script],
        _ => vec!["-e".into(), "sh".into(), "-c".into(), script],
    }
}

async fn detect_terminal() -> Option<&'static str> {
    for term in [
        "kitty",
        "alacritty",
        "foot",
        "wezterm",
        "konsole",
        "gnome-terminal",
        "xterm",
    ] {
        if which(term).await {
            return Some(term);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: Source, name: &str, old: Option<&str>, new: Option<&str>) -> UpdateEntry {
        UpdateEntry {
            source,
            name: name.into(),
            old_version: old.map(String::from),
            new_version: new.map(String::from),
        }
    }

    #[test]
    fn arrow_list_parses_checkupdates_output() {
        let out = "linux 6.9.1.arch1-1 -> 6.9.2.arch1-1\n\
                   mesa 1:24.1.0-1 -> 1:24.1.1-1\n";
        assert_eq!(
            parse_arrow_list(out, Source::Repo),
            vec![
                entry(
                    Source::Repo,
                    "linux",
                    Some("6.9.1.arch1-1"),
                    Some("6.9.2.arch1-1")
                ),
                entry(Source::Repo, "mesa", Some("1:24.1.0-1"), Some("1:24.1.1-1")),
            ]
        );
    }

    #[test]
    fn arrow_list_tolerates_blank_lines_padding_and_missing_arrow() {
        let out = "\n   \n  paru-bin   2.0.3-1   ->   2.0.4-1  \nlonely\n";
        assert_eq!(
            parse_arrow_list(out, Source::Aur),
            vec![
                entry(Source::Aur, "paru-bin", Some("2.0.3-1"), Some("2.0.4-1")),
                entry(Source::Aur, "lonely", None, None),
            ]
        );
        assert!(parse_arrow_list("", Source::Repo).is_empty());
    }

    #[test]
    fn arrow_list_skips_ignored_packages() {
        // pacman -Qu / checkupdates and paru list IgnorePkg entries with
        // an `[ignored]` tag; `-Syu` won't upgrade them, so they aren't
        // pending updates.
        let out = "linux 6.9.1-1 -> 6.9.2-1 [ignored]\nmesa 1-1 -> 2-1\n";
        assert_eq!(
            parse_arrow_list(out, Source::Repo),
            vec![entry(Source::Repo, "mesa", Some("1-1"), Some("2-1"))]
        );
    }

    #[test]
    fn dnf_obsoleting_section_is_not_listed() {
        // The section header is not a package, and the packages under it
        // already appear in the main list.
        let out = "grub2-tools.x86_64   1:2.06-100.fc39   updates\n\
                   Obsoleting Packages\n\
                   grub2-tools.x86_64   1:2.06-100.fc39   updates\n\
                   \x20   grub2-tools.x86_64   1:2.06-95.fc39   @updates\n";
        let names: Vec<_> = parse_dnf_check_update(out)
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["grub2-tools.x86_64"]);
    }

    #[test]
    fn arrow_list_keeps_the_source() {
        let e = parse_arrow_list("a 1 -> 2", Source::Aur);
        assert_eq!(e[0].source, Source::Aur);
    }

    #[test]
    fn dnf_names_skip_continuation_lines() {
        let out = "\nkernel.x86_64          6.8.9-300.fc40     updates\n\
                   firefox.x86_64         126.0-1.fc40       updates\n\
                   \x20   continued.noarch   1-1   updates\n";
        let names: Vec<_> = parse_dnf_check_update(out)
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["kernel.x86_64", "firefox.x86_64"]);
    }

    #[test]
    fn apt_names_from_upgradable_lines_only() {
        let out = "Listing... Done\n\
                   firefox/jammy-updates 126.0+build1 amd64 [upgradable from: 125.0]\n\
                   libc6/jammy-updates,jammy-security 2.35-0ubuntu3.8 amd64 [upgradable from: 2.35-0ubuntu3.7]\n";
        let names: Vec<_> = parse_apt_upgradable(out)
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["firefox", "libc6"]);
    }

    #[test]
    fn flatpak_join_lines() {
        let out = "org.mozilla.firefox\tFirefox\t126.0\t125.0\n\
                   com.example.NoName\t\t2.0\t1.0\n\
                   org.example.Unversioned\tUnversioned\t\t\n\
                   \n";
        assert_eq!(
            parse_flatpak_join(out),
            vec![
                entry(Source::Flatpak, "Firefox", Some("125.0"), Some("126.0")),
                entry(
                    Source::Flatpak,
                    "com.example.NoName",
                    Some("1.0"),
                    Some("2.0")
                ),
                entry(Source::Flatpak, "Unversioned", None, None),
            ]
        );
    }

    #[test]
    fn report_counts_per_source() {
        let report = UpdateReport {
            entries: vec![
                entry(Source::Repo, "a", None, None),
                entry(Source::Aur, "b", None, None),
                entry(Source::Repo, "c", None, None),
            ],
            error: None,
        };
        assert_eq!(report.total(), 3);
        assert_eq!(report.count(Source::Repo), 2);
        assert_eq!(report.count(Source::Aur), 1);
        assert_eq!(report.count(Source::Flatpak), 0);
        assert!(!report.is_empty());
        assert!(UpdateReport::default().is_empty());
    }

    #[test]
    fn cache_round_trips_through_json() {
        let cache = UpdateCache {
            checked_at: 42,
            report: UpdateReport {
                entries: vec![entry(Source::Flatpak, "x", Some("1"), None)],
                error: Some("boom".into()),
            },
        };
        let back: UpdateCache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(back.checked_at, 42);
        assert_eq!(back.report, cache.report);
    }

    #[test]
    fn terminal_argv_per_emulator() {
        let s = || "echo hi".to_string();
        assert_eq!(terminal_argv("kitty", s()), ["--", "sh", "-c", "echo hi"]);
        assert_eq!(terminal_argv("foot", s()), ["sh", "-c", "echo hi"]);
        assert_eq!(
            terminal_argv("gnome-terminal", s()),
            ["--", "sh", "-c", "echo hi"]
        );
        assert_eq!(
            terminal_argv("alacritty", s()),
            ["-e", "sh", "-c", "echo hi"]
        );
        assert_eq!(terminal_argv("xterm", s()), ["-e", "sh", "-c", "echo hi"]);
    }
}
