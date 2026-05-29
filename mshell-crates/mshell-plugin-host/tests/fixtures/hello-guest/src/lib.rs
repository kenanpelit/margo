//! Minimal WASM plugin guest — runtime-verifies the mplugins host (W1/W2):
//! it implements `view`/`update` and calls the host `log` capability.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../wit",
});

use crate::margo::plugin::types::NodeKind;

struct HelloGuest;

impl Guest for HelloGuest {
    fn view() -> Vec<Node> {
        margo::plugin::host::log(2, "hello-guest: view");
        vec![
            Node {
                id: "root".into(),
                kind: NodeKind::Vbox,
                text: String::new(),
                children: vec!["greeting".into(), "btn".into()],
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
        ]
    }

    fn update(ev: Event) -> Vec<Node> {
        margo::plugin::host::log(2, &format!("hello-guest: update {}", ev.id));
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
