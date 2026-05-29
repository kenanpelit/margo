//! Minimal WASM plugin guest — runtime-verifies the mplugins host.
//! W1/W2: implements `view`/`update` and calls the host `log` capability.
//! W3: a "caps" button exercises `get-setting` + `http` + `notify`.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../wit",
});

use crate::margo::plugin::host::{self, HttpRequest};
use crate::margo::plugin::types::NodeKind;

struct HelloGuest;

impl Guest for HelloGuest {
    fn view() -> Vec<Node> {
        host::log(2, "hello-guest: view");
        vec![
            Node {
                id: "root".into(),
                kind: NodeKind::Vbox,
                text: String::new(),
                children: vec!["greeting".into(), "btn".into(), "caps".into()],
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
        ]
    }

    fn update(ev: Event) -> Vec<Node> {
        host::log(2, &format!("hello-guest: update {}", ev.id));

        // W3: the "caps" button reads a setting, makes an HTTP request, and
        // posts a notification — exercising every capability the host exposes.
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
            return vec![
                Node {
                    id: "root".into(),
                    kind: NodeKind::Vbox,
                    text: String::new(),
                    children: vec!["out".into()],
                },
                Node {
                    id: "out".into(),
                    kind: NodeKind::Label,
                    text,
                    children: vec![],
                },
            ];
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
