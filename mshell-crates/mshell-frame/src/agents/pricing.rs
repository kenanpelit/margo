//! Model prices for the cost estimates: LiteLLM's public price table
//! (the source the claude-sessions / claude-operator plugins use), plus
//! the ECB USD→EUR rate from Frankfurter. Both are public and fetched
//! without credentials, at most once a day, off the GTK thread, into
//! `$XDG_CACHE_HOME/mshell/agents-prices.json`; a failed day keeps the
//! previous table and is retried an hour later.
//!
//! Costs are estimates from token counts — for a subscription they are a
//! measure of value, not a bill. A model the table doesn't know (and has
//! no family sibling for) has no cost rather than a guessed one.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

pub(crate) const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
pub(crate) const FX_URL: &str = "https://api.frankfurter.dev/v1/latest?from=USD&to=EUR";
/// A failed refresh is retried after this long.
const RETRY_MS: i64 = 60 * 60_000;

/// USD per token.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Price {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_read: f64,
    /// 5-minute cache writes.
    pub(crate) cache_write: f64,
    /// 1-hour cache writes.
    pub(crate) cache_write_1h: f64,
    /// Context window, tokens.
    #[serde(default)]
    pub(crate) context: Option<u64>,
}

/// Tokens of one model, split the way they are billed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BilledTokens {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write_5m: u64,
    pub(crate) cache_write_1h: u64,
}

impl BilledTokens {
    /// From a transcript `message.usage` object. Cache writes Claude Code
    /// records as 1-hour (`cache_creation.ephemeral_1h_input_tokens`) cost
    /// more; the rest of `cache_creation_input_tokens` count as 5-minute.
    pub(crate) fn from_usage(u: &Value) -> Self {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let write = n("cache_creation_input_tokens");
        let w1h = u
            .pointer("/cache_creation/ephemeral_1h_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(write);
        BilledTokens {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write_5m: write - w1h,
            cache_write_1h: w1h,
        }
    }

    pub(crate) fn add(&mut self, o: &BilledTokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write_5m += o.cache_write_5m;
        self.cache_write_1h += o.cache_write_1h;
    }

    pub(crate) fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    pub(crate) fn cost(&self, p: &Price) -> f64 {
        self.input as f64 * p.input
            + self.output as f64 * p.output
            + self.cache_read as f64 * p.cache_read
            + self.cache_write_5m as f64 * p.cache_write
            + self.cache_write_1h as f64 * p.cache_write_1h
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PriceTable {
    /// `YYYY-MM-DD` of the last good download.
    #[serde(default)]
    pub(crate) updated: Option<String>,
    /// Model id (snapshot date dropped) → price.
    #[serde(default)]
    pub(crate) models: BTreeMap<String, Price>,
    /// EUR per USD.
    #[serde(default)]
    pub(crate) eur: Option<f64>,
}

/// `claude-sonnet-4-5-20250929` → `claude-sonnet-4-5`.
pub(crate) fn strip_snapshot(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => id,
    }
}

/// `claude-opus-5-5` → `("opus", [5, 5])`; non-numeric parts sort first.
fn family_version(id: &str) -> Option<(&str, Vec<i64>)> {
    let mut parts = id.strip_prefix("claude-")?.split('-');
    let family = parts.next().filter(|f| !f.is_empty())?;
    Some((family, parts.map(|p| p.parse().unwrap_or(-1)).collect()))
}

/// Anthropic chat models from LiteLLM's `model_prices_and_context_window.json`.
pub(crate) fn parse_litellm(body: &str) -> Option<BTreeMap<String, Price>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out = BTreeMap::new();
    for (key, m) in v.as_object()? {
        let f = |k: &str| m.get(k).and_then(Value::as_f64);
        if m.get("litellm_provider").and_then(Value::as_str) != Some("anthropic")
            || m.get("mode").and_then(Value::as_str) != Some("chat")
            || !key.starts_with("claude-")
            || key.contains(['/', '.', ':', '@'])
        {
            continue;
        }
        let Some(input) = f("input_cost_per_token").filter(|p| *p > 0.0) else {
            continue;
        };
        let write = f("cache_creation_input_token_cost").unwrap_or(0.0);
        out.insert(
            strip_snapshot(key).to_string(),
            Price {
                input,
                output: f("output_cost_per_token").unwrap_or(0.0),
                cache_read: f("cache_read_input_token_cost").unwrap_or(0.0),
                cache_write: write,
                cache_write_1h: f("cache_creation_input_token_cost_above_1hr").unwrap_or(write),
                context: m.get("max_input_tokens").and_then(Value::as_u64),
            },
        );
    }
    (!out.is_empty()).then_some(out)
}

/// `{"rates":{"EUR":0.89}}` → `0.89`.
pub(crate) fn parse_fx(body: &str) -> Option<f64> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.pointer("/rates/EUR")
        .and_then(Value::as_f64)
        .filter(|r| *r > 0.0)
}

