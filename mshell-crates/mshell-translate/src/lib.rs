//! GTK-free translation engine for margo.
//!
//! Two providers: a **key-free Google Translate** (the public dict-widget
//! endpoint `translate.googleapis.com/translate_a/single` — no API key,
//! no account) as the default so the feature works with zero setup, and an
//! optional **DeepL** upgrade for users who bring their own key. No GTK, no
//! async runtime — [`translate`] runs on the calling thread (a worker
//! thread in the UI), same shape as `mshell-ai`'s `chat_stream`.
//!
//! Also home to [`looks_sensitive`], a best-effort screen for secret-shaped
//! text (API keys, JWTs, high-entropy tokens) — checked by callers before
//! text ever leaves the machine, since both providers are third-party
//! services.

pub mod capture;
pub mod config;

use std::collections::VecDeque;
use std::time::Duration;

/// A translation provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// `translate.googleapis.com`'s key-free dict-widget endpoint.
    Google,
    DeepL,
}

impl Provider {
    pub fn parse(s: &str) -> Provider {
        match s.trim().to_lowercase().as_str() {
            "deepl" => Provider::DeepL,
            _ => Provider::Google,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::DeepL => "deepl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google Translate (no key needed)",
            Provider::DeepL => "DeepL",
        }
    }

    pub fn needs_key(self) -> bool {
        matches!(self, Provider::DeepL)
    }
}

/// Everything a request needs. Build it from the Settings values.
#[derive(Clone)]
pub struct TranslateConfig {
    pub provider: Provider,
    /// Target language code (`tr`, `en`, …).
    pub target_lang: String,
    /// Source language override; `None`/empty = auto-detect.
    pub source_lang: Option<String>,
    /// DeepL key. Zeroized on drop — unused (and never read) for `Google`.
    pub deepl_api_key: zeroize::Zeroizing<String>,
}

impl std::fmt::Debug for TranslateConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslateConfig")
            .field("provider", &self.provider)
            .field("target_lang", &self.target_lang)
            .field("source_lang", &self.source_lang)
            .field("deepl_api_key", &"[redacted]")
            .finish()
    }
}

/// A completed translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateResult {
    pub translated: String,
    /// Source language as detected/echoed by the provider (`None` if the
    /// provider didn't report one, e.g. an explicit `source_lang` override).
    pub detected_source: Option<String>,
}

const MAX_TEXT_LEN: usize = 5000;

fn google_request_url(cfg: &TranslateConfig, text: &str) -> String {
    let sl = cfg
        .source_lang
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");
    format!(
        "https://translate.googleapis.com/translate_a/single?client=dict-chrome-ex&sl={}&tl={}&dt=t&q={}",
        urlencode(sl),
        urlencode(&cfg.target_lang),
        urlencode(text)
    )
}

/// Minimal percent-encoding — just enough for arbitrary UTF-8 query text
/// (no dependency on a full URL-encoding crate for this one call site).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Google's response is `[[[translated, original, …], …], null, detected_source]`
/// — chunked per sentence when the input is long. Concatenate the chunks.
fn parse_google_response(body: &str) -> Result<TranslateResult, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad response: {e}"))?;
    let chunks = v[0]
        .as_array()
        .ok_or_else(|| "unexpected response shape".to_string())?;
    let translated: String = chunks
        .iter()
        .filter_map(|chunk| chunk[0].as_str())
        .collect();
    if translated.is_empty() {
        return Err("empty translation".into());
    }
    let detected_source = v[2].as_str().map(str::to_string);
    Ok(TranslateResult {
        translated,
        detected_source,
    })
}

fn translate_google(cfg: &TranslateConfig, text: &str) -> Result<TranslateResult, String> {
    let url = google_request_url(cfg, text);
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(429, _) => "rate limited — try again in a moment".to_string(),
            ureq::Error::Status(code, _) => format!("request failed ({code})"),
            e => format!("request failed: {e}"),
        })?;
    let body = resp
        .into_string()
        .map_err(|e| format!("read failed: {e}"))?;
    parse_google_response(&body)
}

