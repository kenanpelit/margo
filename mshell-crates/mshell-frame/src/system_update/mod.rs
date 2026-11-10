//! Shared system-update probing core, used by the SystemUpdate bar
//! pill and its panel menu. Ports the noctalia `arch-updater`: it
//! counts and lists pending updates split by source — official repo,
//! AUR, and Flatpak — each gated by a Settings toggle.
//!
//! Arch is the primary target (checkupdates + an AUR helper +
//! flatpak). On non-Arch systems the repo source degrades to a
//! count-only probe via dnf / apt (no per-package versions).
//!
//! Submodules add the rest of the arch-updater / arch-audit /
//! topgrade-wrapper feature set: [`news`] (Arch news heads-up),
//! [`size`] (download estimate), [`run`] (background pkexec upgrade
//! with log + notification), [`history`] (update history + rollback),
//! [`pacnew`] (.pacnew/.pacsave review), [`ignore`] (IgnorePkg +
//! hidden packages), [`audit`] (arch-audit advisories); the topgrade
//! backend is [`backend_steps`].

pub(crate) mod audit;
pub(crate) mod history;
pub(crate) mod ignore;
pub(crate) mod news;
pub(crate) mod pacnew;
pub(crate) mod run;
pub(crate) mod size;

use margo_fs::path::on_path;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields,
};
use reactive_graph::traits::GetUntracked;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Single-flight cadence shared by pills and published panel probes.
pub(crate) static LAST_PROBE_SECS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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
    /// Estimated bytes pacman will download for the repo updates
    /// (already-cached packages excluded). `None` = unknown.
    #[serde(default)]
    pub(crate) download_bytes: Option<u64>,
    /// Pending updates the user hid from the count (still upgraded).
    #[serde(default)]
    pub(crate) hidden: Vec<UpdateEntry>,
    /// arch-audit result (`Off` when disabled or not probed).
    #[serde(default)]
    pub(crate) audit: audit::AuditState,
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

/// How old the shared cache may be before a glance surface probes itself.
const GLANCE_CACHE_FRESH_SECS: u64 = 1800;
/// A glance surface's own probe runs at most this often (all monitors and
/// surfaces together).
const GLANCE_PROBE_THROTTLE_SECS: u64 = 900;
static LAST_GLANCE_PROBE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Pending updates for the glance surfaces (the Overview card, mdash's chip
/// and Updates card): the full report the System Update widget cached when
/// it is fresh, else a cheap repo-only `checkupdates` probe (no sudo, no
/// AUR helper) so the count shows without that widget on the bar. When
/// someone probed recently the (stale) cache answers; `None` without one.
pub(crate) async fn glance_report() -> Option<UpdateReport> {
    use std::sync::atomic::Ordering;
    if let Some((checked_at, report)) = load_cache()
        && now_secs().saturating_sub(checked_at) < GLANCE_CACHE_FRESH_SECS
    {
        return Some(report);
    }
    let now = now_secs();
    if now.saturating_sub(LAST_GLANCE_PROBE.load(Ordering::Relaxed)) < GLANCE_PROBE_THROTTLE_SECS {
        return load_cache().map(|(_, r)| r);
    }
    LAST_GLANCE_PROBE.store(now, Ordering::Relaxed);
    Some(
        probe(ProbeConfig {
            repo: true,
            aur: false,
            flatpak: false,
        })
        .await,
    )
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

/// Minimum age for an automatic panel refresh. Explicit Refresh always probes.
const PANEL_TTL: u64 = 600;

#[derive(Default)]
struct ProbeMemo {
    cached: Option<(ProbeConfig, u64, UpdateReport)>,
}
impl ProbeMemo {
    async fn get<F, Fut>(
        &mut self,
        cfg: ProbeConfig,
        now: u64,
        force: bool,
        probe: F,
    ) -> UpdateReport
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = UpdateReport>,
    {
        if !force
            && let Some((key, checked, report)) = &self.cached
            && *key == cfg
            && now.saturating_sub(*checked) < PANEL_TTL
        {
            return report.clone();
        }
        let report = probe().await;
        self.cached = Some((cfg, now, report.clone()));
        report
    }
}

/// Every automatic reveal shares this memo/lock, so two monitors cannot
/// start duplicate probes. Seed with the pill's disk cache on first use.
pub(crate) async fn panel_report(cfg: ProbeConfig, force: bool) -> UpdateReport {
    static MEMO: std::sync::OnceLock<tokio::sync::Mutex<ProbeMemo>> = std::sync::OnceLock::new();
    let mut memo = MEMO
        .get_or_init(|| tokio::sync::Mutex::new(ProbeMemo::default()))
        .lock()
        .await;
    // Prefer a newer report published by any pill/background run.
    if let Some((ts, report)) = load_cache()
        && memo.cached.as_ref().is_none_or(|(_, old, _)| ts > *old)
    {
        memo.cached = Some((cfg, ts, report));
    }
    memo.get(cfg, now_secs(), force, || async {
        let report = probe_with_audit(cfg).await;
        publish_report(report.clone());
        report
    })
    .await
}

/// Which sources to probe — mirrors the config toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Ok(mut entries) => {
                let repo: Vec<&UpdateEntry> = entries.iter().collect();
                if on_path("pacman") {
                    report.download_bytes = size::probe(&repo).await;
                }
                report.entries.append(&mut entries);
            }
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

    let hidden = config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .hidden_packages()
        .get_untracked();
    ignore::apply_hidden(&mut report, &hidden);
    report
}

