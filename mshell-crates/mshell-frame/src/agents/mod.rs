//! AI coding agents: plan limits (Claude, Codex), today's local Claude Code
//! stats, and the live Claude Code sessions ([`sessions`], [`SESSIONS_POLL`]).
//!
//! Replaces the `model-usage` WASM plugin, which only counted messages from
//! Claude Code's stale `stats-cache.json`. One [`USAGE_POLL`] serves every
//! bar and the menu: it ticks once a minute while something shows it, reads
//! the local stats incrementally, and calls the (rate-limited) usage
//! endpoints at most every [`LIMITS_EVERY_MS`], backing off on HTTP 429.

pub(crate) mod accounts;
pub(crate) mod details;
pub(crate) mod history;
pub(crate) mod jsonl;
pub(crate) mod launcher;
pub(crate) mod limits;
pub(crate) mod local_stats;
pub(crate) mod memory;
pub(crate) mod plugins;
pub(crate) mod pricing;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod watch;

use limits::{LimitWindow, LimitsError, PlanUsage, Provider, RateTracker, WindowKind};
use local_stats::{LocalStats, StatsCache};
use mshell_utils::shared_poll::SharedPoll;
use sessions::{LiveSession, SavedSession};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// Local stats cadence (and how often the limits are reconsidered).
const TICK: Duration = Duration::from_secs(60);
/// How often the usage endpoints are actually called.
const LIMITS_EVERY_MS: i64 = 5 * 60_000;
/// A manual refresh never calls them sooner than this after the last try.
const MIN_GAP_MS: i64 = 60_000;
/// 429 backoff: from 10 minutes, doubling, up to an hour.
const BACKOFF_MS: i64 = 10 * 60_000;
const BACKOFF_MAX_MS: i64 = 60 * 60_000;

/// One provider's limits as last seen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProviderLimits {
    pub(crate) provider: Provider,
    /// The last good reading, kept (and shown dimmed) across errors.
    pub(crate) usage: Option<PlanUsage>,
    /// Why the latest attempt failed, if it did.
    pub(crate) error: Option<LimitsError>,
    /// Unix ms of the last good reading.
    pub(crate) fetched_at: Option<i64>,
}

/// Plan limits of a Claude account other than the primary one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AccountLimits {
    pub(crate) label: String,
    pub(crate) limits: ProviderLimits,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UsageSnapshot {
    /// The primary account (mshell's own `CLAUDE_CONFIG_DIR`, or `~/.claude`).
    pub(crate) claude: ProviderLimits,
    /// Its label, when there are other accounts to tell it from.
    pub(crate) claude_label: Option<String>,
    /// Every other Claude account seen (see [`accounts`]).
    pub(crate) others: Vec<AccountLimits>,
    /// `None` when the Codex CLI isn't signed in on this machine.
    pub(crate) codex: Option<ProviderLimits>,
    pub(crate) local: LocalStats,
    /// Recent burn rate of Claude's 5-hour window (percent/hour).
    pub(crate) session_rate: Option<f64>,
    pub(crate) now_ms: i64,
}

impl UsageSnapshot {
    /// The window the pill shows: the one closest to its limit among the
    /// session and weekly windows of every provider.
    pub(crate) fn headline(&self) -> Option<(&ProviderLimits, &LimitWindow)> {
        std::iter::once(&self.claude)
            .chain(self.others.iter().map(|a| &a.limits))
            .chain(self.codex.as_ref())
            .filter_map(|p| p.usage.as_ref().map(|u| (p, u)))
            .flat_map(|(p, u)| u.windows.iter().map(move |w| (p, w)))
            .filter(|(_, w)| w.kind != WindowKind::Other)
            .max_by(|a, b| a.1.percent.total_cmp(&b.1.percent))
    }
}

pub(crate) static USAGE_POLL: LazyLock<SharedPoll<UsageSnapshot>> =
    LazyLock::new(|| SharedPoll::new(|| TICK, || Box::pin(probe())));

