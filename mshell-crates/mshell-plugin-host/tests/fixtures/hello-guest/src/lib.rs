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

/// A scrollable log holding one markdown bubble — the shape a chat panel uses.
fn bubble(text: &str) -> Vec<Node> {
    vec![
        Node {
            id: "root".into(),
            kind: NodeKind::Vbox,
            text: String::new(),
            children: vec!["log".into()],
        },
        Node {
            id: "log".into(),
            kind: NodeKind::Scroll,
            text: String::new(),
            children: vec!["msg".into()],
        },
        Node {
            id: "msg".into(),
            kind: NodeKind::Markdown,
            text: format!("**ai:** {text}"),
            children: vec![],
        },
    ]
}

impl Guest for HelloGuest {
    fn view() -> Vec<Node> {
        host::log(2, "hello-guest: view");
        vec![
            Node {
                id: "root".into(),
                kind: NodeKind::Vbox,
                text: String::new(),
                children: vec![
                    "greeting".into(),
                    "btn".into(),
                    "caps".into(),
                    "stream".into(),
                ],
            },
            Node {
                id: "greeting".into(),
                kind: NodeKind::Label,
                text: "Hello from WASM".into(),
                children: vec![],
            },
            Node {
                id: "btn".into(),
                kind: NodeKind::Button,
                text: "Click me".into(),
                children: vec![],
            },
            Node {
                id: "caps".into(),
                kind: NodeKind::Button,
                text: "Fetch".into(),
                children: vec![],
            },
            Node {
                id: "stream".into(),
                kind: NodeKind::Button,
                text: "Stream".into(),
                children: vec![],
            },
        ]
    }

    fn update(ev: Event) -> Vec<Node> {
        host::log(2, &format!("hello-guest: update {}", ev.id));

        // W4: host-originated stream events — append each chunk and re-render
        // the accumulated bubble.
        match ev.kind {
            EventKind::StreamChunk | EventKind::StreamEnd => {
                ACC.with(|a| a.borrow_mut().push_str(&ev.value));
                return bubble(&ACC.with(|a| a.borrow().clone()));
            }
            _ => {}
        }

        // W4: kick off a streamed request; the body arrives later via the
        // stream events above.
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
            Node {
                id: "root".into(),
                kind: NodeKind::Vbox,
                text: String::new(),
                children: vec!["greeting".into()],
            },
            Node {
                id: "greeting".into(),
                kind: NodeKind::Label,
                text: format!("clicked {}", ev.id),
                children: vec![],
            },
        ]
    }
}

export!(HelloGuest);
