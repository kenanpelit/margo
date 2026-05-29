//! The assistant-panel chat, as a real WASM plugin built with `mplugin-sdk`.
//!
//! A scrollable log of markdown bubbles over a text entry. Submitting the entry
//! appends a "you" bubble, opens an empty "ai" bubble, and streams Google
//! Gemini's reply into it token-by-token (via `http-start` + `stream-chunk`,
//! parsing the `alt=sse` event stream).
//!
//! Settings (from the declarative `[[setting]]` tier): `api_key` (secret),
//! `model` (choice), and `endpoint` (base URL — overridable to a proxy or, in
//! tests, a local server).

use mplugin_sdk::{export_component, host, Component, El, Event, EventKind};
use std::cell::RefCell;

struct Msg {
    role: &'static str, // "you" | "ai"
    text: String,
}

thread_local! {
    static LOG: RefCell<Vec<Msg>> = const { RefCell::new(Vec::new()) };
    /// Bytes of the in-flight SSE response not yet split into complete lines.
    static SSE_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com";

struct Assistant;

fn view_tree() -> El {
    let bubbles = LOG.with(|log| {
        log.borrow()
            .iter()
            .map(|m| El::markdown(format!("**{}:** {}", m.role, m.text)))
            .collect::<Vec<_>>()
    });
    El::vbox(vec![
        El::scroll(bubbles).with_id("log"),
        El::entry("input", ""),
    ])
}

/// Build the Gemini `generateContent` request body from the conversation so
/// far (excluding the trailing empty ai bubble we're about to fill).
fn request_body() -> String {
    let contents: Vec<serde_json::Value> = LOG.with(|log| {
        log.borrow()
            .iter()
            .filter(|m| !(m.role == "ai" && m.text.is_empty()))
            .map(|m| {
                let role = if m.role == "you" { "user" } else { "model" };
                serde_json::json!({ "role": role, "parts": [{ "text": m.text }] })
            })
            .collect()
    });
    serde_json::json!({ "contents": contents }).to_string()
}

/// Append a completed SSE `data:` payload's text delta to the open ai bubble.
fn consume_sse_line(line: &str) {
    let Some(payload) = line.strip_prefix("data:") else {
        return;
    };
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return;
    }
    let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) else {
        return;
    };
    if let Some(text) = json["candidates"][0]["content"]["parts"][0]["text"].as_str() {
        LOG.with(|log| {
            if let Some(last) = log.borrow_mut().last_mut() {
                last.text.push_str(text);
            }
        });
    }
}

impl Component for Assistant {
    fn view() -> El {
        view_tree()
    }

    fn update(ev: Event) -> El {
        match ev.kind {
            EventKind::Submit if ev.id == "input" && !ev.value.trim().is_empty() => {
                LOG.with(|log| {
                    let mut log = log.borrow_mut();
                    log.push(Msg {
                        role: "you",
                        text: ev.value.clone(),
                    });
                    log.push(Msg {
                        role: "ai",
                        text: String::new(),
                    });
                });
                SSE_BUF.with(|b| b.borrow_mut().clear());

                let endpoint = {
                    let e = host::get_setting("endpoint");
                    if e.trim().is_empty() {
                        DEFAULT_ENDPOINT.to_string()
                    } else {
                        e
                    }
                };
                let model = {
                    let m = host::get_setting("model");
                    if m.trim().is_empty() {
                        "gemini-2.5-flash".to_string()
                    } else {
                        m
                    }
                };
                let api_key = host::get_setting("api_key");
                let url =
                    format!("{endpoint}/v1beta/models/{model}:streamGenerateContent?alt=sse");
                let _ = host::http_start(&host::HttpRequest {
                    method: "POST".into(),
                    url,
                    headers: vec![
                        ("content-type".into(), "application/json".into()),
                        ("x-goog-api-key".into(), api_key),
                    ],
                    body: request_body(),
                });
            }
            EventKind::StreamChunk => {
                // Accumulate, then drain complete lines; keep any partial line.
                let lines: Vec<String> = SSE_BUF.with(|b| {
                    let mut buf = b.borrow_mut();
                    buf.push_str(&ev.value);
                    let mut complete = Vec::new();
                    while let Some(nl) = buf.find('\n') {
                        let line: String = buf.drain(..=nl).collect();
                        complete.push(line.trim_end().to_string());
                    }
                    complete
                });
                for line in lines {
                    consume_sse_line(&line);
                }
            }
            EventKind::StreamEnd => {
                // Flush any trailing line without a newline.
                let tail = SSE_BUF.with(|b| std::mem::take(&mut *b.borrow_mut()));
                if !tail.is_empty() {
                    consume_sse_line(tail.trim_end());
                }
            }
            _ => {}
        }
        view_tree()
    }
}

export_component!(Assistant);
