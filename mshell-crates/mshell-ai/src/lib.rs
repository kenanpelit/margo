//! GTK-free AI chat engine for margo.
//!
//! A native port of the `assistant-panel` WASM plugin: multi-provider request
//! building, blocking-but-streaming transport over [`ureq`] (SSE for
//! Gemini / OpenAI-compatible / Anthropic, NDJSON for Ollama), and **live
//! model discovery** from each provider's list-models endpoint with a curated
//! fallback. No GTK, no async runtime — the UI runs [`chat_stream`] /
//! [`list_models`] on a worker thread and marshals results back itself.
//!
//! Providers: Gemini · OpenAI-compatible (also LocalAI / LM Studio / vLLM) ·
//! Anthropic · Ollama · Custom (alias of OpenAI for any compatible endpoint).

pub mod config;

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A chat provider. `Custom` is an OpenAI-compatible alias (LocalAI, vLLM, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    OpenAi,
    Anthropic,
    Ollama,
    Custom,
}

impl Provider {
    /// Parse the config string (tolerant of synonyms / case).
    pub fn parse(s: &str) -> Provider {
        match s.trim().to_lowercase().as_str() {
            "openai" | "openai-compatible" | "chatgpt" | "gpt" => Provider::OpenAi,
            "anthropic" | "claude" => Provider::Anthropic,
            "ollama" | "local" => Provider::Ollama,
            "custom" => Provider::Custom,
            _ => Provider::Gemini,
        }
    }

    /// The config token (what we persist / pass around).
    pub fn id(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Ollama => "ollama",
            Provider::Custom => "custom",
        }
    }

    /// Human label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Gemini => "Gemini",
            Provider::OpenAi => "OpenAI",
            Provider::Anthropic => "Anthropic",
            Provider::Ollama => "Ollama",
            Provider::Custom => "Custom (OpenAI-compatible)",
        }
    }

    /// Every selectable provider, in UI order.
    pub fn all() -> [Provider; 5] {
        [
            Provider::Gemini,
            Provider::OpenAi,
            Provider::Anthropic,
            Provider::Ollama,
            Provider::Custom,
        ]
    }

    /// Whether this provider needs an API key (Ollama / local don't).
    pub fn needs_key(self) -> bool {
        !matches!(self, Provider::Ollama | Provider::Custom)
    }

    /// Default base URL when the user leaves the endpoint override blank.
    pub fn default_endpoint(self) -> &'static str {
        match self {
            Provider::Gemini => "https://generativelanguage.googleapis.com",
            Provider::OpenAi => "https://api.openai.com",
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::Ollama => "http://localhost:11434",
            Provider::Custom => "http://localhost:8080",
        }
    }

    /// A sensible default model id for the provider.
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini-2.5-flash",
            Provider::OpenAi => "gpt-4o-mini",
            Provider::Anthropic => "claude-sonnet-4-5-20250929",
            Provider::Ollama => "llama3",
            Provider::Custom => "gpt-4o-mini",
        }
    }

    /// Curated fallback model list — used when there's no API key yet or the
    /// live list-models call fails. Kept reasonably current; the live fetch
    /// supersedes it whenever it succeeds.
    pub fn fallback_models(self) -> &'static [&'static str] {
        match self {
            Provider::Gemini => &[
                "gemini-2.5-flash",
                "gemini-2.5-pro",
                "gemini-2.0-flash",
                "gemini-1.5-flash",
                "gemini-1.5-pro",
            ],
            Provider::OpenAi | Provider::Custom => &[
                "gpt-4o",
                "gpt-4o-mini",
                "gpt-4.1",
                "gpt-4.1-mini",
                "o3-mini",
                "gpt-3.5-turbo",
            ],
            Provider::Anthropic => &[
                "claude-sonnet-4-5-20250929",
                "claude-opus-4-1-20250805",
                "claude-3-5-haiku-20241022",
                "claude-3-5-sonnet-20241022",
            ],
            Provider::Ollama => &["llama3", "qwen2.5", "mistral", "phi3", "gemma2"],
        }
    }
}

/// Conversation role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// One chat turn.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            text: text.into(),
        }
    }
    pub fn assistant(text: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            text: text.into(),
        }
    }
}

