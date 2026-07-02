//! End-to-end runtime check: load the `hello-guest` WASM component, call
//! `view`/`update`, and assert the node tree round-trips (W1+W2). The
//! `capabilities` test drives the W3 host imports (`get-setting` + `http`)
//! against a local one-shot HTTP server — no external network.
//!
//! Build the fixture first:
//!   (cd tests/fixtures/hello-guest && cargo build --target wasm32-wasip2 --release)
//! then: cargo test -p mshell-plugin-host --features wasm
#![cfg(feature = "wasm")]

use mshell_plugin_host::{PluginCapabilities, PluginRuntime, UiEvent, UiEventKind, UiKind};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hello-guest/target/wasm32-wasip2/release/hello_guest.wasm")
}

fn sdk_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sdk-guest/target/wasm32-wasip2/release/sdk_guest.wasm")
}

fn assistant_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/assistant-guest/target/wasm32-wasip2/release/assistant_guest.wasm")
}

/// Serve one canned HTTP/1.1 response from a fresh local socket; returns the
/// bound address. The body is sent then the connection closes.
fn serve_once(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let handle = std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(response.as_bytes());
        }
    });
    (addr, handle)
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
        .instantiate("hello", &path, HashMap::new(), PluginCapabilities::all())
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
        .instantiate("hello", &path, settings, PluginCapabilities::all())
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

/// Deny-by-default: a plugin instantiated with **no** granted capabilities must
/// not reach the network. The host returns a capability error instead of
/// performing the `http` call, so the server is never contacted and the fetched
/// body never appears in the rendered tree.
#[test]
fn denies_network_without_capability() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let path = fixture();
    if !path.exists() {
        eprintln!("skip: guest fixture not built ({})", path.display());
        return;
    }

    // A server that *would* answer if the (denied) request ever arrived. The
    // accept is non-blocking and polled briefly so the test can never hang when
    // — as expected — no connection is made.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let addr = listener.local_addr().expect("addr");
    let hit = Arc::new(AtomicBool::new(false));
    let hit_srv = hit.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..20 {
            match listener.accept() {
                Ok((mut sock, _)) => {
                    hit_srv.store(true, Ordering::SeqCst);
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf);
                    let _ = sock.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world",
                    );
                    return;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    });

    let rt = PluginRuntime::new().expect("runtime");
    let mut settings = HashMap::new();
    settings.insert("url".to_string(), format!("http://{addr}/"));
    let mut inst = rt
        // No capabilities granted → the http host call is refused.
        .instantiate("hello", &path, settings, PluginCapabilities::default())
        .expect("instantiate");

    inst.view().expect("view");
    let after = inst
        .update(&UiEvent {
            id: "caps".into(),
            kind: UiEventKind::Click,
            value: String::new(),
        })
        .expect("update");

    server.join().ok();
    assert!(
        !hit.load(Ordering::SeqCst),
        "network was reached despite no 'network' capability"
    );
    assert!(
        !after
            .iter()
            .any(|n| n.kind == UiKind::Markdown && n.text.contains("hello world")),
        "the fetched body must not appear when 'network' is denied"
    );
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
        .instantiate("hello", &path, settings, PluginCapabilities::all())
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

/// W5: the SDK-built chat guest (`mplugin-sdk` — `Component` + `El` builder +
/// `export_component!`) loads and runs a full chat turn: submit a line, stream
/// the reply into an "ai" bubble. Proves the authoring SDK end to end.
#[test]
fn sdk_chat_guest_runs_a_turn() {
    let path = sdk_fixture();
    if !path.exists() {
        eprintln!("skip: sdk-guest fixture not built ({})", path.display());
        return;
    }

    let (addr, server) = serve_once("hello from the model");

    let rt = PluginRuntime::new().expect("runtime");
    let mut settings = HashMap::new();
    settings.insert("url".to_string(), format!("http://{addr}/"));
    let mut inst = rt
        .instantiate("chat", &path, settings, PluginCapabilities::all())
        .expect("instantiate");

    // Initial UI: an entry to type into, inside the SDK-built tree.
    let nodes = inst.view().expect("view");
    assert!(
        nodes
            .iter()
            .any(|n| n.kind == UiKind::Entry && n.id == "input"),
        "expected the chat entry"
    );

    // Submit a line → a "you" bubble appears and a streamed reply starts.
    let after = inst
        .update(&UiEvent {
            id: "input".into(),
            kind: UiEventKind::Submit,
            value: "hi there".into(),
        })
        .expect("submit");
    assert!(
        after.iter().any(|n| n.kind == UiKind::Markdown
            && n.text.contains("you:")
            && n.text.contains("hi there")),
        "expected the user's line as a bubble"
    );
    assert!(inst.streams_active(), "a reply stream should be in flight");

    // Drain the streamed reply.
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
    assert!(
        tree.iter().any(|n| n.kind == UiKind::Markdown
            && n.text.contains("ai:")
            && n.text.contains("hello from the model")),
        "expected the streamed reply in an ai bubble, got: {:?}",
        tree.iter().map(|n| &n.text).collect::<Vec<_>>()
    );

    server.join().ok();
}

/// W5 real port: the actual assistant-panel chat (SDK guest with Gemini SSE
/// parsing) loads and assembles a token stream. We serve a Gemini-shaped
/// `alt=sse` response from a local socket and point the plugin's `endpoint`
/// setting at it.
#[test]
fn assistant_guest_streams_gemini_sse() {
    let path = assistant_fixture();
    if !path.exists() {
        eprintln!(
            "skip: assistant-guest fixture not built ({})",
            path.display()
        );
        return;
    }

    // Two SSE events, each a Gemini generateContent chunk with a text delta.
    let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n\
               data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\", world\"}]}}]}\n\n";
    let (addr, server) = serve_once(sse);

    let rt = PluginRuntime::new().expect("runtime");
    let mut settings = HashMap::new();
    settings.insert("endpoint".to_string(), format!("http://{addr}"));
    settings.insert("model".to_string(), "gemini-2.5-flash".to_string());
    settings.insert("api_key".to_string(), "test-key".to_string());
    let mut inst = rt
        .instantiate(
            "assistant-panel",
            &path,
            settings,
            PluginCapabilities::all(),
        )
        .expect("instantiate");
    inst.view().expect("view");

    inst.update(&UiEvent {
        id: "input".into(),
        kind: UiEventKind::Submit,
        value: "selam".into(),
    })
    .expect("submit");
    assert!(inst.streams_active(), "a reply stream should be in flight");

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
    assert!(
        tree.iter().any(|n| n.kind == UiKind::Markdown
            && n.text.contains("ai:")
            && n.text.contains("Hello, world")),
        "expected the SSE deltas assembled into the ai bubble, got: {:?}",
        tree.iter().map(|n| &n.text).collect::<Vec<_>>()
    );

    server.join().ok();
}
