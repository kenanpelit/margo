//! Plan limits of the AI coding agents (Claude Code, Codex).
//!
//! Claude: `GET https://api.anthropic.com/api/oauth/usage` — the endpoint
//! Claude Code's own `/usage` screen reads — with the OAuth session Claude
//! Code saved in `<config>/.credentials.json`. The token is read only to
//! make this one call: it is never logged, printed, refreshed or written
//! back (refresh tokens rotate; spending one here could sign Claude Code
//! out). An expired token is reported as such; Claude Code renews it the
//! next time it runs.
//!
//! Codex: `GET https://chatgpt.com/backend-api/wham/usage` with the ChatGPT
//! sign-in the Codex CLI keeps in `~/.codex/auth.json`, same rules.
//!
//! Neither endpoint is a documented public API, so every parser here is
//! defensive: unknown or missing fields are skipped, never fatal, and the
//! fixtures in the tests are real responses captured on this machine.

use serde_json::Value;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub(crate) const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub(crate) const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub(crate) const HOUR_MS: i64 = 3_600_000;
pub(crate) const DAY_MS: i64 = 24 * HOUR_MS;
const SESSION_MS: i64 = 5 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;

/// Which agent a set of limits belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    /// The vendor's own usage page, for the menu's "open" action.
    pub(crate) fn dashboard_url(self) -> &'static str {
        match self {
            Provider::Claude => "https://claude.ai/settings/usage",
            Provider::Codex => "https://chatgpt.com/codex/settings/usage",
        }
    }
}

/// What a limit window covers; drives the label and the pace thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowKind {
    /// The rolling 5-hour session window.
    Session,
    /// The 7-day window over every model.
    Weekly,
    /// A 7-day window scoped to one model (e.g. "Weekly · Fable").
    Model,
    Other,
}

/// The vendor's own severity hint, when it sends one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Severity {
    #[default]
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LimitWindow {
    pub(crate) kind: WindowKind,
    pub(crate) label: String,
    /// Used share of the window, 0–100 (may exceed 100 on overage).
    pub(crate) percent: f64,
    /// Reset time, unix ms.
    pub(crate) resets_at: Option<i64>,
    /// Window length, ms.
    pub(crate) period_ms: i64,
    pub(crate) severity: Severity,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanUsage {
    pub(crate) provider: Provider,
    /// "Claude Max 5x", "Codex Plus", …
    pub(crate) plan: String,
    pub(crate) windows: Vec<LimitWindow>,
}

/// Why a provider's limits are unavailable. Messages never carry a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LimitsError {
    /// No credentials file / no OAuth sign-in (API-key setups have no plan).
    NotSignedIn,
    /// The saved access token is past its expiry; the CLI renews it.
    Expired,
    /// HTTP 429; the server's Retry-After in seconds when it sent one.
    RateLimited(Option<u64>),
    Http(u16),
    Network(String),
    Parse(String),
}

impl LimitsError {
    pub(crate) fn message(&self, provider: Provider) -> String {
        let cli = match provider {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        };
        match self {
            LimitsError::NotSignedIn => format!("Not signed in — run `{cli}` and log in"),
            LimitsError::Expired => format!("Sign-in expired — open `{cli}` once to renew it"),
            LimitsError::RateLimited(_) => "Rate limited by the server — retrying later".into(),
            LimitsError::Http(code) => format!("Usage request failed (HTTP {code})"),
            LimitsError::Network(e) => format!("Network error: {e}"),
            LimitsError::Parse(e) => format!("Unexpected response: {e}"),
        }
    }
}

// ── Claude ─────────────────────────────────────────────────────────────

/// The parts of Claude Code's `.credentials.json` this module uses.
pub(crate) struct ClaudeOauth {
    pub(crate) token: Zeroizing<String>,
    pub(crate) subscription: Option<String>,
    pub(crate) tier: Option<String>,
}

impl std::fmt::Debug for ClaudeOauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeOauth")
            .field("token", &"<redacted>")
            .field("subscription", &self.subscription)
            .field("tier", &self.tier)
            .finish()
    }
}

/// `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub(crate) fn claude_config_dir() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => margo_fs::xdg::home().unwrap_or_default().join(".claude"),
    }
}