/// Everything a request needs. Build it from the Settings values.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub provider: Provider,
    pub model: String,
    pub api_key: String,
    /// Endpoint override; blank → [`Provider::default_endpoint`].
    pub endpoint: String,
    pub temperature: f64,
    pub max_tokens: u32,
    pub system_prompt: String,
}

impl AiConfig {
    fn base(&self) -> String {
        let e = self.endpoint.trim().trim_end_matches('/');
        if e.is_empty() {
            self.provider.default_endpoint().to_string()
        } else {
            e.to_string()
        }
    }

    fn model_or_default(&self) -> String {
        let m = self.model.trim();
        if m.is_empty() {
            self.provider.default_model().to_string()
        } else {
            m.to_string()
        }
    }
}

/// A built HTTP request.
struct Req {
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn role_str(role: Role, assistant: &str) -> &'static str {
    match role {
        Role::User => "user",
        Role::System => "system",
        Role::Assistant => match assistant {
            "model" => "model",
            _ => "assistant",
        },
    }
}

fn build_request(cfg: &AiConfig, msgs: &[Message]) -> Req {
    match cfg.provider {
        Provider::Gemini => gemini_request(cfg, msgs),
        Provider::Anthropic => anthropic_request(cfg, msgs),
        Provider::Ollama => ollama_request(cfg, msgs),
        Provider::OpenAi | Provider::Custom => openai_request(cfg, msgs),
    }
}

fn gemini_request(cfg: &AiConfig, msgs: &[Message]) -> Req {
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        cfg.base(),
        cfg.model_or_default()
    );
    let contents: Vec<serde_json::Value> = msgs
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| {
            serde_json::json!({
                "role": role_str(m.role, "model"),
                "parts": [{"text": m.text}],
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "temperature": cfg.temperature,
            "maxOutputTokens": cfg.max_tokens,
        }
    });
    if !cfg.system_prompt.trim().is_empty() {
        body["systemInstruction"] = serde_json::json!({"parts": [{"text": cfg.system_prompt}]});
    }
    Req {
        url,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("x-goog-api-key".into(), cfg.api_key.clone()),
        ],
        body: body.to_string(),
    }
}

/// `…/v1` or `…/v4` bases get `/chat/completions`; bare bases get
/// `/v1/chat/completions` (matches DMS's heuristic).
fn openai_chat_url(base: &str) -> String {
    let versioned = base
        .rsplit('/')
        .next()
        .map(|seg| seg.starts_with('v') && seg[1..].chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false);
    if versioned {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn openai_messages(cfg: &AiConfig, msgs: &[Message]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if !cfg.system_prompt.trim().is_empty() {
        out.push(serde_json::json!({"role": "system", "content": cfg.system_prompt}));
    }
    out.extend(
        msgs.iter()
            .filter(|m| m.role != Role::System)
            .map(|m| serde_json::json!({"role": role_str(m.role, "assistant"), "content": m.text})),
    );
    out
}

fn openai_request(cfg: &AiConfig, msgs: &[Message]) -> Req {
    let body = serde_json::json!({
        "model": cfg.model_or_default(),
        "messages": openai_messages(cfg, msgs),
        "temperature": cfg.temperature,
        "max_tokens": cfg.max_tokens,
        "stream": true,
    });
    let mut headers = vec![("content-type".into(), "application/json".into())];
    if !cfg.api_key.trim().is_empty() {
        headers.push(("authorization".into(), format!("Bearer {}", cfg.api_key)));
    }
    Req {
        url: openai_chat_url(&cfg.base()),
        headers,
        body: body.to_string(),
    }
}

fn anthropic_request(cfg: &AiConfig, msgs: &[Message]) -> Req {
    let messages: Vec<serde_json::Value> = msgs
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| serde_json::json!({"role": role_str(m.role, "assistant"), "content": m.text}))
        .collect();
    let mut body = serde_json::json!({
        "model": cfg.model_or_default(),
        "messages": messages,
        "max_tokens": cfg.max_tokens,
        "temperature": cfg.temperature,
        "stream": true,
    });
    if !cfg.system_prompt.trim().is_empty() {
        body["system"] = serde_json::Value::String(cfg.system_prompt.clone());
    }
    Req {
        url: format!("{}/v1/messages", cfg.base()),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("x-api-key".into(), cfg.api_key.clone()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ],
        body: body.to_string(),
    }
}

fn ollama_request(cfg: &AiConfig, msgs: &[Message]) -> Req {
    let body = serde_json::json!({
        "model": cfg.model_or_default(),
        "messages": openai_messages(cfg, msgs),
        "stream": true,
        "options": { "temperature": cfg.temperature, "num_predict": cfg.max_tokens },
    });
    Req {
        url: format!("{}/api/chat", cfg.base()),
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.to_string(),
    }
}

/// Pull the text delta out of one parsed streaming JSON value for `provider`.
/// Returns `None` for keep-alive / non-content frames.
pub fn delta_from_json(provider: Provider, v: &serde_json::Value) -> Option<String> {
    let text = match provider {
        Provider::Gemini => v["candidates"][0]["content"]["parts"][0]["text"].as_str(),
        Provider::Anthropic => {
            if v["type"] == "content_block_delta" {
                v["delta"]["text"].as_str()
            } else {
                None
            }
        }
        Provider::Ollama => v["message"]["content"].as_str(),
        Provider::OpenAi | Provider::Custom => v["choices"][0]["delta"]["content"].as_str(),
    };
    text.filter(|t| !t.is_empty()).map(str::to_string)
}

/// Parse a transport line into a delta. Gemini / OpenAI / Anthropic use SSE
/// (`data: {…}`); Ollama streams bare NDJSON objects.
fn delta_from_line(provider: Provider, line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let json_str = if provider == Provider::Ollama {
        line
    } else {
        let payload = line.strip_prefix("data:")?.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return None;
        }
        payload
    };
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
    delta_from_json(provider, &v)
}