/// Free (`:fx`-suffixed key) vs Pro DeepL host, matching their own
/// account-tier convention.
fn deepl_base(key: &str) -> &'static str {
    if key.trim_end().ends_with(":fx") {
        "https://api-free.deepl.com"
    } else {
        "https://api.deepl.com"
    }
}

fn translate_deepl(cfg: &TranslateConfig, text: &str) -> Result<TranslateResult, String> {
    let key = cfg.deepl_api_key.trim();
    if key.is_empty() {
        return Err("no DeepL API key configured".into());
    }
    let url = format!("{}/v2/translate", deepl_base(key));
    let req = ureq::post(&url)
        .timeout(Duration::from_secs(15))
        .set("authorization", &format!("DeepL-Auth-Key {key}"))
        .set("content-type", "application/json");
    let mut body = serde_json::json!({
        "text": [text],
        "target_lang": cfg.target_lang.to_uppercase(),
    });
    if let Some(sl) = cfg.source_lang.as_deref().filter(|s| !s.is_empty()) {
        body["source_lang"] = serde_json::Value::String(sl.to_uppercase());
    }
    let resp = req.send_string(&body.to_string()).map_err(|e| match e {
        ureq::Error::Status(403, _) => "invalid DeepL API key".to_string(),
        ureq::Error::Status(456, _) => "DeepL quota exceeded".to_string(),
        ureq::Error::Status(429, _) => "rate limited — try again in a moment".to_string(),
        ureq::Error::Status(code, _) => format!("request failed ({code})"),
        e => format!("request failed: {e}"),
    })?;
    let raw = resp
        .into_string()
        .map_err(|e| format!("read failed: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("bad response: {e}"))?;
    let translated = v["translations"][0]["text"]
        .as_str()
        .ok_or("unexpected response shape")?
        .to_string();
    let detected_source = v["translations"][0]["detected_source_language"]
        .as_str()
        .map(|s| s.to_lowercase());
    Ok(TranslateResult {
        translated,
        detected_source,
    })
}

/// Translate `text` per `cfg`. Rejects text over [`MAX_TEXT_LEN`] chars
/// outright (a captured selection should never be a whole document) and
/// blank input.
pub fn translate(cfg: &TranslateConfig, text: &str) -> Result<TranslateResult, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to translate".into());
    }
    if text.chars().count() > MAX_TEXT_LEN {
        return Err(format!("text too long (> {MAX_TEXT_LEN} chars)"));
    }
    match cfg.provider {
        Provider::Google => translate_google(cfg, text),
        Provider::DeepL => translate_deepl(cfg, text),
    }
}

// ── Privacy screening ────────────────────────────────────────────────────

/// Best-effort check for secret-shaped text, run *before* anything is sent
/// to a third-party provider. Not a security boundary (a determined user
/// can always paste a secret anyway) — a speed bump against *accidentally*
/// translating a copied token/password. Returns the reason when it looks
/// sensitive.
pub fn looks_sensitive(text: &str) -> Option<&'static str> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // JWT: three base64url segments separated by dots, starting "eyJ"
    // (base64 of `{"`).
    if t.starts_with("eyJ") && t.matches('.').count() >= 2 {
        return Some("looks like a JWT");
    }
    // Common provider key prefixes.
    for prefix in ["sk-", "ghp_", "gho_", "github_pat_", "AKIA", "AIza", "xox"] {
        if t.starts_with(prefix) {
            return Some("looks like an API key");
        }
    }
    // A single long token (no whitespace) that's mostly hex/base64 reads as
    // a secret/credential rather than prose worth translating.
    if !t.contains(char::is_whitespace) && t.len() >= 32 {
        let plausible = t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'));
        if plausible {
            return Some("looks like a credential/token");
        }
    }
    None
}

// ── Cache + in-flight dedup ──────────────────────────────────────────────

