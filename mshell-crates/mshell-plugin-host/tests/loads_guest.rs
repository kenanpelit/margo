//! End-to-end runtime check: load the `hello-guest` WASM component, call
//! `view`/`update`, and assert the node tree round-trips (W1+W2). The
//! `capabilities` test drives the W3 host imports (`get-setting` + `http`)
//! against a local one-shot HTTP server — no external network.
//!
//! Build the fixture first:
//!   (cd tests/fixtures/hello-guest && cargo build --target wasm32-wasip2 --release)
//! then: cargo test -p mshell-plugin-host --features wasm
#![cfg(feature = "wasm")]

use mshell_plugin_host::{PluginRuntime, UiEvent, UiEventKind, UiKind};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

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
    let mut inst = rt
        .instantiate("hello", &path, HashMap::new())
        .expect("instantiate");

    // Initial render.
    let nodes = inst.view().expect("view");
    let root = nodes.iter().find(|n| n.id == "root").expect("a root node");
    assert_eq!(root.kind, UiKind::VBox);
    assert_eq!(
        root.children,
        vec![
            "greeting".to_string(),
            "btn".to_string(),
            "caps".to_string(),
            "stream".to_string()
        ]
    );
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

/// W3: the guest reads a setting (`url`), calls `http`, and renders the
/// response. We serve one canned HTTP/1.1 response from a local socket and
/// pass its address in via settings, so the whole capability path is exercised
/// deterministically.
#[test]
fn capabilities_get_setting_and_http() {
    let path = fixture();
    if !path.exists() {
        eprintln!("skip: guest fixture not built ({})", path.display());
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf); // drain the request line/headers
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world",
            );
        }
    });

    let rt = PluginRuntime::new().expect("runtime");
    let mut settings = HashMap::new();
    settings.insert("url".to_string(), format!("http://{addr}/"));
    let mut inst = rt
        .instantiate("hello", &path, settings)
        .expect("instantiate");

    inst.view().expect("view");
    let after = inst
        .update(&UiEvent {
            id: "caps".into(),
            kind: UiEventKind::Click,
            value: String::new(),
        })
        .expect("update");

    // The guest renders the response as a markdown bubble inside a scroll —
    // verifies the W4 rich nodes round-trip alongside the W3 capabilities.
    assert!(
        after.iter().any(|n| n.kind == UiKind::Scroll),
        "expected a scroll node"
    );
    let msg = after
        .iter()
        .find(|n| n.kind == UiKind::Markdown)
        .expect("a markdown bubble");
    assert!(
        msg.text.contains("200") && msg.text.contains("hello world"),
        "expected the fetched body in the bubble, got: {:?}",
        msg.text
    );

    server.join().ok();
}

/// W4: `http-start` delivers the body off-thread as `stream-chunk` events that
/// `pump` feeds back to the guest. We drive `pump` manually here — the role the
/// GTK timeout plays in the live shell.
#[test]
fn streaming_http_delivers_chunks() {
    let path = fixture();
    if !path.exists() {
        eprintln!("skip: guest fixture not built ({})", path.display());
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\ntok-a tok-b",
            );
        }
    });

    let rt = PluginRuntime::new().expect("runtime");
    let mut settings = HashMap::new();
    settings.insert("url".to_string(), format!("http://{addr}/"));
    let mut inst = rt
        .instantiate("hello", &path, settings)
        .expect("instantiate");
    inst.view().expect("view");

    // Kick off the streamed request.
    let started = inst
        .update(&UiEvent {
            id: "stream".into(),
            kind: UiEventKind::Click,
            value: String::new(),
        })
        .expect("update");
    assert!(
        started.iter().any(|n| n.kind == UiKind::Markdown),
        "expected a loading bubble"
    );
    assert!(inst.streams_active(), "a stream should be in flight");

    // Drain chunks until the worker finishes (then a final flush).
    let mut last = None;
    for _ in 0..200 {
        if let Some(tree) = inst.pump().expect("pump") {
            last = Some(tree);
        }
        if !inst.streams_active() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if let Some(tree) = inst.pump().expect("pump") {
        last = Some(tree);
    }

    let tree = last.expect("at least one pumped render");
    let msg = tree
        .iter()
        .find(|n| n.kind == UiKind::Markdown)
        .expect("a bubble");
    assert!(
        msg.text.contains("tok-a tok-b"),
        "expected the streamed body in the bubble, got: {:?}",
        msg.text
    );

    server.join().ok();
}