impl PriceTable {
    /// The price of `model`: its own row, else the newest model of the
    /// same family (`claude-opus-9` before LiteLLM lists it costs like
    /// the newest Opus it does list). `None` for anything else.
    pub(crate) fn price(&self, model: &str) -> Option<&Price> {
        let id = strip_snapshot(model);
        if let Some(p) = self.models.get(id) {
            return Some(p);
        }
        let (family, _) = family_version(id)?;
        self.models
            .iter()
            .filter_map(|(k, p)| family_version(k).map(|(f, v)| (f, v, p)))
            .filter(|(f, _, _)| *f == family)
            .max_by(|a, b| a.1.cmp(&b.1))
            .map(|(_, _, p)| p)
    }

    /// Cost of `tokens` per model, `None` when no model of it is priced.
    pub(crate) fn cost<'a>(
        &self,
        tokens: impl IntoIterator<Item = (&'a str, &'a BilledTokens)>,
    ) -> Option<f64> {
        let mut any = false;
        let mut sum = 0.0;
        for (model, t) in tokens {
            if let Some(p) = self.price(model) {
                any = true;
                sum += t.cost(p);
            }
        }
        any.then_some(sum)
    }
}

/// Which currency costs are shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Currency {
    #[default]
    Usd,
    Eur,
}

/// `$1.23` / `€1.10`; `None` when EUR is asked for and no rate is known
/// (never shown as dollars with a euro sign).
pub(crate) fn money(usd: f64, currency: Currency, eur_rate: Option<f64>) -> Option<String> {
    let (sym, v) = match currency {
        Currency::Usd => ("$", usd),
        Currency::Eur => ("€", usd * eur_rate?),
    };
    Some(if v >= 100.0 {
        format!("{sym}{v:.0}")
    } else {
        format!("{sym}{v:.2}")
    })
}

// ── The shared table ───────────────────────────────────────────────────

struct Shared {
    table: PriceTable,
    loaded: bool,
    last_try_ms: i64,
}

static TABLE: LazyLock<RwLock<Shared>> = LazyLock::new(|| {
    RwLock::new(Shared {
        table: PriceTable::default(),
        loaded: false,
        last_try_ms: 0,
    })
});

fn cache_path() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("mshell")
        .join("agents-prices.json")
}

/// The current table (a clone; it is small).
pub(crate) fn current() -> PriceTable {
    TABLE.read().map(|s| s.table.clone()).unwrap_or_default()
}

/// Whether `table` is due for a download on `today` (`YYYY-MM-DD`).
pub(crate) fn due(table: &PriceTable, today: &str, now_ms: i64, last_try_ms: i64) -> bool {
    table.updated.as_deref() != Some(today) && now_ms - last_try_ms >= RETRY_MS
}