/// Session-lifetime cache, keyed by the exact request shape. Bounded —
/// evicts the oldest entry past [`CACHE_CAP`] rather than growing forever
/// across a long-running shell session.
const CACHE_CAP: usize = 200;

#[derive(Default)]
pub struct Cache {
    order: VecDeque<String>,
    entries: std::collections::HashMap<String, TranslateResult>,
}

fn cache_key(cfg: &TranslateConfig, text: &str) -> String {
    format!(
        "{}|{}|{}|{}",
        cfg.provider.id(),
        cfg.target_lang,
        cfg.source_lang.as_deref().unwrap_or("auto"),
        text
    )
}

impl Cache {
    pub fn new() -> Cache {
        Cache::default()
    }

    pub fn get(&self, cfg: &TranslateConfig, text: &str) -> Option<TranslateResult> {
        self.entries.get(&cache_key(cfg, text)).cloned()
    }

    pub fn insert(&mut self, cfg: &TranslateConfig, text: &str, result: TranslateResult) {
        let key = cache_key(cfg, text);
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
            while self.order.len() > CACHE_CAP {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(p: Provider) -> TranslateConfig {
        TranslateConfig {
            provider: p,
            target_lang: "tr".into(),
            source_lang: None,
            deepl_api_key: String::new().into(),
        }
    }

    #[test]
    fn provider_parse() {
        assert_eq!(Provider::parse("DeepL"), Provider::DeepL);
        assert_eq!(Provider::parse("whatever"), Provider::Google);
    }

    #[test]
    fn google_url_shape() {
        let url = google_request_url(&cfg(Provider::Google), "hello world");
        assert!(url.contains("sl=auto"));
        assert!(url.contains("tl=tr"));
        assert!(url.contains("q=hello%20world") || url.contains("q=hello+world"));
    }

    #[test]
    fn google_response_parses_chunks_and_detected_source() {
        // Real shape: sentence chunks concatenate; third element is the
        // detected source language.
        let body =
            r#"[[["Merhaba ","Hello ",null,null,1],["dünya","world",null,null,1]],null,"en"]"#;
        let r = parse_google_response(body).unwrap();
        assert_eq!(r.translated, "Merhaba dünya");
        assert_eq!(r.detected_source, Some("en".to_string()));
    }

    #[test]
    fn deepl_host_picks_free_vs_pro() {
        assert_eq!(deepl_base("abc123:fx"), "https://api-free.deepl.com");
        assert_eq!(deepl_base("abc123"), "https://api.deepl.com");
    }

    #[test]
    fn rejects_blank_and_too_long() {
        let c = cfg(Provider::Google);
        assert!(translate(&c, "   ").is_err());
        let long = "a".repeat(MAX_TEXT_LEN + 1);
        assert!(translate(&c, &long).is_err());
    }

    #[test]
    fn sensitive_detection() {
        assert_eq!(looks_sensitive("hello world"), None);
        assert!(looks_sensitive("sk-abcdefghijklmnopqrstuvwxyz123456").is_some());
        assert!(
            looks_sensitive(
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
            )
            .is_some()
        );
        assert!(looks_sensitive(&"a".repeat(40)).is_some());
        assert_eq!(looks_sensitive("short"), None);
    }

    #[test]
    fn cache_roundtrip_and_eviction() {
        let mut cache = Cache::new();
        let c = cfg(Provider::Google);
        let r = TranslateResult {
            translated: "hi".into(),
            detected_source: Some("en".into()),
        };
        assert!(cache.get(&c, "hello").is_none());
        cache.insert(&c, "hello", r.clone());
        assert_eq!(cache.get(&c, "hello"), Some(r.clone()));

        // Eviction: fill past CACHE_CAP, the first key should be gone.
        for i in 0..CACHE_CAP {
            cache.insert(&c, &format!("text{i}"), r.clone());
        }
        assert!(cache.get(&c, "hello").is_none());
    }
}