/// Parse `.credentials.json`. `now_ms` decides expiry.
pub(crate) fn parse_claude_credentials(raw: &str, now_ms: i64) -> Result<ClaudeOauth, LimitsError> {
    let v: Value = serde_json::from_str(raw).map_err(|_| LimitsError::NotSignedIn)?;
    let oauth = v.get("claudeAiOauth").ok_or(LimitsError::NotSignedIn)?;
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or(LimitsError::NotSignedIn)?;
    if let Some(exp) = oauth.get("expiresAt").and_then(Value::as_i64)
        && exp <= now_ms
    {
        return Err(LimitsError::Expired);
    }
    let text = |k: &str| {
        oauth
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Ok(ClaudeOauth {
        token: Zeroizing::new(token.to_owned()),
        subscription: text("subscriptionType"),
        tier: text("rateLimitTier"),
    })
}

pub(crate) fn read_claude_credentials(dir: &Path, now_ms: i64) -> Result<ClaudeOauth, LimitsError> {
    let raw = Zeroizing::new(
        std::fs::read_to_string(dir.join(".credentials.json"))
            .map_err(|_| LimitsError::NotSignedIn)?,
    );
    parse_claude_credentials(&raw, now_ms)
}

const PLAN_WORDS: &[(&str, &str)] = &[
    ("free", "Free"),
    ("pro", "Pro"),
    ("max", "Max"),
    ("team", "Team"),
    ("enterprise", "Enterprise"),
    ("business", "Business"),
];

fn plan_word(w: &str) -> Option<&'static str> {
    PLAN_WORDS.iter().find(|(k, _)| *k == w).map(|(_, v)| *v)
}