/// Load the cached table and download a fresh one when it is from an
/// earlier day. Blocking (disk + network): call off the GTK thread.
pub(crate) fn ensure_fresh(now_ms: i64) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let (table, last_try) = {
        let Ok(mut s) = TABLE.write() else { return };
        if !s.loaded {
            s.loaded = true;
            if let Some(t) = std::fs::read_to_string(cache_path())
                .ok()
                .and_then(|raw| serde_json::from_str::<PriceTable>(&raw).ok())
            {
                s.table = t;
            }
        }
        (s.table.clone(), s.last_try_ms)
    };
    if !due(&table, &today, now_ms, last_try) {
        return;
    }
    if let Ok(mut s) = TABLE.write() {
        s.last_try_ms = now_ms;
    }
    let mut next = table;
    match get(LITELLM_URL).as_deref().and_then(parse_litellm) {
        Some(models) => {
            next.models = models;
            next.updated = Some(today);
        }
        None => tracing::debug!("agents: model price table unavailable"),
    }
    if let Some(r) = get(FX_URL).as_deref().and_then(parse_fx) {
        next.eur = Some(r);
    }
    if let Ok(json) = serde_json::to_vec(&next) {
        let path = cache_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = margo_fs::atomic_write::atomic_write(&path, &json) {
            tracing::debug!(error = %e, "agents: price cache not written");
        }
    }
    if let Ok(mut s) = TABLE.write() {
        s.table = next;
    }
}

fn get(url: &str) -> Option<String> {
    ureq::get(url)
        .timeout(std::time::Duration::from_secs(20))
        .set("User-Agent", concat!("mshell/", env!("CARGO_PKG_VERSION")))
        .call()
        .ok()?
        .into_string()
        .ok()
}

#[cfg(test)]
pub(crate) fn test_table() -> PriceTable {
    PriceTable {
        updated: Some("2026-11-05".into()),
        models: parse_litellm(LITELLM_SAMPLE).unwrap_or_default(),
        eur: Some(0.89095),
    }
}