/// Extract a human error message from a non-streaming error body.
pub fn error_from_body(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "no response from the API — check network and endpoint".into();
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(raw) {
        for key in ["error", "detail"] {
            if let Some(msg) = json[key]["message"].as_str() {
                return msg.to_string();
            }
            if let Some(msg) = json[key].as_str() {
                return msg.to_string();
            }
        }
    }
    raw.chars().take(300).collect()
}

/// Stream a completion. Blocks the calling thread, invoking `on_token` for
/// every text delta as it arrives. `cancel` is polled between lines so the UI
/// can stop a reply mid-stream. Returns `Ok(())` on a clean finish, or an
/// `Err` with a user-facing message (bad key, quota, network, …).
pub fn chat_stream(
    cfg: &AiConfig,
    msgs: &[Message],
    cancel: &Arc<AtomicBool>,
    mut on_token: impl FnMut(&str),
) -> Result<(), String> {
    let provider = cfg.provider;
    let req = build_request(cfg, msgs);
    let mut request = ureq::post(&req.url).timeout(Duration::from_secs(120));
    for (k, v) in &req.headers {
        request = request.set(k, v);
    }
    let resp = match request.send_string(&req.body) {
        Ok(r) => r,
        // ureq returns Err for non-2xx; surface the body so the user sees
        // "invalid api key" etc. instead of a silent failure.
        Err(ureq::Error::Status(_code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(error_from_body(&body));
        }
        Err(e) => return Err(format!("request failed: {e}")),
    };

    let reader = BufReader::new(resp.into_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(line) = line else { break };
        if let Some(delta) = delta_from_line(provider, &line) {
            on_token(&delta);
        }
    }
    Ok(())
}

// ── Live model discovery ─────────────────────────────────────────────────

fn http_get_json(url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let mut req = ureq::get(url).timeout(Duration::from_secs(15));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(r) => {
            let body = r.into_string().map_err(|e| format!("read failed: {e}"))?;
            serde_json::from_str(&body).map_err(|e| format!("bad JSON: {e}"))
        }
        Err(ureq::Error::Status(_c, r)) => {
            Err(error_from_body(&r.into_string().unwrap_or_default()))
        }
        Err(e) => Err(format!("request failed: {e}")),
    }
}

/// Fetch the list of available model ids for the provider from its list-models
/// endpoint. Falls back to [`Provider::fallback_models`] on any failure (no
/// key, offline, unsupported endpoint). Result is de-duplicated + sorted.
pub fn list_models(cfg: &AiConfig) -> Vec<String> {
    fetch_models(cfg).unwrap_or_else(|_| {
        cfg.provider
            .fallback_models()
            .iter()
            .map(|s| s.to_string())
            .collect()
    })
}

