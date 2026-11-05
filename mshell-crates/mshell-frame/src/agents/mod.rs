//! AI coding agents: plan limits (Claude, Codex), today's local Claude Code
//! stats, and the live Claude Code sessions ([`sessions`], [`SESSIONS_POLL`]).
//!
//! Replaces the `model-usage` WASM plugin, which only counted messages from
//! Claude Code's stale `stats-cache.json`. One [`USAGE_POLL`] serves every
//! bar and the menu: it ticks once a minute while something shows it, reads
//! the local stats incrementally, and calls the (rate-limited) usage
//! endpoints at most every [`LIMITS_EVERY_MS`], backing off on HTTP 429.

pub(crate) mod jsonl;
pub(crate) mod limits;
pub(crate) mod local_stats;
pub(crate) mod sessions;

use limits::{LimitWindow, LimitsError, PlanUsage, Provider, RateTracker, WindowKind};
use local_stats::{LocalStats, StatsCache};
use mshell_utils::shared_poll::SharedPoll;
use sessions::{LiveSession, SavedSession};
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UsageSnapshot {
    pub(crate) claude: ProviderLimits,
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

    refresh_provider(&mut st.claude, Provider::Claude, now, force, || {
        let oauth = limits::read_claude_credentials(&claude_dir, now)?;
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
    });

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
        claude: st.claude.limits.clone().unwrap_or(ProviderLimits {
            provider: Provider::Claude,
            usage: None,
            error: None,
            fetched_at: None,
        }),
        codex,
        local,
        session_rate,
        now_ms: now,
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SessionsSnapshot {
    /// `<config>/sessions` exists (Claude Code 2.1+ is installed and ran).
    pub(crate) available: bool,
    /// Running sessions, needs-you first.
    pub(crate) live: Vec<LiveSession>,
    /// Recent sessions to resume, newest first (running ones excluded).
    pub(crate) saved: Vec<SavedSession>,
}

impl SessionsSnapshot {
    pub(crate) fn count(&self, status: sessions::SessionStatus) -> usize {
        self.live.iter().filter(|s| s.status == status).count()
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

static SAVED: LazyLock<Mutex<SavedCache>> = LazyLock::new(Mutex::default);

async fn probe_sessions() -> SessionsSnapshot {
    tokio::task::spawn_blocking(probe_sessions_blocking)
        .await
        .unwrap_or_default()
}

fn probe_sessions_blocking() -> SessionsSnapshot {
    let dir = limits::claude_config_dir();
    let available = dir.join("sessions").is_dir();
    let live = sessions::read_live_sessions(&dir);
    let live_ids: Vec<String> = live.iter().map(|s| s.session_id.clone()).collect();
    let history = dir.join("history.jsonl");
    let saved = match (std::fs::metadata(&history), SAVED.lock()) {
        (Ok(meta), Ok(mut guard)) => {
            let cache = &mut *guard;
            let stat = (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH));
            let mut changed = false;
            if cache.stat != Some(stat) {
                changed = cache.index.refresh(&history);
                cache.stat = Some(stat);
            }
            if changed || cache.live_ids.as_ref() != Some(&live_ids) {
                let exclude: Vec<&str> = live_ids.iter().map(String::as_str).collect();
                cache.saved = cache.index.top(&exclude, SAVED_LIMIT);
                cache.live_ids = Some(live_ids);
            }
            cache.saved.clone()
        }
        _ => Vec::new(),
    };
    SessionsSnapshot {
        available,
        live,
        saved,
    }
}

#[path = "mod_tests.rs"]
#[cfg(test)]
mod mod_tests;