/// Rows of the real LiteLLM table downloaded 2026-11-05 (fields trimmed
/// to the ones read, plus an OpenAI and a Bedrock row that must be
/// skipped).
#[cfg(test)]
pub(crate) const LITELLM_SAMPLE: &str = r#"{
"claude-opus-5-5": {"litellm_provider": "anthropic", "mode": "chat", "input_cost_per_token": 4e-06, "output_cost_per_token": 2e-05, "cache_creation_input_token_cost": 5e-06, "cache_creation_input_token_cost_above_1hr": 8e-06, "cache_read_input_token_cost": 2e-07, "max_input_tokens": 1000000},
"claude-sonnet-5": {"litellm_provider": "anthropic", "mode": "chat", "input_cost_per_token": 2e-06, "output_cost_per_token": 1e-05, "cache_creation_input_token_cost": 2.5e-06, "cache_creation_input_token_cost_above_1hr": 4e-06, "cache_read_input_token_cost": 2e-07, "max_input_tokens": 1000000},
"claude-sonnet-4-5-20250929": {"litellm_provider": "anthropic", "mode": "chat", "input_cost_per_token": 3e-06, "output_cost_per_token": 1.5e-05, "cache_creation_input_token_cost": 3.75e-06, "cache_creation_input_token_cost_above_1hr": 6e-06, "cache_read_input_token_cost": 3e-07, "max_input_tokens": 200000},
"claude-haiku-5-5": {"litellm_provider": "anthropic", "mode": "chat", "input_cost_per_token": 1e-07, "output_cost_per_token": 5e-07, "cache_creation_input_token_cost": 1.25e-07, "cache_creation_input_token_cost_above_1hr": 2e-07, "cache_read_input_token_cost": 1e-08, "max_input_tokens": 1000000},
"claude-mythos-preview": {"litellm_provider": "anthropic", "mode": "chat", "input_cost_per_token": 1e-05, "output_cost_per_token": 5e-05, "cache_creation_input_token_cost": 1.25e-05, "cache_creation_input_token_cost_above_1hr": 2e-05, "cache_read_input_token_cost": 1e-06, "max_input_tokens": 1000000},
"gpt-4o": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 2.5e-06, "output_cost_per_token": 1e-05, "cache_read_input_token_cost": 1.25e-06, "max_input_tokens": 128000},
"anthropic.claude-3-5-haiku-20241022-v1:0": {"litellm_provider": "bedrock", "mode": "chat", "input_cost_per_token": 8e-07}
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn litellm_keeps_anthropic_chat_models_without_snapshot_dates() {
        let m = parse_litellm(LITELLM_SAMPLE).unwrap();
        let keys: Vec<_> = m.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "claude-haiku-5-5",
                "claude-mythos-preview",
                "claude-opus-5-5",
                "claude-sonnet-4-5",
                "claude-sonnet-5"
            ]
        );
        let opus = m["claude-opus-5-5"];
        assert_eq!((opus.input, opus.cache_write_1h), (4e-06, 8e-06));
        assert_eq!(opus.context, Some(1_000_000));
        assert!(parse_litellm("{}").is_none());
        assert!(parse_litellm("<html>").is_none());
    }

    #[test]
    fn unknown_ids_fall_back_to_the_newest_of_their_family() {
        let t = test_table();
        assert_eq!(t.price("claude-sonnet-4-5-20250929").unwrap().input, 3e-06);
        // No claude-opus-6 row: the newest Opus listed (5-5) stands in.
        assert_eq!(t.price("claude-opus-6").unwrap().input, 4e-06);
        // sonnet-5 (5) is newer than sonnet-4-5 (4, 5).
        assert_eq!(t.price("claude-sonnet-9").unwrap().input, 2e-06);
        assert!(t.price("gpt-4o").is_none());
        assert!(t.price("<synthetic>").is_none());
        assert!(t.price("sonnet").is_none());
    }

    #[test]
    fn usage_splits_one_hour_cache_writes() {
        // Real `message.usage` of a claude-opus-5-5 turn (2026-11-05).
        let u: Value = serde_json::from_str(r#"{"input_tokens":2,"cache_creation_input_tokens":322,"cache_read_input_tokens":376689,"output_tokens":123,"cache_creation":{"ephemeral_1h_input_tokens":322,"ephemeral_5m_input_tokens":0}}"#).unwrap();
        let t = BilledTokens::from_usage(&u);
        assert_eq!((t.cache_write_5m, t.cache_write_1h), (0, 322));
        assert_eq!(t.total(), 2 + 322 + 376689 + 123);
        let p = test_table();
        let cost = t.cost(p.price("claude-opus-5-5").unwrap());
        let want = 2.0 * 4e-06 + 123.0 * 2e-05 + 376689.0 * 2e-07 + 322.0 * 8e-06;
        assert!((cost - want).abs() < 1e-12);
        let none = p.cost([("gpt-4o", &t)]);
        assert_eq!(none, None);
    }

    #[test]
    fn fx_and_money() {
        // Real Frankfurter reply (2026-11-05).
        let r =
            parse_fx(r#"{"amount":1.0,"base":"USD","date":"2026-11-05","rates":{"EUR":0.89095}}"#);
        assert_eq!(r, Some(0.89095));
        assert_eq!(parse_fx(r#"{"rates":{}}"#), None);
        assert_eq!(money(1.234, Currency::Usd, None).as_deref(), Some("$1.23"));
        assert_eq!(money(10.0, Currency::Eur, r).as_deref(), Some("€8.91"));
        assert_eq!(money(10.0, Currency::Eur, None), None);
        assert_eq!(money(250.4, Currency::Usd, None).as_deref(), Some("$250"));
    }

    #[test]
    fn downloads_once_a_day_and_retries_hourly() {
        let mut t = PriceTable::default();
        assert!(due(&t, "2026-11-05", RETRY_MS, 0));
        t.updated = Some("2026-11-05".into());
        assert!(!due(&t, "2026-11-05", 10 * RETRY_MS, 0));
        assert!(due(&t, "2026-11-06", 10 * RETRY_MS, 0));
        assert!(!due(&t, "2026-11-06", 10 * RETRY_MS, 10 * RETRY_MS - 1));
    }
}