static FORCE: AtomicBool = AtomicBool::new(false);

/// Ask for fresh limits now (the menu's refresh button). Still honours
/// [`MIN_GAP_MS`] and any 429 hold.
pub(crate) fn request_refresh() {
    FORCE.store(true, Ordering::SeqCst);
    USAGE_POLL.refresh_now();
}

#[derive(Default)]
struct FetchState {
    last_try: i64,
    hold_until: i64,
    backoff: i64,
    limits: Option<ProviderLimits>,
}

#[derive(Default)]
struct PollState {
    claude: FetchState,
    /// The other accounts, by config dir.
    others: HashMap<PathBuf, FetchState>,
    codex: FetchState,
    rate: RateTracker,
    session_rate: Option<f64>,
    stats: StatsCache,
}

static STATE: LazyLock<Mutex<PollState>> = LazyLock::new(Mutex::default);

async fn probe() -> UsageSnapshot {
    let force = FORCE.swap(false, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || probe_blocking(force))
        .await
        .unwrap_or_else(|_| empty_snapshot(now_ms()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn empty_snapshot(now: i64) -> UsageSnapshot {
    UsageSnapshot {
        claude: ProviderLimits {
            provider: Provider::Claude,
            usage: None,
            error: None,
            fetched_at: None,
        },
        claude_label: None,
        others: Vec::new(),
        codex: None,
        local: LocalStats::default(),
        session_rate: None,
        now_ms: now,
    }
}

fn probe_blocking(force: bool) -> UsageSnapshot {
    let now = now_ms();
    let Ok(mut st) = STATE.lock() else {
        return empty_snapshot(now);
    };
    let st = &mut *st;
    let claude_dir = limits::claude_config_dir();
    pricing::ensure_fresh(now);

    refresh_provider(&mut st.claude, Provider::Claude, now, force, || {
        fetch_claude(&claude_dir, now)
    });
    let all = accounts::accounts(now);
    let mut others = Vec::new();
    for a in all.iter().filter(|a| !a.primary) {
        let fs = st.others.entry(a.dir.clone()).or_default();
        refresh_provider(fs, Provider::Claude, now, force, || {
            fetch_claude(&a.dir, now)
        });
        others.push(AccountLimits {
            label: a.label.clone(),
            limits: fs
                .limits
                .clone()
                .unwrap_or_else(|| empty_limits(Provider::Claude)),
        });
    }
    let claude_label = (!others.is_empty()).then(|| all[0].label.clone());

    let codex_auth = limits::codex_home().join("auth.json");
    let codex = if codex_auth.exists() {
        refresh_provider(&mut st.codex, Provider::Codex, now, force, || {
            let raw = zeroize::Zeroizing::new(
                std::fs::read_to_string(&codex_auth).map_err(|_| LimitsError::NotSignedIn)?,
            );
            let auth = limits::parse_codex_auth(&raw)?;
            let bearer = zeroize::Zeroizing::new(format!("Bearer {}", auth.token.as_str()));
            let mut headers: Vec<(&str, &str)> = vec![("Authorization", bearer.as_str())];
            if let Some(id) = auth.account_id.as_deref() {
                headers.push(("ChatGPT-Account-Id", id));
            }
            let body = http_get(limits::CODEX_USAGE_URL, &headers)?;
            limits::parse_codex_usage(&body, now)
        });
        st.codex.limits.clone()
    } else {
        None
    };

    // Sample only a reading fetched on this tick; re-sampling a cached one
    // would flatten the rate. Between fetches the last rate stands.
    if let Some(l) = st.claude.limits.as_ref()
        && l.fetched_at == Some(now)
    {
        st.session_rate = l
            .usage
            .as_ref()
            .and_then(|u| u.windows.iter().find(|w| w.kind == WindowKind::Session))
            .and_then(|w| st.rate.sample(w, now));
    }
    let session_rate = st.session_rate;

    let local = st
        .stats
        .refresh(&claude_dir, local_stats::local_day_start_ms());

    UsageSnapshot {
        claude: st
            .claude
            .limits
            .clone()
            .unwrap_or_else(|| empty_limits(Provider::Claude)),
        claude_label,
        others,
        codex,
        local,
        session_rate,
        now_ms: now,
    }
}

fn empty_limits(provider: Provider) -> ProviderLimits {
    ProviderLimits {
        provider,
        usage: None,
        error: None,
        fetched_at: None,
    }
}

/// One account's Claude plan limits, with the OAuth session Claude Code
/// saved in `dir` (read for this call only, never logged or written).
fn fetch_claude(dir: &Path, now: i64) -> Result<PlanUsage, LimitsError> {
    let oauth = limits::read_claude_credentials(dir, now)?;
    let plan = limits::claude_plan_name(oauth.subscription.as_deref(), oauth.tier.as_deref());
    let bearer = zeroize::Zeroizing::new(format!("Bearer {}", oauth.token.as_str()));
    let body = http_get(
        limits::CLAUDE_USAGE_URL,
        &[
            ("Authorization", bearer.as_str()),
            ("anthropic-beta", "oauth-2025-04-20"),
        ],
    )?;
    limits::parse_claude_usage(&body, plan)
}

/// Call `fetch` when the provider is due (and not held by a 429), keeping
/// the last good reading on failure.
fn refresh_provider(
    st: &mut FetchState,
    provider: Provider,
    now: i64,
    force: bool,
    fetch: impl FnOnce() -> Result<PlanUsage, LimitsError>,
) {
    let due = if force {
        now - st.last_try >= MIN_GAP_MS
    } else {
        now - st.last_try >= LIMITS_EVERY_MS
    };
    if !due || now < st.hold_until {
        return;
    }
    st.last_try = now;
    let prev = st.limits.take();
    let (usage, fetched_at) = match &prev {
        Some(p) => (p.usage.clone(), p.fetched_at),
        None => (None, None),
    };
    st.limits = Some(match fetch() {
        Ok(u) => {
            st.backoff = 0;
            ProviderLimits {
                provider,
                usage: Some(u),
                error: None,
                fetched_at: Some(now),
            }
        }
        Err(e) => {
            if let LimitsError::RateLimited(after) = &e {
                st.backoff = if st.backoff == 0 {
                    BACKOFF_MS
                } else {
                    (st.backoff * 2).min(BACKOFF_MAX_MS)
                };
                let server = after.map_or(0, |s| s as i64 * 1000);
                st.hold_until = now + st.backoff.max(server);
            }
            tracing::debug!(provider = provider.name(), error = ?e, "agent limits unavailable");
            ProviderLimits {
                provider,
                usage,
                error: Some(e),
                fetched_at,
            }
        }
    });
}

/// GET `url` and return the body. Errors never include the headers.
fn http_get(url: &str, headers: &[(&str, &str)]) -> Result<String, LimitsError> {
    let mut req = ureq::get(url)
        .timeout(Duration::from_secs(15))
        .set("Accept", "application/json")
        .set("User-Agent", concat!("mshell/", env!("CARGO_PKG_VERSION")));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| LimitsError::Network(e.kind().to_string())),
        Err(ureq::Error::Status(429, resp)) => Err(LimitsError::RateLimited(
            resp.header("retry-after")
                .and_then(|s| s.trim().parse().ok()),
        )),
        Err(ureq::Error::Status(401, _)) => Err(LimitsError::Expired),
        Err(ureq::Error::Status(code, _)) => Err(LimitsError::Http(code)),
        Err(ureq::Error::Transport(t)) => Err(LimitsError::Network(t.kind().to_string())),
    }
}

// ── Live sessions ──────────────────────────────────────────────────────

/// Session status changes need to show quickly; reading a handful of small
/// JSON files is cheap.
const SESSIONS_TICK: Duration = Duration::from_secs(3);
/// Recent saved sessions listed in the menu.
const SAVED_LIMIT: usize = 12;

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct SessionsSnapshot {
    /// Some account's `<config>/sessions` exists (Claude Code 2.1+ ran).
    pub(crate) available: bool,
    /// Running sessions of every account, needs-you first.
    pub(crate) live: Vec<LiveSession>,
    /// Recent sessions to resume, newest first (running ones excluded).
    pub(crate) saved: Vec<SavedSession>,
    /// Account labels, primary first; rows name their account when
    /// there is more than one.
    pub(crate) accounts: Vec<String>,
}

impl SessionsSnapshot {
    pub(crate) fn count(&self, status: sessions::SessionStatus) -> usize {
        self.live.iter().filter(|s| s.status == status).count()
    }

    pub(crate) fn multi_account(&self) -> bool {
        self.accounts.len() > 1
    }
}

pub(crate) static SESSIONS_POLL: LazyLock<SharedPoll<SessionsSnapshot>> =
    LazyLock::new(|| SharedPoll::new(|| SESSIONS_TICK, || Box::pin(probe_sessions())));

/// history.jsonl is megabytes: [`sessions::SavedIndex`] reads only what
/// was appended, and only when its size or mtime moved.
#[derive(Default)]
struct SavedCache {
    index: sessions::SavedIndex,
    stat: Option<(u64, std::time::SystemTime)>,
    /// The running sessions `saved` leaves out.
    live_ids: Option<Vec<String>>,
    saved: Vec<SavedSession>,
}

#[derive(Default)]
struct SessionsState {
    /// Per account config dir.
    saved: HashMap<PathBuf, SavedCache>,
    /// Per live session ([`LiveSession::key`]).
    details: HashMap<String, details::DetailTracker>,
    watch: watch::StatusWatch,
}

static SESSIONS_STATE: LazyLock<Mutex<SessionsState>> = LazyLock::new(Mutex::default);

async fn probe_sessions() -> SessionsSnapshot {
    tokio::task::spawn_blocking(|| sessions_snapshot(true))
        .await
        .unwrap_or_default()
}

/// Read every account's live and saved sessions. `notify` sends the
/// notifications the user turned on (only the shared poll does; the
/// launcher's one-off read must not repeat them). Blocking.
pub(crate) fn sessions_snapshot(notify: bool) -> SessionsSnapshot {
    let now = now_ms();
    let accounts = accounts::accounts(now);
    let available = accounts.iter().any(|a| a.dir.join("sessions").is_dir());
    let prices = pricing::current();
    let Ok(mut guard) = SESSIONS_STATE.lock() else {
        return SessionsSnapshot::default();
    };
    let st = &mut *guard;
    let mut live: Vec<LiveSession> = Vec::new();
    for a in &accounts {
        live.extend(sessions::read_live_sessions(&a.dir, &a.label));
    }
    let keys: Vec<String> = live.iter().map(LiveSession::key).collect();
    st.details.retain(|k, _| keys.contains(k));
    for s in &mut live {
        let path = details::transcript_path(&s.config_dir, &s.cwd, &s.session_id);
        let d = st.details.entry(s.key()).or_default().refresh(&path);
        s.cost_usd = d.cost_usd(&prices);
        s.context_fraction = d.context_fraction(&prices);
        s.detail = Some(d);
    }
    sessions::sort_sessions(&mut live);
    if notify {
        let multi = accounts.len() > 1;
        let events = st.watch.update(&live, |s| {
            if multi {
                format!("{} · {}", s.account, s.title())
            } else {
                s.title().to_string()
            }
        });
        let cfg = settings::current();
        for e in events {
            send_event(&e, &cfg);
        }
    }
    let mut saved: Vec<SavedSession> = Vec::new();
    for a in &accounts {
        let ids: Vec<String> = live
            .iter()
            .filter(|s| s.config_dir == a.dir)
            .map(|s| s.session_id.clone())
            .collect();
        let cache = st.saved.entry(a.dir.clone()).or_default();
        saved.extend(
            saved_for(cache, &a.dir.join("history.jsonl"), ids)
                .into_iter()
                .map(|mut x| {
                    x.config_dir = a.dir.clone();
                    x.account = a.label.clone();
                    x
                }),
        );
    }
    saved.sort_by_key(|s| std::cmp::Reverse(s.last_ms));
    saved.truncate(SAVED_LIMIT);
    SessionsSnapshot {
        available,
        live,
        saved,
        accounts: accounts.into_iter().map(|a| a.label).collect(),
    }
}

/// One account's recent saved sessions, minus its running `live_ids`.
fn saved_for(cache: &mut SavedCache, history: &Path, live_ids: Vec<String>) -> Vec<SavedSession> {
    let Ok(meta) = std::fs::metadata(history) else {
        return Vec::new();
    };
    let stat = (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH));
    let mut changed = false;
    if cache.stat != Some(stat) {
        changed = cache.index.refresh(history);
        cache.stat = Some(stat);
    }
    if changed || cache.live_ids.as_ref() != Some(&live_ids) {
        let exclude: Vec<&str> = live_ids.iter().map(String::as_str).collect();
        cache.saved = cache.index.top(&exclude, SAVED_LIMIT);
        cache.live_ids = Some(live_ids);
    }
    cache.saved.clone()
}

