//! IPC connection lifecycle against a live `MargoState` + event loop.

use super::fixture::Fixture;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Accept one IPC connection; returns the client end and its token.
fn connect(fx: &mut Fixture) -> (UnixStream, u32) {
    let (server, client) = UnixStream::pair().expect("pair");
    let token = fx.server.state.ipc.next_token;
    fx.server.state.ipc_accept(server);
    assert!(fx.server.state.ipc.conns.contains_key(&token));
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("timeout");
    (client, token)
}

/// Read whatever the server has sent so far, after a few loop turns.
/// `None` = the server closed the connection (EOF).
fn read_available(fx: &mut Fixture, client: &mut UnixStream) -> Option<String> {
    let mut out = Vec::new();
    for _ in 0..20 {
        fx.dispatch();
        let mut chunk = [0u8; 65536];
        match client.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if !out.is_empty() {
                    break;
                }
            }
            Err(e) => panic!("read: {e}"),
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn frames(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json frame"))
        .collect()
}

#[test]
fn a_dropped_connection_is_closed_for_the_client() {
    // `ipc_drop_conn` used to forget the map entry and the WRITE source but
    // leave the READ source (and its fd) registered: the socket stayed open,
    // the client never saw EOF, and the connection leaked outside
    // MAX_IPC_CONNECTIONS.
    let mut fx = Fixture::new();
    let (mut client, token) = connect(&mut fx);

    fx.server.state.ipc_drop_conn(token);

    assert!(
        read_available(&mut fx, &mut client).is_none(),
        "the client must see EOF once margo drops its connection"
    );
}

#[test]
fn a_slow_watcher_over_the_outbound_cap_is_disconnected() {
    let mut fx = Fixture::new();
    let (mut client, token) = connect(&mut fx);

    // Nobody reads: queue more than the 4 MiB cap.
    let big = format!("{}\n", "x".repeat(5 * 1024 * 1024));
    fx.server.state.ipc_send_line(token, &big);
    assert!(!fx.server.state.ipc.conns.contains_key(&token));

    // Drain what made it into the socket before the drop, then expect EOF.
    let mut saw_eof = false;
    for _ in 0..200 {
        fx.dispatch();
        let mut chunk = [0u8; 65536];
        match client.read(&mut chunk) {
            Ok(0) => {
                saw_eof = true;
                break;
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => panic!("read: {e}"),
        }
    }
    assert!(
        saw_eof,
        "a slow watcher cut loose must see EOF and reconnect"
    );
}

#[test]
fn watch_perf_and_layers_push_real_frames() {
    // These two topics are not projections of the state snapshot; the
    // watch fan-out used to send them through `project_topic`, which
    // answered every push with "unknown topic".
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    for topic in ["perf", "layers"] {
        let (mut client, _token) = connect(&mut fx);
        client
            .write_all(format!("watch {topic}\n").as_bytes())
            .expect("write");
        let first = frames(&read_available(&mut fx, &mut client).expect("open"));
        assert_eq!(first.len(), 1, "{topic}: initial frame");
        assert!(first[0].get("error").is_none(), "{topic}: {}", first[0]);

        fx.server.state.ipc_push_watches();
        let pushed = frames(&read_available(&mut fx, &mut client).expect("open"));
        assert!(!pushed.is_empty(), "{topic}: a push frame");
        for f in pushed {
            assert!(f.get("error").is_none(), "{topic} push: {f}");
        }
    }
}

#[test]
fn every_listed_topic_is_routed() {
    // `TOPICS` gates `watch`; it must match what `get` actually serves.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    for topic in crate::ipc::topics::TOPICS {
        let v = fx.server.state.ipc_topic(topic, &["DP-1".to_string()]);
        let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
        assert!(!err.contains("unknown topic"), "{topic}: {v}");
    }
    assert!(!crate::ipc::topics::is_known_topic("bogus"));
    let v = fx.server.state.ipc_topic("bogus", &[]);
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("unknown topic"))
    );
}

#[test]
fn watching_an_unknown_topic_is_refused() {
    let mut fx = Fixture::new();
    let (mut client, token) = connect(&mut fx);
    client.write_all(b"watch bogus\n").expect("write");

    let reply = frames(&read_available(&mut fx, &mut client).expect("open"));
    assert_eq!(reply.len(), 1);
    assert!(
        reply[0]["error"]
            .as_str()
            .is_some_and(|e| e.contains("unknown topic"))
    );
    assert!(
        !fx.server
            .state
            .ipc
            .watches
            .watches
            .iter()
            .any(|w| w.token == token),
        "an unknown topic must not be registered (it would push errors forever)"
    );
}

#[test]
fn a_request_flood_is_handled_in_bounded_batches() {
    // One readable wakeup used to run every buffered request before
    // returning to the loop. Requests are now handled in batches; the rest
    // are continued from an idle callback, so all of them still get a reply.
    let mut fx = Fixture::new();
    let (mut client, token) = connect(&mut fx);
    let n = 500;
    let flood = "get layouts\n".repeat(n);
    client.write_all(flood.as_bytes()).expect("write");

    // One readable wakeup handles at most one batch.
    fx.server.state.ipc_readable_for_test(token);
    let mut first = Vec::new();
    let mut chunk = [0u8; 65536];
    while let Ok(n) = client.read(&mut chunk) {
        if n == 0 {
            break;
        }
        first.extend_from_slice(&chunk[..n]);
    }
    let first = frames(&String::from_utf8_lossy(&first)).len();
    assert_eq!(
        first,
        crate::ipc::server::MAX_LINES_PER_WAKEUP,
        "exactly one batch is answered per wakeup"
    );

    let mut got = first;
    for _ in 0..50 {
        if let Some(text) = read_available(&mut fx, &mut client) {
            got += frames(&text).len();
        }
        if got == n {
            break;
        }
    }
    assert_eq!(got, n, "every request is answered eventually");
}

