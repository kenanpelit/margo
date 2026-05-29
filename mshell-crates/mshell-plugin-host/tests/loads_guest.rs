//! End-to-end runtime check (W1+W2): load the `hello-guest` WASM component,
//! call `view`/`update`, and assert the node tree round-trips.
//!
//! Build the fixture first:
//!   (cd tests/fixtures/hello-guest && cargo build --target wasm32-wasip2 --release)
//! then: cargo test -p mshell-plugin-host --features wasm
#![cfg(feature = "wasm")]

use mshell_plugin_host::{PluginRuntime, UiEvent, UiEventKind, UiKind};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hello-guest/target/wasm32-wasip2/release/hello_guest.wasm")
}

#[test]
fn loads_and_drives_guest() {
    let path = fixture();
    if !path.exists() {
        eprintln!("skip: guest fixture not built ({})", path.display());
        return;
    }

    let rt = PluginRuntime::new().expect("runtime");
    let mut inst = rt.instantiate("hello", &path).expect("instantiate");

    // Initial render.
    let nodes = inst.view().expect("view");
    let root = nodes.iter().find(|n| n.id == "root").expect("a root node");
    assert_eq!(root.kind, UiKind::VBox);
    assert_eq!(root.children, vec!["greeting".to_string(), "btn".to_string()]);
    assert!(
        nodes
            .iter()
            .any(|n| n.kind == UiKind::Label && n.text == "Hello from WASM")
    );
    assert!(nodes.iter().any(|n| n.kind == UiKind::Button));

    // Drive an event → re-render.
    let after = inst
        .update(&UiEvent {
            id: "btn".into(),
            kind: UiEventKind::Click,
            value: String::new(),
        })
        .expect("update");
    assert!(
        after
            .iter()
            .any(|n| n.kind == UiKind::Label && n.text.contains("clicked btn"))
    );
}