/// Notify about `e` when the user turned that kind on.
fn send_event(e: &watch::SessionEvent, cfg: &settings::AgentSettings) {
    use mshell_process::desktop::{Notice, Urgency, notify};
    let notice = match e {
        watch::SessionEvent::NeedsYou(key, title, what) if cfg.notify_waiting => {
            let body = if what.is_empty() {
                title.clone()
            } else {
                format!("{title} · {what}")
            };
            Notice::new("AI Agents", "Claude needs you", body)
                .urgency(Urgency::Critical)
                .replace(format!("agents-{key}"))
        }
        watch::SessionEvent::Finished(key, title, turn_ms) if cfg.notify_finished => Notice::new(
            "AI Agents",
            "Claude finished",
            format!("{title} · {}", limits::span(*turn_ms)),
        )
        .replace(format!("agents-{key}")),
        _ => return,
    };
    notify(notice.icon("ai-agents-symbolic").category("im.received"));
}

// ── Usage history ──────────────────────────────────────────────────────

/// Long-range usage changes slowly; a refresh reads only appended lines.
const HISTORY_TICK: Duration = Duration::from_secs(5 * 60);
/// The "where it goes" views cover this many days.
pub(crate) const HISTORY_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct HistorySnapshot {
    pub(crate) today: history::Totals,
    /// Since Monday.
    pub(crate) week: history::Totals,
    /// Since the 1st.
    pub(crate) month: history::Totals,
    /// Since the first build of the cache.
    pub(crate) all: history::Totals,
    pub(crate) all_sessions: u32,
    /// Tokens per day of this week, Monday first.
    pub(crate) weekdays: Vec<u64>,
    /// Per project over the last [`HISTORY_DAYS`], most tokens first.
    pub(crate) projects: Vec<(String, history::Totals)>,
    /// Tokens per local hour of the day over the last [`HISTORY_DAYS`].
    pub(crate) hours: Vec<u64>,
}