/// Like [`list_models`] but surfaces the error (for a "Refresh" button that
/// wants to report why a fetch failed).
pub fn fetch_models(cfg: &AiConfig) -> Result<Vec<String>, String> {
    let base = cfg.base();
    let key = cfg.api_key.trim();
    let mut models: Vec<String> = match cfg.provider {
        Provider::Gemini => {
            let url = format!("{base}/v1beta/models?key={key}&pageSize=200");
            let v = http_get_json(&url, &[])?;
            v["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|m| {
                            m["supportedGenerationMethods"]
                                .as_array()
                                .map(|s| s.iter().any(|x| x == "generateContent"))
                                .unwrap_or(true)
                        })
                        .filter_map(|m| m["name"].as_str())
                        .map(|n| n.trim_start_matches("models/").to_string())
                        .collect()
                })
                .unwrap_or_default()
        }
        Provider::OpenAi | Provider::Custom => {
            let v = http_get_json(
                &format!("{base}/v1/models"),
                &[("authorization", &format!("Bearer {key}"))],
            )?;
            json_id_list(&v["data"])
        }
        Provider::Anthropic => {
            let v = http_get_json(
                &format!("{base}/v1/models"),
                &[("x-api-key", key), ("anthropic-version", "2023-06-01")],
            )?;
            json_id_list(&v["data"])
        }
        Provider::Ollama => {
            let v = http_get_json(&format!("{base}/api/tags"), &[])?;
            v["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["name"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        }
    };
    models.retain(|m| !m.is_empty());
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err("the provider returned no models".into());
    }
    Ok(models)
}

fn json_id_list(arr: &serde_json::Value) -> Vec<String> {
    arr.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(p: Provider) -> AiConfig {
        AiConfig {
            provider: p,
            model: String::new(),
            api_key: "k".into(),
            endpoint: String::new(),
            temperature: 0.7,
            max_tokens: 1024,
            system_prompt: String::new(),
        }
    }

    #[test]
    fn provider_parse_synonyms() {
        assert_eq!(Provider::parse("ChatGPT"), Provider::OpenAi);
        assert_eq!(Provider::parse("claude"), Provider::Anthropic);
        assert_eq!(Provider::parse("ollama"), Provider::Ollama);
        assert_eq!(Provider::parse("whatever"), Provider::Gemini);
    }

    #[test]
    fn openai_url_heuristic() {
        assert_eq!(
            openai_chat_url("https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_url("https://x/v1"),
            "https://x/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_url("https://x/v4"),
            "https://x/v4/chat/completions"
        );
    }

    #[test]
    fn gemini_request_shape() {
        let mut c = cfg(Provider::Gemini);
        c.model = "gemini-2.5-flash".into();
        let r = build_request(&c, &[Message::user("hi")]);
        assert!(r.url.contains("gemini-2.5-flash:streamGenerateContent"));
        assert!(r.headers.iter().any(|(k, _)| k == "x-goog-api-key"));
        assert!(r.body.contains("\"role\":\"user\""));
    }

    #[test]
    fn delta_parsers() {
        // OpenAI SSE
        assert_eq!(
            delta_from_line(
                Provider::OpenAi,
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}"
            ),
            Some("hi".into())
        );
        assert_eq!(delta_from_line(Provider::OpenAi, "data: [DONE]"), None);
        // Gemini SSE
        assert_eq!(
            delta_from_line(
                Provider::Gemini,
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"yo\"}]}}]}"
            ),
            Some("yo".into())
        );
        // Anthropic delta vs non-delta
        assert_eq!(
            delta_from_line(
                Provider::Anthropic,
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"x\"}}"
            ),
            Some("x".into())
        );
        assert_eq!(
            delta_from_line(Provider::Anthropic, "data: {\"type\":\"message_start\"}"),
            None
        );
        // Ollama NDJSON (no `data:` prefix)
        assert_eq!(
            delta_from_line(Provider::Ollama, "{\"message\":{\"content\":\"z\"}}"),
            Some("z".into())
        );
    }

    #[test]
    fn error_extraction() {
        assert_eq!(
            error_from_body("{\"error\":{\"message\":\"bad key\"}}"),
            "bad key"
        );
        assert!(error_from_body("").contains("no response"));
    }

    #[test]
    fn fallback_never_empty() {
        for p in Provider::all() {
            assert!(!p.fallback_models().is_empty());
        }
    }
}
