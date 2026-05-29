//! Minimal WASM plugin guest — runtime-verifies the mplugins host.
//! W1/W2: implements `view`/`update` and calls the host `log` capability.
//! W3: a "caps" button exercises `get-setting` + `http` + `notify`.
//! W4: a "stream" button exercises `http-start` + the stream-chunk events,
//! accumulating chunks into a markdown bubble.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../wit",
});

use crate::exports::margo::plugin::guest::Guest;
use crate::margo::plugin::host::{self, HttpRequest};
use crate::margo::plugin::types::{Event, EventKind, Node, NodeKind};
use std::cell::RefCell;

// Streamed-response accumulator. wasm guests are single-threaded, so a plain
// thread-local is fine.
thread_local! {
    static ACC: RefCell<String> = RefCell::new(String::new());
}

struct HelloGuest;

/// Build one node (class defaulted empty).
fn node(id: &str, kind: NodeKind, text: impl Into<String>, children: &[&str]) -> Node {
    Node {
        id: id.into(),
        kind,
        text: text.into(),
        children: children.iter().map(|s| (*s).into()).collect(),
        class: String::new(),
    }
}

/// A scrollable log holding one markdown bubble — the shape a chat panel uses.
fn bubble(text: &str) -> Vec<Node> {
    vec![
        node("root", NodeKind::Vbox, "", &["log"]),
        node("log", NodeKind::Scroll, "", &["msg"]),
        node("msg", NodeKind::Markdown, format!("**ai:** {text}"), &[]),
    ]
}

impl Guest for HelloGuest {
    fn view() -> Vec<Node> {
        host::log(2, "hello-guest: view");
        vec![
            node(
                "root",
                NodeKind::Vbox,
                "",
                &["greeting", "btn", "caps", "stream"],
            ),
            node("greeting", NodeKind::Label, "Hello from WASM", &[]),
            node("btn", NodeKind::Button, "Click me", &[]),
            node("caps", NodeKind::Button, "Fetch", &[]),
            node("stream", NodeKind::Button, "Stream", &[]),
        ]
    }

    fn update(ev: Event) -> Vec<Node> {
        host::log(2, &format!("hello-guest: update {}", ev.id));

        // W4: host-originated stream events — append each chunk and re-render.
        match ev.kind {
            EventKind::StreamChunk | EventKind::StreamEnd => {
                ACC.with(|a| a.borrow_mut().push_str(&ev.value));
                return bubble(&ACC.with(|a| a.borrow().clone()));
            }
            _ => {}
        }

        // W4: kick off a streamed request; the body arrives via stream events.
        if ev.id == "stream" {
            ACC.with(|a| a.borrow_mut().clear());
            let url = host::get_setting("url");
            let _ = host::http_start(&HttpRequest {
                method: "GET".into(),
                url,
                headers: vec![],
                body: String::new(),
            });
            return bubble("…");
        }

        // W3: blocking request + notification, rendered as a bubble.
        if ev.id == "caps" {
            let url = host::get_setting("url");
            host::notify("hello-guest", "fetching");
            let text = match host::http(&HttpRequest {
                method: "GET".into(),
                url,
                headers: vec![],
                body: String::new(),
            }) {
                Ok(resp) => format!("{} {}", resp.status, resp.body),
                Err(e) => format!("error: {e}"),
            };
            return bubble(&text);
        }

        vec![
            node("root", NodeKind::Vbox, "", &["greeting"]),
            node("greeting", NodeKind::Label, format!("clicked {}", ev.id), &[]),
        ]
    }
}

export!(HelloGuest);
