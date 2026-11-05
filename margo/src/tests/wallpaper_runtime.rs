//! Runtime compositor wallpaper over a live IPC connection: the protocol a
//! shell uses to hand margo its wallpaper (`wallpaper_set` / `_update` /
//! `_clear`, `get wallpaper`), including the refusal a shell's feature
//! probe relies on. Rendering pieces are unit-tested in
//! `crate::wallpaper::runtime`.

use super::fixture::Fixture;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn connect(fx: &mut Fixture) -> UnixStream {
    let (server, client) = UnixStream::pair().expect("pair");
    fx.server.state.ipc_accept(server);
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("timeout");
    client
}

/// Send one request line and return the reply frame.
fn request(fx: &mut Fixture, client: &mut UnixStream, line: &str) -> serde_json::Value {
    client
        .write_all(format!("{line}\n").as_bytes())
        .expect("write");
    let mut out = Vec::new();
    for _ in 0..40 {
        fx.dispatch();
        let mut chunk = [0u8; 65536];
        match client.read(&mut chunk) {
            Ok(0) => panic!("margo closed the connection"),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => panic!("read: {e}"),
        }
        if out.ends_with(b"\n") {
            break;
        }
    }
    let text = String::from_utf8(out).expect("utf-8");
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("reply `{text}`: {e}"))
}

/// A headed `[w][h][RGBA]` file, the format mshell hands over.
fn write_raw(path: &std::path::Path, w: u32, h: u32) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.resize(8 + (w * h * 4) as usize, 200);
    std::fs::write(path, bytes).expect("write raw");
}

#[test]
fn a_backend_without_a_compositor_wallpaper_refuses_so_the_shell_keeps_its_own() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.raw");
    write_raw(&path, 4, 2);
    let mut client = connect(&mut fx);
    let reply = request(
        &mut fx,
        &mut client,
        &format!("dispatch wallpaper_set * {}", path.display()),
    );
    assert_eq!(reply["ok"], serde_json::json!(false), "{reply}");
    let topic = request(&mut fx, &mut client, "get wallpaper");
    assert_eq!(topic["supported"], serde_json::json!(false));
    assert!(!fx.server.state.runtime_wallpaper.covers("DP-1"));
}

#[test]
fn a_raw_wallpaper_is_in_memory_when_margo_says_ok() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.runtime_wallpaper.force_backend = true;
    let dir = tempfile::tempdir().expect("tempdir");
    // A path with a space survives the line protocol.
    let path = dir.path().join("mshell wallpaper.raw");
    write_raw(&path, 4, 2);
    let mut client = connect(&mut fx);
    let reply = request(
        &mut fx,
        &mut client,
        &format!(
            "dispatch wallpaper_set * fit=scale-down bg=#101010 owner=mshell {}",
            path.display()
        ),
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }));
    // mshell deletes its tmpfs copy once margo accepted it.
    std::fs::remove_file(&path).expect("rm");
    assert!(fx.server.state.runtime_wallpaper.covers("DP-1"));
    assert!(fx.server.state.runtime_wallpaper.covers("HDMI-A-1"));

    let topic = request(&mut fx, &mut client, "get wallpaper");
    assert_eq!(topic["supported"], serde_json::json!(true));
    let entry = &topic["entries"][0];
    assert_eq!(entry["fit"], serde_json::json!("scale-down"));
    assert_eq!(entry["size"], serde_json::json!([4, 2]));
    assert_eq!(entry["owner"], serde_json::json!("mshell"));

    let reply = request(
        &mut fx,
        &mut client,
        "dispatch wallpaper_update * fit=cover",
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }));
    let topic = request(&mut fx, &mut client, "get wallpaper");
    assert_eq!(topic["entries"][0]["fit"], serde_json::json!("cover"));

    // Another client's clear leaves mshell's entry; mshell's own removes it.
    let reply = request(
        &mut fx,
        &mut client,
        "dispatch wallpaper_clear owner=swaybg",
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }));
    assert!(fx.server.state.runtime_wallpaper.covers("DP-1"));
    let reply = request(
        &mut fx,
        &mut client,
        "dispatch wallpaper_clear * owner=mshell",
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }));
    assert!(!fx.server.state.runtime_wallpaper.covers("DP-1"));
}

#[test]
fn a_bad_wallpaper_is_refused_and_the_old_one_stays() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.runtime_wallpaper.force_backend = true;
    let dir = tempfile::tempdir().expect("tempdir");
    let good = dir.path().join("good.raw");
    write_raw(&good, 2, 2);
    let mut client = connect(&mut fx);
    let ok = request(
        &mut fx,
        &mut client,
        &format!("dispatch wallpaper_set DP-1 {}", good.display()),
    );
    assert_eq!(ok["ok"], serde_json::json!(true), "{ok}");

    let truncated = dir.path().join("bad.raw");
    std::fs::write(&truncated, [2, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3]).expect("write");
    let missing = dir.path().join("missing.raw");
    for line in [
        format!("dispatch wallpaper_set DP-1 {}", truncated.display()),
        format!("dispatch wallpaper_set DP-1 {}", missing.display()),
        "dispatch wallpaper_set DP-1 relative.raw".to_string(),
        "dispatch wallpaper_set".to_string(),
        "dispatch wallpaper_update DP-2 fit=fill".to_string(),
    ] {
        let reply = request(&mut fx, &mut client, &line);
        assert_eq!(reply["ok"], serde_json::json!(false), "{line}: {reply}");
        assert!(
            reply["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{line}: {reply}"
        );
    }
    let topic = request(&mut fx, &mut client, "get wallpaper");
    assert_eq!(topic["entries"][0]["size"], serde_json::json!([2, 2]));
    assert_eq!(topic["pending"], serde_json::json!([]));
}

#[test]
fn an_encoded_wallpaper_decodes_off_the_loop_and_lands() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.runtime_wallpaper.force_backend = true;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.png");
    image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]))
        .save(&path)
        .expect("png");
    let mut client = connect(&mut fx);
    let reply = request(
        &mut fx,
        &mut client,
        &format!("dispatch wallpaper_set DP-1 fit=fill {}", path.display()),
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }));
    for _ in 0..200 {
        if fx.server.state.runtime_wallpaper.covers("DP-1") {
            break;
        }
        fx.dispatch();
        std::thread::sleep(Duration::from_millis(5));
    }
    let topic = request(&mut fx, &mut client, "get wallpaper");
    assert_eq!(
        topic["entries"][0]["size"],
        serde_json::json!([3, 2]),
        "{topic}"
    );
    assert_eq!(topic["pending"], serde_json::json!([]));

    // An undecodable file reports its error and keeps the image shown.
    let junk = dir.path().join("junk.png");
    std::fs::write(&junk, b"not a png").expect("write");
    let reply = request(
        &mut fx,
        &mut client,
        &format!("dispatch wallpaper_set DP-1 {}", junk.display()),
    );
    assert_eq!(reply, serde_json::json!({ "ok": true }), "queued");
    let mut topic = serde_json::Value::Null;
    for _ in 0..200 {
        topic = request(&mut fx, &mut client, "get wallpaper");
        if topic["error"].is_string() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        topic["error"]
            .as_str()
            .is_some_and(|e| e.contains("junk.png")),
        "{topic}"
    );
    assert_eq!(topic["entries"][0]["size"], serde_json::json!([3, 2]));
    assert_eq!(topic["pending"], serde_json::json!([]));
}