/// One `dispatch …` round trip; the reply frame.
fn dispatch_reply(fx: &mut Fixture, line: &str) -> serde_json::Value {
    let (mut client, _token) = connect(fx);
    client
        .write_all(format!("dispatch {line}\n").as_bytes())
        .expect("write");
    let reply = frames(&read_available(fx, &mut client).expect("open"));
    assert_eq!(reply.len(), 1, "{line}: {reply:?}");
    reply[0].clone()
}

fn assert_refused(reply: &serde_json::Value, needle: &str) {
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"].as_str().is_some_and(|e| e.contains(needle)),
        "expected an error mentioning {needle:?}: {reply}"
    );
}

/// A refused request answers `{"ok":false,"error":…}` (what docs/ipc.md
/// promises, and what mctl and the shell check), not `{"ok":true}`.
#[test]
fn a_refused_dispatch_replies_with_the_error() {
    let mut fx = Fixture::new();
    fx.add_output("eDP-1", (1920, 1200));
    fx.add_output("DP-3", (2560, 1440));

    let r = dispatch_reply(&mut fx, "mirror_output eDP-1 DP-9");
    assert_refused(&r, "no output named DP-9");
    let r = dispatch_reply(&mut fx, "disable_output DP-9");
    assert_refused(&r, "DP-9");
    let r = dispatch_reply(&mut fx, "focuslsat");
    assert_refused(&r, "unknown action");

    let r = dispatch_reply(&mut fx, "mirror_output eDP-1 DP-3");
    assert_eq!(r["ok"], true, "{r}");
    // The mirror target is the last output left in the layout's way:
    // switching eDP-1 off ends the mirror, and DP-3 can't follow.
    let r = dispatch_reply(&mut fx, "disable_output eDP-1");
    assert_eq!(r["ok"], true, "{r}");
    let r = dispatch_reply(&mut fx, "disable_output DP-3");
    assert_refused(&r, "DP-3");
}

#[test]
fn a_failed_reload_replies_with_the_error() {
    // `mctl reload` exited 0 while margo kept the old config.
    let dir = std::env::temp_dir().join(format!("margo-ipc-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.conf");
    std::fs::write(&config, "xkb_rules_layout = no-such-layout-zz\n").unwrap();

    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.server.state.config_path = Some(config);
    fx.server.state.generations_dir = dir.join("generations");
    let r = dispatch_reply(&mut fx, "reload");
    assert_refused(&r, "reload");
    std::fs::remove_dir_all(dir).unwrap();
}


/// Subscribe a fresh connection to `watch state` and swallow its initial
/// frame plus the first push (the first push is never de-duplicated
/// against the subscribe-time frame).
fn subscribe_state(fx: &mut Fixture) -> UnixStream {
    let (mut client, _token) = connect(fx);
    client.write_all(b"watch state\n").expect("write");
    let first = frames(&read_available(fx, &mut client).expect("open"));
    assert_eq!(first.len(), 1, "initial frame");
    fx.server.state.ipc_push_watches();
    let _ = read_available(fx, &mut client);
    client
}

#[test]
fn an_unchanged_state_snapshot_is_not_pushed_again() {
    // Focus refreshes and no-op arranges mark state dirty without changing
    // anything a watcher can see; each used to push a byte-identical frame.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let mut client = subscribe_state(&mut fx);

    fx.server.state.ipc_push_watches();
    let again = frames(&read_available(&mut fx, &mut client).expect("open"));
    assert!(again.is_empty(), "identical frame pushed: {again:?}");

    fx.server.state.current_kb_layout = "Turkish".into();
    fx.server.state.ipc_push_watches();
    let changed = frames(&read_available(&mut fx, &mut client).expect("open"));
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["keyboard_layout"], "Turkish");
}

#[test]
fn state_pushes_are_rate_capped_and_the_deferred_frame_still_arrives() {
    // A drag arranges every few ms; each arrange used to push a full
    // snapshot. Pushes are capped, and the frame held back by the cap is
    // sent from a timer even if nothing else wakes the loop.
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let mut client = subscribe_state(&mut fx);

    fx.server.state.current_kb_layout = "A".into();
    fx.server.state.mark_state_dirty();
    fx.server.state.flush_ipc_if_dirty();
    fx.server.state.current_kb_layout = "B".into();
    fx.server.state.mark_state_dirty();
    fx.server.state.flush_ipc_if_dirty();

    let mut layouts = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(500) && layouts.last() != Some(&"B".to_string()) {
        for f in frames(&read_available(&mut fx, &mut client).expect("open")) {
            layouts.push(f["keyboard_layout"].as_str().unwrap_or("").to_string());
        }
    }
    assert_eq!(layouts, vec!["A".to_string(), "B".to_string()]);
    assert!(!fx.server.state.state_dirty.get(), "the deferred push ran");
}

#[test]
fn a_flush_inside_the_cap_interval_defers_the_push() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let _client = subscribe_state(&mut fx);
    fx.server.state.mark_state_dirty();
    fx.server.state.flush_ipc_if_dirty();
    fx.server.state.current_kb_layout = "C".into();
    fx.server.state.mark_state_dirty();
    fx.server.state.flush_ipc_if_dirty();
    assert!(
        fx.server.state.state_dirty.get(),
        "a second flush within the cap interval must wait for the timer"
    );
}