/// [`probe`] plus the arch-audit advisories when enabled — what the
/// pill and the panel show. (The overview's repo-only count probe
/// stays on plain [`probe`]: no tracker download for a number.)
pub(crate) async fn probe_with_audit(cfg: ProbeConfig) -> UpdateReport {
    let mut report = probe(cfg).await;
    let enabled = config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .check_audit()
        .get_untracked();
    if enabled {
        report.audit = audit::probe().await;
    }
    report
}

// ── Repo (Arch checkupdates, else dnf / apt) ────────────────────

async fn probe_repo() -> Result<Vec<UpdateEntry>, String> {
    if on_path("checkupdates") {
        // pacman-contrib: refreshes a fake DB under /tmp (no sudo).
        // Output: `name oldver -> newver`. Exit 2 = up to date.
        let out = run("checkupdates", &[]).await?;
        return match out.code {
            0 => Ok(parse_arrow_list(&out.stdout, Source::Repo)),
            2 => Ok(Vec::new()),
            c => Err(probe_err("checkupdates", c, &out.stderr)),
        };
    }
    if on_path("dnf") {
        // exit 100 = updates available, 0 = none.
        let out = run("dnf", &["check-update", "--refresh", "-q"]).await?;
        return match out.code {
            0 => Ok(Vec::new()),
            100 => Ok(parse_dnf_check_update(&out.stdout)),
            c => Err(probe_err("dnf", c, &out.stderr)),
        };
    }
    if on_path("apt") {
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
    let helper = if on_path("paru") {
        "paru"
    } else if on_path("yay") {
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
    if !on_path("flatpak") {
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

// ── Fresh reports (published after a background run) ────────────

fn fresh_channel() -> &'static (
    tokio::sync::watch::Sender<Option<UpdateReport>>,
    tokio::sync::watch::Receiver<Option<UpdateReport>>,
) {
    static CH: std::sync::OnceLock<(
        tokio::sync::watch::Sender<Option<UpdateReport>>,
        tokio::sync::watch::Receiver<Option<UpdateReport>>,
    )> = std::sync::OnceLock::new();
    CH.get_or_init(|| tokio::sync::watch::channel(None))
}

/// Every pill + panel listens here so one re-probe after a run updates
/// all monitors (instead of each one probing — and prompting — again).
pub(crate) fn subscribe_reports() -> tokio::sync::watch::Receiver<Option<UpdateReport>> {
    fresh_channel().1.clone()
}

fn deliver_report(tx: &tokio::sync::watch::Sender<Option<UpdateReport>>, report: UpdateReport) {
    tx.send_replace(Some(report));
}

pub(crate) fn publish_report(report: UpdateReport) {
    LAST_PROBE_SECS.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    save_cache(&report);
    deliver_report(&fresh_channel().0, report);
}

// ── Upgrade dispatch ────────────────────────────────────────────

/// Which background upgrade steps apply on this machine for the
/// enabled sources.
pub(crate) async fn upgrade_plan(cfg: ProbeConfig) -> run::UpgradePlan {
    run::UpgradePlan {
        system: (cfg.repo || cfg.aur) && on_path("pacman"),
        flatpak: cfg.flatpak && on_path("flatpak"),
    }
}

/// The installed AUR helper (`paru`, else `yay`).
pub(crate) async fn aur_helper() -> Option<&'static str> {
    if on_path("paru") {
        Some("paru")
    } else if on_path("yay") {
        Some("yay")
    } else {
        None
    }
}

/// The panel's "Review in terminal" for pending AUR updates: the
/// helper's interactive AUR-only upgrade (PKGBUILD diffs shown), in a
/// terminal, whatever `update_mode` says.
pub(crate) async fn launch_aur_review() -> Result<(), String> {
    let Some(helper) = aur_helper().await else {
        return Err("no AUR helper (paru / yay) found".into());
    };
    spawn_in_terminal(&[run::aur_review_step(helper)]).await
}

/// Background mode leaves pending AUR updates to the terminal review:
/// the panel then shows "Review in terminal" on the AUR group.
pub(crate) fn aur_left_for_review(
    report: &UpdateReport,
    mode: mshell_config::schema::config::SystemUpdateMode,
) -> bool {
    mode == mshell_config::schema::config::SystemUpdateMode::Background
        && report.count(Source::Aur) > 0
}

/// How many pending updates the configured Update button will apply
/// (sizes the background progress bar): background mode leaves the
/// AUR to the terminal review.
pub(crate) fn upgrade_expected(
    report: &UpdateReport,
    mode: mshell_config::schema::config::SystemUpdateMode,
) -> usize {
    use mshell_config::schema::config::SystemUpdateMode;
    match mode {
        SystemUpdateMode::Terminal => report.total(),
        SystemUpdateMode::Background => report.total() - report.count(Source::Aur),
    }
}

/// Where an upgrade runs, from the mode + backend settings. topgrade
/// is interactive by design (its own sudo, per-step prompts), so it
/// always gets a terminal.
pub(crate) fn effective_mode(
    mode: mshell_config::schema::config::SystemUpdateMode,
    backend: mshell_config::schema::config::SystemUpdateBackend,
) -> mshell_config::schema::config::SystemUpdateMode {
    use mshell_config::schema::config::{SystemUpdateBackend, SystemUpdateMode};
    match backend {
        SystemUpdateBackend::Topgrade => SystemUpdateMode::Terminal,
        SystemUpdateBackend::Native => mode,
    }
}

/// The configured mode after [`effective_mode`].
pub(crate) fn configured_mode() -> mshell_config::schema::config::SystemUpdateMode {
    let su = || config_manager().config().bars().widgets().system_update();
    effective_mode(
        su().update_mode().get_untracked(),
        su().upgrade_backend().get_untracked(),
    )
}

fn configured_backend() -> mshell_config::schema::config::SystemUpdateBackend {
    config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .upgrade_backend()
        .get_untracked()
}

/// The terminal steps for a backend: `topgrade` alone (it covers the
/// system, AUR and Flatpak itself, per the user's topgrade.toml), or
/// the native steps. A selected-but-missing topgrade is an error, not a
/// silent fallback.
pub(crate) fn backend_steps(
    backend: mshell_config::schema::config::SystemUpdateBackend,
    topgrade_installed: bool,
    native: Vec<String>,
) -> Result<Vec<String>, String> {
    use mshell_config::schema::config::SystemUpdateBackend;
    match backend {
        SystemUpdateBackend::Native => Ok(native),
        SystemUpdateBackend::Topgrade if topgrade_installed => Ok(vec!["topgrade".to_string()]),
        SystemUpdateBackend::Topgrade => {
            Err("topgrade isn't installed (package topgrade)".to_string())
        }
    }
}

/// Run the upgrade the configured way. `expected` (the pending count)
/// sizes the background progress bar.
pub(crate) async fn launch_upgrade(cfg: ProbeConfig, expected: usize) -> Result<(), String> {
    use mshell_config::schema::config::SystemUpdateMode;
    match configured_mode() {
        SystemUpdateMode::Terminal => launch_terminal_upgrade(cfg).await,
        SystemUpdateMode::Background => {
            if !on_path("pkexec") {
                return Err("pkexec (polkit) is not installed".into());
            }
            if !mshell_privileged::installed() {
                return Err(mshell_privileged::MISSING.into());
            }
            let plan = upgrade_plan(cfg).await;
            run::start(run::background_steps(&plan), expected)
        }
    }
}

// ── Upgrade: open a terminal running the full upgrade ───────────

/// Spawn the user's terminal running the upgrade for every enabled
/// source: an AUR helper (covers repo + AUR) or `sudo pacman -Syu`
/// / dnf / apt, then `flatpak update` when flatpak is enabled — or
/// `topgrade` with the topgrade backend.
pub(crate) async fn launch_terminal_upgrade(cfg: ProbeConfig) -> Result<(), String> {
    let mut steps: Vec<String> = Vec::new();
    if cfg.repo || cfg.aur {
        if on_path("paru") {
            steps.push("paru -Syu".into());
        } else if on_path("yay") {
            steps.push("yay -Syu".into());
        } else if on_path("pacman") {
            steps.push("sudo pacman -Syu".into());
        } else if on_path("dnf") {
            steps.push("sudo dnf upgrade".into());
        } else if on_path("apt") {
            steps.push("sudo apt upgrade".into());
        }
    }
    if cfg.flatpak && on_path("flatpak") {
        steps.push("flatpak update".into());
    }
    let steps = backend_steps(configured_backend(), on_path("topgrade"), steps)?;
    if steps.is_empty() {
        warn!("system_update: nothing to upgrade (no backend / all sources off)");
        return Err("nothing to upgrade (no backend / all sources off)".into());
    }
    spawn_in_terminal(&steps).await
}

/// Open the user's terminal running `steps` one after another, then
/// wait for Enter.
async fn spawn_in_terminal(steps: &[String]) -> Result<(), String> {
    use mshell_process::SpawnReaped;
    let script = format!(
        "{}; echo; echo \"[mshell] done — press Enter to close.\"; read",
        steps.join("; ")
    );
    // In its own scope: an upgrade must not die with an mshell restart.
    let (term, mut cmd) =
        mshell_process::terminal::command(&["sh", "-c", &script], "system-update").map_err(
            |e| {
                warn!("system_update: {e}; upgrade ignored");
                e.to_string()
            },
        )?;
    if let Err(e) = cmd.spawn_reaped() {
        warn!(error = %e, term, "system_update: terminal spawn failed");
        return Err(format!("{term}: {e}"));
    }
    Ok(())
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
            ..Default::default()
        };
        assert_eq!(report.total(), 3);
        assert_eq!(report.count(Source::Repo), 2);
        assert_eq!(report.count(Source::Aur), 1);
        assert_eq!(report.count(Source::Flatpak), 0);
        assert!(!report.is_empty());
        assert!(UpdateReport::default().is_empty());
    }

    #[tokio::test]
    async fn two_reveals_share_one_probe_and_refresh_bypasses_ttl() {
        let mut cache = ProbeMemo::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cfg = ProbeConfig {
            repo: true,
            aur: false,
            flatpak: false,
        };
        for (now, force, expected) in [
            (100, false, 1),
            (101, false, 1),
            (102, true, 2),
            (800, false, 3),
        ] {
            cache
                .get(cfg, now, force, || async {
                    calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    UpdateReport::default()
                })
                .await;
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), expected);
        }
    }

    #[test]
    fn a_completed_probe_reaches_both_monitors() {
        let (tx, mut first) = tokio::sync::watch::channel(None);
        let mut second = first.clone();
        let report = UpdateReport {
            entries: vec![entry(Source::Repo, "shared", None, None)],
            ..Default::default()
        };
        deliver_report(&tx, report.clone());
        assert!(first.has_changed().unwrap());
        assert!(second.has_changed().unwrap());
        assert_eq!(*first.borrow_and_update(), Some(report.clone()));
        assert_eq!(*second.borrow_and_update(), Some(report));
    }

    #[test]
    fn cache_round_trips_through_json() {
        let cache = UpdateCache {
            checked_at: 42,
            report: UpdateReport {
                entries: vec![entry(Source::Flatpak, "x", Some("1"), None)],
                error: Some("boom".into()),
                download_bytes: Some(7),
                hidden: vec![entry(Source::Repo, "h", None, None)],
                audit: audit::AuditState::Missing,
            },
        };
        let back: UpdateCache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(back.checked_at, 42);
        assert_eq!(back.report, cache.report);
    }

    #[test]
    fn topgrade_backend_replaces_the_native_steps() {
        use mshell_config::schema::config::{SystemUpdateBackend, SystemUpdateMode};
        let native = || vec!["paru -Syu".to_string(), "flatpak update".to_string()];
        assert_eq!(
            backend_steps(SystemUpdateBackend::Native, true, native()).unwrap(),
            native()
        );
        assert_eq!(
            backend_steps(SystemUpdateBackend::Topgrade, true, native()).unwrap(),
            ["topgrade"]
        );
        assert!(
            backend_steps(SystemUpdateBackend::Topgrade, false, native())
                .unwrap_err()
                .contains("package topgrade")
        );
        // topgrade never runs detached; native follows the mode.
        assert_eq!(
            effective_mode(SystemUpdateMode::Background, SystemUpdateBackend::Topgrade),
            SystemUpdateMode::Terminal
        );
        assert_eq!(
            effective_mode(SystemUpdateMode::Background, SystemUpdateBackend::Native),
            SystemUpdateMode::Background
        );
    }

    #[test]
    fn background_mode_leaves_the_aur_for_review() {
        use mshell_config::schema::config::SystemUpdateMode;
        let report = UpdateReport {
            entries: vec![
                entry(Source::Repo, "a", None, None),
                entry(Source::Aur, "b", None, None),
                entry(Source::Flatpak, "c", None, None),
            ],
            ..Default::default()
        };
        assert_eq!(upgrade_expected(&report, SystemUpdateMode::Terminal), 3);
        assert_eq!(upgrade_expected(&report, SystemUpdateMode::Background), 2);
        assert!(aur_left_for_review(&report, SystemUpdateMode::Background));
        assert!(!aur_left_for_review(&report, SystemUpdateMode::Terminal));
        let no_aur = UpdateReport {
            entries: vec![entry(Source::Repo, "a", None, None)],
            ..Default::default()
        };
        assert!(!aur_left_for_review(&no_aur, SystemUpdateMode::Background));
    }
}