pub(crate) static HISTORY_POLL: LazyLock<SharedPoll<HistorySnapshot>> =
    LazyLock::new(|| SharedPoll::new(|| HISTORY_TICK, || Box::pin(probe_history())));

static HISTORY: LazyLock<Mutex<Option<history::UsageHistory>>> = LazyLock::new(Mutex::default);

async fn probe_history() -> HistorySnapshot {
    tokio::task::spawn_blocking(history_snapshot)
        .await
        .unwrap_or_default()
}

/// The `projects/` dirs of every account, each once (accounts may share
/// one through a symlink).
fn projects_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for a in accounts::accounts(now_ms()) {
        let p = a.dir.join("projects");
        let real = std::fs::canonicalize(&p).unwrap_or(p);
        if real.is_dir() && !out.contains(&real) {
            out.push(real);
        }
    }
    out
}

fn history_snapshot() -> HistorySnapshot {
    let Ok(mut guard) = HISTORY.lock() else {
        return HistorySnapshot::default();
    };
    let h = guard.get_or_insert_with(history::load);
    if h.refresh(&projects_dirs(), &history::local_day) {
        history::save(h);
    }
    summarize(h, chrono::Local::now().date_naive())
}

/// The snapshot views of `h` on `today`.
fn summarize(h: &history::UsageHistory, today: chrono::NaiveDate) -> HistorySnapshot {
    use chrono::Datelike;
    let day = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
    let from = day(today - chrono::Duration::days(HISTORY_DAYS - 1));
    let monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let (today_s, week_s, month_s) = (
        day(today),
        day(monday),
        day(today.with_day(1).unwrap_or(today)),
    );
    let mut snap = HistorySnapshot {
        projects: h.by_project(&from),
        hours: vec![0; 24],
        weekdays: vec![0; 7],
        all_sessions: h.sessions(),
        ..HistorySnapshot::default()
    };
    for (d, u) in h.by_day() {
        snap.all.add(&u.totals);
        if d > today_s.as_str() {
            continue;
        }
        if d >= from.as_str() {
            for (i, n) in u.hours.iter().enumerate().take(24) {
                snap.hours[i] += n;
            }
        }
        if d >= month_s.as_str() {
            snap.month.add(&u.totals);
        }
        if d >= week_s.as_str() {
            snap.week.add(&u.totals);
            if let Ok(date) = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                let i = date.weekday().num_days_from_monday() as usize;
                snap.weekdays[i] += u.totals.tokens();
            }
        }
        if d == today_s {
            snap.today.add(&u.totals);
        }
    }
    snap
}