/// `"max"` + `"default_claude_max_5x"` → `"Claude Max 5x"`.
pub(crate) fn claude_plan_name(subscription: Option<&str>, tier: Option<&str>) -> String {
    let clean = |v: Option<&str>| {
        v.map(str::to_ascii_lowercase)
            .filter(|s| !s.is_empty() && s != "unknown")
    };
    let sub = clean(subscription);
    let tier = clean(tier);
    let rest = tier.as_deref().unwrap_or("");
    let rest = rest.strip_prefix("default_").unwrap_or(rest);
    let rest = rest.strip_prefix("claude_").unwrap_or(rest);
    let mut multiplier = None;
    let mut family = None;
    for word in rest.split('_').filter(|w| !w.is_empty()) {
        if word.len() > 1
            && word.ends_with('x')
            && word[..word.len() - 1].bytes().all(|b| b.is_ascii_digit())
        {
            multiplier = Some(word.to_string());
        } else if family.is_none() {
            family = plan_word(word);
        }
    }
    let plan = sub
        .as_deref()
        .and_then(plan_word)
        .or(family)
        .map(str::to_owned)
        .or_else(|| sub.map(|s| capitalize(&s)));
    let mut out = String::from("Claude");
    if let Some(p) = plan {
        out.push(' ');
        out.push_str(&p);
    }
    if let Some(m) = multiplier {
        out.push(' ');
        out.push_str(&m);
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `"seven_day_oauth_apps"` → `"Seven day oauth apps"`.
fn humanize(key: &str) -> String {
    capitalize(&key.replace('_', " "))
}

pub(crate) fn parse_iso_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn severity_of(v: Option<&Value>) -> Severity {
    match v.and_then(Value::as_str) {
        Some("critical") => Severity::Critical,
        Some("high") => Severity::High,
        _ => Severity::Normal,
    }
}

/// Parse an `api/oauth/usage` body. The generic `limits` list (which also
/// carries per-model weekly windows) wins; older responses only have the
/// fixed `five_hour` / `seven_day` objects.
pub(crate) fn parse_claude_usage(body: &str, plan: String) -> Result<PlanUsage, LimitsError> {
    let v: Value = serde_json::from_str(body).map_err(|e| LimitsError::Parse(e.to_string()))?;
    let mut windows = Vec::new();
    if let Some(limits) = v.get("limits").and_then(Value::as_array) {
        for l in limits {
            let Some(percent) = l.get("percent").and_then(Value::as_f64) else {
                continue;
            };
            let kind_key = l.get("kind").and_then(Value::as_str).unwrap_or("");
            let group = l.get("group").and_then(Value::as_str).unwrap_or("");
            let scope = l.get("scope");
            let model = scope
                .and_then(|s| {
                    s.pointer("/model/display_name")
                        .or_else(|| s.pointer("/surface/display_name"))
                })
                .and_then(Value::as_str);
            let (kind, label) = match (kind_key, model) {
                ("session", _) => (WindowKind::Session, "5-hour".to_string()),
                ("weekly_all", _) => (WindowKind::Weekly, "Weekly".to_string()),
                (_, Some(m)) => (WindowKind::Model, format!("Weekly · {m}")),
                (k, None) => (
                    WindowKind::Other,
                    humanize(if k.is_empty() { "limit" } else { k }),
                ),
            };
            let period_ms = match (kind, group) {
                (WindowKind::Session, _) | (_, "session") => SESSION_MS,
                _ => WEEK_MS,
            };
            windows.push(LimitWindow {
                kind,
                label,
                percent,
                resets_at: l
                    .get("resets_at")
                    .and_then(Value::as_str)
                    .and_then(parse_iso_ms),
                period_ms,
                severity: severity_of(l.get("severity")),
            });
        }
    }
    if windows.is_empty() {
        for (key, kind, label, period_ms) in [
            ("five_hour", WindowKind::Session, "5-hour", SESSION_MS),
            ("seven_day", WindowKind::Weekly, "Weekly", WEEK_MS),
            (
                "seven_day_opus",
                WindowKind::Model,
                "Weekly · Opus",
                WEEK_MS,
            ),
            (
                "seven_day_sonnet",
                WindowKind::Model,
                "Weekly · Sonnet",
                WEEK_MS,
            ),
        ] {
            let Some(w) = v.get(key).filter(|w| w.is_object()) else {
                continue;
            };
            let Some(percent) = w.get("utilization").and_then(Value::as_f64) else {
                continue;
            };
            windows.push(LimitWindow {
                kind,
                label: label.to_string(),
                percent,
                resets_at: w
                    .get("resets_at")
                    .and_then(Value::as_str)
                    .and_then(parse_iso_ms),
                period_ms,
                severity: Severity::Normal,
            });
        }
    }
    if windows.is_empty() {
        return Err(LimitsError::Parse("no usage windows in response".into()));
    }
    Ok(PlanUsage {
        provider: Provider::Claude,
        plan,
        windows,
    })
}

// ── Codex ──────────────────────────────────────────────────────────────

pub(crate) struct CodexAuth {
    pub(crate) token: Zeroizing<String>,
    pub(crate) account_id: Option<String>,
}

impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAuth")
            .field("token", &"<redacted>")
            .field("account_id", &self.account_id.is_some())
            .finish()
    }
}

/// `$CODEX_HOME`, else `~/.codex`.
pub(crate) fn codex_home() -> PathBuf {
    match std::env::var_os("CODEX_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => margo_fs::xdg::home().unwrap_or_default().join(".codex"),
    }
}

