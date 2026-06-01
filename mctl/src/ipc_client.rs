//! Client for margo's Unix-socket IPC. One request per line; replies
//! are newline-delimited JSON.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Resolve the socket path: `$MARGO_SOCKET` if set, else
/// `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("MARGO_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    runtime.join("margo").join("margo-ipc.sock")
}

pub fn connect() -> std::io::Result<UnixStream> {
    UnixStream::connect(socket_path())
}

/// Send one request, return the single JSON reply line.
pub fn request_once(req: &str) -> std::io::Result<serde_json::Value> {
    request_once_at(socket_path(), req)
}

/// [`request_once`] against an explicit socket path (testable seam).
pub fn request_once_at(path: impl AsRef<Path>, req: &str) -> std::io::Result<serde_json::Value> {
    let mut sock = UnixStream::connect(path)?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(line.trim())
        .unwrap_or_else(|_| serde_json::json!({ "error": "bad reply" })))
}

/// Send a `watch …` request and invoke `on_frame` for every JSON line
/// until the connection closes or the callback returns `false`.
pub fn watch_stream(
    req: &str,
    on_frame: impl FnMut(serde_json::Value) -> bool,
) -> std::io::Result<()> {
    watch_stream_at(socket_path(), req, on_frame)
}

/// [`watch_stream`] against an explicit socket path (testable seam).
pub fn watch_stream_at(
    path: impl AsRef<Path>,
    req: &str,
    mut on_frame: impl FnMut(serde_json::Value) -> bool,
) -> std::io::Result<()> {
    let mut sock = UnixStream::connect(path)?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    let reader = BufReader::new(sock);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim())
            && !on_frame(v)
        {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// Bind a throwaway socket under a unique temp path.
    fn temp_sock(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mctl-ipc-test-{}-{}-{:?}.sock",
            tag,
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn request_once_round_trips_one_reply() {
        let path = temp_sock("req");
        let listener = UnixListener::bind(&path).unwrap();
        // Fake margo: read one request line, reply with one JSON line.
        let srv = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut req = String::new();
            reader.read_line(&mut req).unwrap();
            assert_eq!(req.trim(), "get state");
            let mut w = stream;
            w.write_all(b"{\"clients\":[],\"ok\":true}\n").unwrap();
        });

        let reply = request_once_at(&path, "get state").unwrap();
        assert_eq!(reply["ok"], serde_json::json!(true));
        assert!(reply["clients"].is_array());

        srv.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn request_once_handles_garbage_reply() {
        let path = temp_sock("garbage");
        let listener = UnixListener::bind(&path).unwrap();
        let srv = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"not json at all\n").unwrap();
        });

        let reply = request_once_at(&path, "get state").unwrap();
        assert_eq!(reply, serde_json::json!({ "error": "bad reply" }));

        srv.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn watch_stream_delivers_frames_until_close() {
        let path = temp_sock("watch");
        let listener = UnixListener::bind(&path).unwrap();
        let srv = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            // Consume the `watch …` request first: closing with unread
            // inbound data makes the kernel send RST (ConnectionReset),
            // which would discard the frames we then write.
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut req = String::new();
            reader.read_line(&mut req).unwrap();
            let mut stream = stream;
            // Three pushed frames, then close (drop the stream).
            stream.write_all(b"{\"n\":1}\n").unwrap();
            stream.write_all(b"\n").unwrap(); // blank keep-alive — skipped
            stream.write_all(b"{\"n\":2}\n").unwrap();
            stream.write_all(b"{\"n\":3}\n").unwrap();
        });

        let mut seen = Vec::new();
        watch_stream_at(&path, "watch state", |v| {
            seen.push(v["n"].as_i64().unwrap());
            true
        })
        .unwrap();

        assert_eq!(seen, vec![1, 2, 3]);
        srv.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn watch_stream_stops_when_callback_returns_false() {
        let path = temp_sock("watch-stop");
        let listener = UnixListener::bind(&path).unwrap();
        let srv = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut req = String::new();
            reader.read_line(&mut req).unwrap();
            let mut stream = stream;
            // Keep pushing; the client stops early, so writes eventually
            // error once it closes its end — that's the exit condition.
            for n in 1..=100 {
                if stream
                    .write_all(format!("{{\"n\":{n}}}\n").as_bytes())
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut count = 0;
        watch_stream_at(&path, "watch state", |_v| {
            count += 1;
            count < 2 // stop after the second frame
        })
        .unwrap();

        assert_eq!(count, 2);
        srv.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