/// Saved sessions whose prompts match every word of `query`, across
/// every account, newest first. Blocking (reads each `history.jsonl`).
pub(crate) fn search_saved(query: &str, limit: usize) -> Vec<SavedSession> {
    let mut out = Vec::new();
    for a in accounts::accounts(now_ms()) {
        let Ok(f) = std::fs::File::open(a.dir.join("history.jsonl")) else {
            continue;
        };
        let hits = sessions::search_history(std::io::BufReader::new(f), query, limit);
        out.extend(hits.into_iter().map(|mut s| {
            s.config_dir = a.dir.clone();
            s.account = a.label.clone();
            s
        }));
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.last_ms));
    out.truncate(limit);
    out
}

/// Every CLAUDE.md of the accounts and of the projects that have
/// sessions. Blocking.
pub(crate) fn memory_files_now() -> Vec<memory::MemoryFile> {
    let indexed = SESSIONS_STATE.lock().is_ok_and(|st| !st.saved.is_empty());
    if !indexed {
        // The project list comes from the history index; build it.
        let _ = sessions_snapshot(false);
    }
    let accounts = accounts::accounts(now_ms());
    let projects: Vec<String> = SESSIONS_STATE
        .lock()
        .map(|st| {
            let mut v: Vec<(i64, String)> =
                st.saved.values().flat_map(|c| c.index.projects()).collect();
            v.sort_by_key(|(ms, _)| std::cmp::Reverse(*ms));
            let mut seen = Vec::new();
            for (_, p) in v {
                if !seen.contains(&p) {
                    seen.push(p);
                }
            }
            seen
        })
        .unwrap_or_default();
    memory::memory_files(&accounts, &projects, |p| {
        std::fs::metadata(p)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len())
    })
}

/// Installed Claude Code plugins and bare skills of every account
/// (blocking file reads; read-only).
pub(crate) fn plugins_now() -> plugins::Inventory {
    plugins::inventory(&accounts::accounts(now_ms()))
}

#[path = "mod_tests.rs"]
#[cfg(test)]
mod mod_tests;