/// Parse `~/.codex/auth.json` (ChatGPT sign-in). An API-key-only setup has
/// no plan limits, which reads as not signed in. Expiry is left to the
/// server (a 401 maps to [`LimitsError::Expired`]): decoding the JWT here
/// would mean handling the token more than the one request needs.
pub(crate) fn parse_codex_auth(raw: &str) -> Result<CodexAuth, LimitsError> {
    let v: Value = serde_json::from_str(raw).map_err(|_| LimitsError::NotSignedIn)?;
    let tokens = v.get("tokens").ok_or(LimitsError::NotSignedIn)?;
    let token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or(LimitsError::NotSignedIn)?;
    Ok(CodexAuth {
        token: Zeroizing::new(token.to_owned()),
        account_id: tokens
            .get("account_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    })
}

fn codex_plan(plan_type: Option<&str>) -> String {
    let name = match plan_type.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => return "Codex".into(),
        Some("team" | "business") => "Business".to_string(),
        Some(other) => capitalize(other),
    };
    format!("Codex {name}")
}

fn codex_window(raw: &Value, label_prefix: Option<&str>, now_ms: i64) -> Option<LimitWindow> {
    let percent = raw.get("used_percent").and_then(Value::as_f64)?;
    let secs = raw.get("limit_window_seconds").and_then(Value::as_i64);
    let resets_at = raw
        .get("reset_at")
        .and_then(Value::as_i64)
        .map(|s| s * 1000)
        .or_else(|| {
            raw.get("reset_after_seconds")
                .and_then(Value::as_i64)
                .map(|s| now_ms + s * 1000)
        });
    // Classify by length so a vendor change from 5h to 4h, or a new
    // monthly window, still lands in the right slot.
    let kind = match secs {
        Some(s) if s <= 12 * 3600 => WindowKind::Session,
        Some(s) if (6 * 86400..=8 * 86400).contains(&s) => WindowKind::Weekly,
        _ => WindowKind::Other,
    };
    let base = match (kind, secs) {
        (WindowKind::Session, Some(s)) => format!("{}-hour", (s + 1800) / 3600),
        (WindowKind::Weekly, _) => "Weekly".to_string(),
        (_, Some(s)) if s >= 86400 => format!("{}-day", s / 86400),
        (_, Some(s)) => format!("{}-hour", (s + 1800) / 3600),
        _ => "Limit".to_string(),
    };
    let label = match label_prefix {
        Some(p) => format!("{p} · {base}"),
        None => base,
    };
    Some(LimitWindow {
        kind: if label_prefix.is_some() {
            WindowKind::Other
        } else {
            kind
        },
        label,
        percent,
        resets_at,
        period_ms: secs.map_or(WEEK_MS, |s| s * 1000),
        severity: Severity::Normal,
    })
}

/// Parse a `wham/usage` body: `rate_limit.{primary,secondary}_window`, the
/// code-review limit and any `additional_rate_limits`.
pub(crate) fn parse_codex_usage(body: &str, now_ms: i64) -> Result<PlanUsage, LimitsError> {
    let v: Value = serde_json::from_str(body).map_err(|e| LimitsError::Parse(e.to_string()))?;
    let mut windows = Vec::new();
    let mut collect = |rl: Option<&Value>, prefix: Option<&str>| {
        let Some(rl) = rl else { return };
        for slot in ["primary_window", "secondary_window"] {
            if let Some(w) = rl.get(slot).and_then(|w| codex_window(w, prefix, now_ms)) {
                windows.push(w);
            }
        }
    };
    collect(v.get("rate_limit"), None);
    collect(v.get("code_review_rate_limit"), Some("Code review"));
    if let Some(extra) = v.get("additional_rate_limits").and_then(Value::as_array) {
        for e in extra {
            let name = e
                .get("limit_name")
                .or_else(|| e.get("metered_feature"))
                .and_then(Value::as_str)
                .unwrap_or("Extra");
            collect(Some(e.get("rate_limit").unwrap_or(e)), Some(name));
        }
    }
    if windows.is_empty() {
        return Err(LimitsError::Parse("no usage windows in response".into()));
    }
    Ok(PlanUsage {
        provider: Provider::Codex,
        plan: codex_plan(v.get("plan_type").and_then(Value::as_str)),
        windows,
    })
}

// ── Pace forecast ──────────────────────────────────────────────────────

/// Where a window is heading at the current burn rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaceStatus {
    /// No active window (reset passed, nothing used since).
    Idle,
    /// Too little of the window has run to project.
    Early,
    Ok,
    /// On course for 85 %+ by the reset.
    Tight,
    /// 100 % well before the reset.
    Hit,
    /// 100 % within the hour (5-hour window) or the day (weekly ones).
    Soon,
    /// At or past 100 %.
    Limit,
}

impl PaceStatus {
    fn rank(self) -> u8 {
        match self {
            PaceStatus::Idle | PaceStatus::Early | PaceStatus::Ok => 0,
            PaceStatus::Tight => 1,
            PaceStatus::Hit => 2,
            PaceStatus::Soon => 3,
            PaceStatus::Limit => 4,
        }
    }

    /// DESIGN.md §2 severity ladder class for this status.
    pub(crate) fn severity_class(self) -> &'static str {
        match self {
            PaceStatus::Limit | PaceStatus::Soon => "danger",
            PaceStatus::Hit | PaceStatus::Tight => "warn",
            _ => "calm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pace {
    pub(crate) status: PaceStatus,
    /// Burn rate in percent per hour, once it means anything.
    pub(crate) rate: Option<f64>,
    /// Percent expected at the reset.
    pub(crate) projected: Option<f64>,
    /// Time until 100 %, when that comes before the reset.
    pub(crate) eta_ms: Option<i64>,
    /// Percent per hour that would last exactly until the reset.
    pub(crate) budget: Option<f64>,
    pub(crate) remaining_ms: Option<i64>,
}

/// How long a window must have run before its average rate means
/// anything: twenty minutes of a week is noise.
fn min_elapsed(kind: WindowKind) -> i64 {
    if kind == WindowKind::Session {
        15 * 60_000
    } else {
        6 * HOUR_MS
    }
}

/// Forecast `w` at `now_ms`. `recent_rate` (percent/hour from the last
/// samples, see [`RateTracker`]) replaces the window average for the
/// 5-hour window when known. The vendor's own severity wins when worse.
pub(crate) fn pace(w: &LimitWindow, now_ms: i64, recent_rate: Option<f64>) -> Pace {
    let util = w.percent;
    let mut p = Pace {
        status: PaceStatus::Ok,
        rate: None,
        projected: None,
        eta_ms: None,
        budget: None,
        remaining_ms: None,
    };
    let Some(resets_at) = w.resets_at.filter(|r| *r > now_ms) else {
        p.status = if util >= 100.0 {
            PaceStatus::Limit
        } else {
            PaceStatus::Idle
        };
        return p;
    };
    let remaining = resets_at - now_ms;
    let elapsed = (w.period_ms - remaining).clamp(0, w.period_ms);
    p.remaining_ms = Some(remaining);
    p.budget = Some((100.0 - util).max(0.0) / (remaining as f64 / HOUR_MS as f64));
    if util >= 100.0 {
        p.status = PaceStatus::Limit;
        return p;
    }
    let rate = match recent_rate {
        Some(r) if w.kind == WindowKind::Session => Some(r),
        _ if elapsed >= min_elapsed(w.kind) => Some(util / (elapsed as f64 / HOUR_MS as f64)),
        _ => None,
    };
    match rate {
        None => {
            p.status = if util >= 85.0 {
                PaceStatus::Tight
            } else {
                PaceStatus::Early
            };
        }
        Some(rate) => {
            p.rate = Some(rate);
            let projected = util + rate * remaining as f64 / HOUR_MS as f64;
            p.projected = Some(projected);
            if rate > 0.0 {
                let eta = ((100.0 - util) / rate * HOUR_MS as f64) as i64;
                if eta < remaining {
                    p.eta_ms = Some(eta);
                }
            }
            let soon = if w.kind == WindowKind::Session {
                HOUR_MS
            } else {
                DAY_MS
            };
            p.status = match p.eta_ms {
                Some(eta) if eta < soon => PaceStatus::Soon,
                Some(eta) if (eta as f64) < remaining as f64 * 0.9 => PaceStatus::Hit,
                _ if projected >= 85.0 => PaceStatus::Tight,
                _ => PaceStatus::Ok,
            };
        }
    }
    let vendor = match w.severity {
        Severity::Critical => Some(PaceStatus::Soon),
        Severity::High => Some(PaceStatus::Tight),
        Severity::Normal => None,
    };
    if let Some(v) = vendor
        && v.rank() > p.status.rank()
    {
        p.status = v;
    }
    p
}

/// Recent burn rate of the 5-hour window, from the samples the poll
/// takes. A reset (new `resets_at`) or a drop in percent starts over.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RateTracker {
    window_reset: Option<i64>,
    samples: Vec<(i64, f64)>,
}

impl RateTracker {
    /// Keep the last hour of samples; the rate needs 15 minutes of them.
    pub(crate) fn sample(&mut self, w: &LimitWindow, now_ms: i64) -> Option<f64> {
        let same = self.window_reset == w.resets_at
            && self.samples.last().is_none_or(|(_, p)| *p <= w.percent);
        if !same {
            self.samples.clear();
            self.window_reset = w.resets_at;
        }
        self.samples.push((now_ms, w.percent));
        self.samples.retain(|(t, _)| now_ms - t <= HOUR_MS);
        let (t0, p0) = *self.samples.first()?;
        let span = now_ms - t0;
        (span >= 15 * 60_000).then(|| (w.percent - p0) / (span as f64 / HOUR_MS as f64))
    }
}

// ── Formatting ─────────────────────────────────────────────────────────

/// `"2d 4h"`, `"3h 10m"` or `"25m"`.
pub(crate) fn span(ms: i64) -> String {
    let m = (ms / 60_000).max(0);
    if m >= 1440 {
        format!("{}d {}h", m / 1440, m % 1440 / 60)
    } else if m >= 60 {
        format!("{}h {}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

/// `"12%/h"` for the 5-hour window, `"14%/day"` for longer ones.
pub(crate) fn rate_text(kind: WindowKind, per_hour: f64) -> String {
    if kind == WindowKind::Session {
        format!("{per_hour:.0}%/h")
    } else {
        format!("{:.0}%/day", per_hour * 24.0)
    }
}

/// One line saying where the window is heading.
pub(crate) fn forecast_text(w: &LimitWindow, p: &Pace) -> String {
    match p.status {
        PaceStatus::Idle => "No active window — starts with your next message".into(),
        PaceStatus::Limit => "Limit reached".into(),
        _ => match (p.eta_ms, p.projected) {
            (Some(eta), _) if matches!(p.status, PaceStatus::Hit | PaceStatus::Soon) => {
                format!("Limit in {} at this rate", span(eta))
            }
            (_, Some(proj)) => format!(
                "On course for ~{:.0}% by reset ({})",
                proj.min(100.0),
                rate_text(w.kind, p.rate.unwrap_or(0.0))
            ),
            _ if p.status == PaceStatus::Soon => "Close to the limit".into(),
            _ => "Too early to project".into(),
        },
    }
}

/// `"resets in 3h 10m · budget 12%/h"`.
pub(crate) fn reset_text(w: &LimitWindow, p: &Pace) -> String {
    let Some(rem) = p.remaining_ms else {
        return String::new();
    };
    let mut s = format!("Resets in {}", span(rem));
    if let Some(b) = p.budget
        && p.status != PaceStatus::Limit
    {
        s.push_str(&format!(" · budget {}", rate_text(w.kind, b)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `api/oauth/usage` response captured on this machine
    /// (2026-11-04, Max 5x), trimmed of the many always-null codename keys.
    const CLAUDE_SAMPLE: &str = r#"{
      "five_hour": {"utilization": 38.0, "resets_at": "2026-11-04T16:50:00.458446+00:00",
                    "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
      "seven_day": {"utilization": 38.0, "resets_at": "2026-11-09T23:00:00.458472+00:00",
                    "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
      "seven_day_oauth_apps": null, "seven_day_opus": null, "seven_day_sonnet": null,
      "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null},
      "limits": [
        {"kind": "session", "group": "session", "percent": 38, "severity": "normal",
         "resets_at": "2026-11-04T16:50:00.458446+00:00", "scope": null, "is_active": true},
        {"kind": "weekly_all", "group": "weekly", "percent": 38, "severity": "normal",
         "resets_at": "2026-11-09T23:00:00.458472+00:00", "scope": null, "is_active": false},
        {"kind": "weekly_scoped", "group": "weekly", "percent": 0, "severity": "normal",
         "resets_at": "2026-11-09T23:00:00+00:00",
         "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}, "is_active": false}
      ],
      "spend": {"used": {"amount_minor": 0, "currency": "USD", "exponent": 2}, "limit": null}
    }"#;

    #[test]
    fn claude_limits_list_gives_session_weekly_and_model_windows() {
        let u = parse_claude_usage(CLAUDE_SAMPLE, "Claude Max 5x".into()).unwrap();
        assert_eq!(u.windows.len(), 3);
        let s = &u.windows[0];
        assert_eq!(
            (s.kind, s.label.as_str(), s.percent),
            (WindowKind::Session, "5-hour", 38.0)
        );
        assert_eq!(
            s.resets_at,
            parse_iso_ms("2026-11-04T16:50:00.458446+00:00")
        );
        assert_eq!(s.period_ms, 5 * HOUR_MS);
        assert_eq!(u.windows[1].kind, WindowKind::Weekly);
        assert_eq!(u.windows[1].period_ms, 7 * DAY_MS);
        assert_eq!(u.windows[2].kind, WindowKind::Model);
        assert_eq!(u.windows[2].label, "Weekly · Fable");
    }

    #[test]
    fn claude_falls_back_to_fixed_fields_without_limits() {
        let body = r#"{"five_hour":{"utilization":12.5,"resets_at":"2026-11-04T16:50:00+00:00"},
                       "seven_day":{"utilization":40,"resets_at":null},"seven_day_opus":null}"#;
        let u = parse_claude_usage(body, "Claude".into()).unwrap();
        assert_eq!(u.windows.len(), 2);
        assert_eq!(u.windows[0].percent, 12.5);
        assert_eq!(u.windows[1].resets_at, None);
    }

    #[test]
    fn claude_without_windows_is_a_parse_error() {
        assert!(matches!(
            parse_claude_usage(r#"{"limits":[]}"#, String::new()),
            Err(LimitsError::Parse(_))
        ));
        assert!(matches!(
            parse_claude_usage("<html>", String::new()),
            Err(LimitsError::Parse(_))
        ));
    }

    #[test]
    fn credentials_parse_expiry_and_plan_without_exposing_the_token() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-test","refreshToken":"r","expiresAt":2000,
                      "subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}"#;
        let ok = parse_claude_credentials(raw, 1000).unwrap();
        assert_eq!(ok.tier.as_deref(), Some("default_claude_max_5x"));
        assert!(!format!("{ok:?}").contains("sk-test"));
        assert_eq!(
            parse_claude_credentials(raw, 2000).unwrap_err(),
            LimitsError::Expired
        );
        assert_eq!(
            parse_claude_credentials("{}", 0).unwrap_err(),
            LimitsError::NotSignedIn
        );
        assert_eq!(
            parse_claude_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#, 0).unwrap_err(),
            LimitsError::NotSignedIn
        );
    }

    #[test]
    fn plan_names() {
        assert_eq!(
            claude_plan_name(Some("max"), Some("default_claude_max_5x")),
            "Claude Max 5x"
        );
        assert_eq!(claude_plan_name(Some("pro"), None), "Claude Pro");
        assert_eq!(
            claude_plan_name(Some("team"), Some("default_claude_max_5x")),
            "Claude Team 5x"
        );
        assert_eq!(claude_plan_name(None, None), "Claude");
        assert_eq!(claude_plan_name(Some("weird"), None), "Claude Weird");
    }

    #[test]
    fn codex_windows_classified_by_length() {
        // Shape of chatgpt.com/backend-api/wham/usage (the Codex CLI's
        // /status source); this machine's token had expired (HTTP 401
        // token_expired), so the body follows the fields the Codex CLI reads.
        let body = r#"{"plan_type":"plus","rate_limit":{
            "primary_window":{"used_percent":20,"limit_window_seconds":18000,"reset_after_seconds":600},
            "secondary_window":{"used_percent":5,"limit_window_seconds":604800,"reset_at":1793900000}},
          "code_review_rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":604800}},
          "credits":null}"#;
        let u = parse_codex_usage(body, 1_000).unwrap();
        assert_eq!(u.plan, "Codex Plus");
        assert_eq!(u.windows[0].kind, WindowKind::Session);
        assert_eq!(u.windows[0].label, "5-hour");
        assert_eq!(u.windows[0].resets_at, Some(1_000 + 600_000));
        assert_eq!(u.windows[1].kind, WindowKind::Weekly);
        assert_eq!(u.windows[1].resets_at, Some(1_793_900_000_000));
        assert_eq!(u.windows[2].label, "Code review · Weekly");
        assert!(parse_codex_usage(r#"{"rate_limit":null}"#, 0).is_err());
    }

    #[test]
    fn codex_auth_requires_chatgpt_tokens() {
        let ok = parse_codex_auth(
            r#"{"OPENAI_API_KEY":null,"tokens":{"access_token":"t","account_id":"a"}}"#,
        )
        .unwrap();
        assert_eq!(ok.account_id.as_deref(), Some("a"));
        assert!(!format!("{ok:?}").contains("\"t\""));
        assert_eq!(
            parse_codex_auth(r#"{"OPENAI_API_KEY":"sk"}"#).unwrap_err(),
            LimitsError::NotSignedIn
        );
    }

    fn window(kind: WindowKind, percent: f64, period_ms: i64, remaining_ms: i64) -> LimitWindow {
        LimitWindow {
            kind,
            label: String::new(),
            percent,
            resets_at: Some(1_000_000_000 + remaining_ms),
            period_ms,
            severity: Severity::Normal,
        }
    }

    const NOW: i64 = 1_000_000_000;

    #[test]
    fn pace_projects_from_the_window_average() {
        // 2.5 h into a 5 h window at 40 % → 16 %/h → 80 % at reset.
        let w = window(WindowKind::Session, 40.0, 5 * HOUR_MS, 150 * 60_000);
        let p = pace(&w, NOW, None);
        assert_eq!(p.status, PaceStatus::Ok);
        assert!((p.rate.unwrap() - 16.0).abs() < 1e-9);
        assert!((p.projected.unwrap() - 80.0).abs() < 1e-9);
        assert_eq!(p.eta_ms, None);
        assert!((p.budget.unwrap() - 24.0).abs() < 1e-9);
    }

    #[test]
    fn pace_flags_hitting_the_limit() {
        // 1 h in, 60 % used: 60 %/h → 100 % in 40 min, under the hour → Soon.
        let w = window(WindowKind::Session, 60.0, 5 * HOUR_MS, 4 * HOUR_MS);
        let p = pace(&w, NOW, None);
        assert_eq!(p.status, PaceStatus::Soon);
        assert_eq!(p.eta_ms, Some(40 * 60_000));
        // Weekly: 1 day in, 30 % → 100 % in ~2.3 days of 6 left → Hit.
        let w = window(WindowKind::Weekly, 30.0, 7 * DAY_MS, 6 * DAY_MS);
        assert_eq!(pace(&w, NOW, None).status, PaceStatus::Hit);
    }

    #[test]
    fn pace_early_idle_limit_and_vendor_severity() {
        let w = window(WindowKind::Weekly, 2.0, 7 * DAY_MS, 7 * DAY_MS - HOUR_MS);
        assert_eq!(pace(&w, NOW, None).status, PaceStatus::Early);
        let mut w = window(WindowKind::Session, 10.0, 5 * HOUR_MS, -1);
        assert_eq!(pace(&w, NOW, None).status, PaceStatus::Idle);
        w.percent = 100.0;
        assert_eq!(pace(&w, NOW, None).status, PaceStatus::Limit);
        let mut w = window(WindowKind::Weekly, 2.0, 7 * DAY_MS, 7 * DAY_MS - HOUR_MS);
        w.severity = Severity::Critical;
        assert_eq!(pace(&w, NOW, None).status, PaceStatus::Soon);
    }

    #[test]
    fn recent_rate_overrides_the_session_average() {
        let w = window(WindowKind::Session, 40.0, 5 * HOUR_MS, 150 * 60_000);
        let p = pace(&w, NOW, Some(4.0));
        assert_eq!(p.rate, Some(4.0));
        assert!((p.projected.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn rate_tracker_needs_fifteen_minutes_and_restarts_on_reset() {
        let mut t = RateTracker::default();
        let mut w = window(WindowKind::Session, 10.0, 5 * HOUR_MS, HOUR_MS);
        assert_eq!(t.sample(&w, 0), None);
        w.percent = 15.0;
        assert_eq!(t.sample(&w, 10 * 60_000), None);
        w.percent = 20.0;
        assert_eq!(t.sample(&w, 30 * 60_000), Some(20.0));
        // New window (different reset) → fresh samples.
        w.resets_at = Some(42);
        assert_eq!(t.sample(&w, 40 * 60_000), None);
    }

    #[test]
    fn spans_and_texts() {
        assert_eq!(span(25 * 60_000), "25m");
        assert_eq!(span(190 * 60_000), "3h 10m");
        assert_eq!(span(52 * HOUR_MS), "2d 4h");
        assert_eq!(rate_text(WindowKind::Weekly, 0.5), "12%/day");
        let w = window(WindowKind::Session, 40.0, 5 * HOUR_MS, 150 * 60_000);
        let p = pace(&w, NOW, None);
        assert_eq!(forecast_text(&w, &p), "On course for ~80% by reset (16%/h)");
        assert_eq!(reset_text(&w, &p), "Resets in 2h 30m · budget 24%/h");
    }
}
