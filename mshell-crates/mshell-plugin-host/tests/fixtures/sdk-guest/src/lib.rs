//! A streaming chat plugin written with the authoring SDK (`mplugin-sdk`).
//! Proves the SDK end to end: the `Component` trait, the `El` builder, the
//! `export_component!` macro, host capabilities, and the streaming event loop.
//!
//! Shape: a scrollable log of markdown bubbles above a text entry. Submitting
//! the entry appends a "you" bubble, kicks off a streamed request to the `url`
//! setting, and grows an "ai" bubble as chunks arrive.

use mplugin_sdk::{export_component, host, Component, El, Event, EventKind};
use std::cell::RefCell;

struct Msg {
    role: &'static str,
    text: String,
}

thread_local! {
    static LOG: RefCell<Vec<Msg>> = const { RefCell::new(Vec::new()) };
}

struct Chat;

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

impl Component for Chat {
    fn view() -> El {
        view_tree()
    }

    fn update(ev: Event) -> El {
        match ev.kind {
            // The user pressed Enter in the entry: record their line, open an
            // empty assistant bubble, and start streaming the reply.
            EventKind::Submit if ev.id == "input" && !ev.value.is_empty() => {
                let prompt = ev.value.clone();
                LOG.with(|log| {
                    let mut log = log.borrow_mut();
                    log.push(Msg {
                        role: "you",
                        text: prompt,
                    });
                    log.push(Msg {
                        role: "ai",
                        text: String::new(),
                    });
                });
                let url = host::get_setting("url");
                let _ = host::http_start(&host::HttpRequest {
                    method: "GET".into(),
                    url,
                    headers: vec![],
                    body: String::new(),
                });
            }
            // A chunk of the streamed reply — append it to the open ai bubble.
            EventKind::StreamChunk => {
                LOG.with(|log| {
                    if let Some(last) = log.borrow_mut().last_mut() {
                        last.text.push_str(&ev.value);
                    }
                });
            }
            _ => {}
        }
        view_tree()
    }
}

export_component!(Chat);
