//! Client for margo's Unix-socket IPC. One request per line; replies
//! are newline-delimited JSON.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Resolve the socket path: `$MARGO_SOCKET` if set, else
/// `$XDG_RUNTIME_DIR/margo/margo-ipc.sock` (set and absolute, see
/// `margo_fs::xdg::runtime_dir`; else `/run/user/<uid>`).
pub fn socket_path() -> PathBuf {
    // SAFETY: getuid() has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    socket_path_from(
        std::env::var_os("MARGO_SOCKET"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        uid,
    )
}

/// [`socket_path`] from explicit inputs (the testable core).
fn socket_path_from(
    margo_socket: Option<std::ffi::OsString>,
    xdg_runtime_dir: Option<std::ffi::OsString>,
    uid: u32,
) -> PathBuf {
    if let Some(p) = margo_socket {
        return PathBuf::from(p);
    }
    // Unset, empty or relative falls back (margo_fs::xdg::runtime_dir,
    // the same rule the shell's client uses).
    margo_fs::xdg::runtime_dir_from(xdg_runtime_dir)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")))
        .join("margo")
        .join("margo-ipc.sock")
}

pub fn connect() -> std::io::Result<UnixStream> {
    UnixStream::connect(socket_path())
}

/// Send one request, return the single JSON reply line.
pub fn request_once(req: &str) -> std::io::Result<serde_json::Value> {
    request_once_at(socket_path(), req)
}

/// Refuse a request that can't be sent as one protocol line.
///
/// The protocol is one request per `\n`-terminated line, so a control
/// character in an argument (`mctl dispatch spawn "$x"` with a newline in
/// `$x`) would end the request early and smuggle the rest in as a second
/// request. Tabs are plain whitespace to the server and stay allowed.
pub fn validate_request(req: &str) -> std::io::Result<()> {
    if let Some(c) = req.chars().find(|&c| c.is_control() && c != '\t') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("request contains a control character ({c:?}); refusing to send it"),
        ));
    }
    Ok(())
}

/// Why a `dispatch` reply is not a success, or `None` when margo said
/// `ok: true`. A reply without `ok: true` is a failure even when it carries
/// no `error` text.
pub fn dispatch_reply_error(reply: &serde_json::Value) -> Option<String> {
    if reply.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    Some(match reply.get("error").and_then(|v| v.as_str()) {
        Some(err) => err.to_string(),
        None => format!("margo did not confirm (reply: {reply})"),
    })
}

/// [`request_once`] against an explicit socket path (testable seam).
pub fn request_once_at(path: impl AsRef<Path>, req: &str) -> std::io::Result<serde_json::Value> {
    validate_request(req)?;
    let mut sock = UnixStream::connect(path)?;
    // Bound the round-trip so a wedged compositor (accepts the socket but
    // never replies) can't hang mctl — and every script/pill that shells out
    // to it — forever. `watch` streams deliberately skip this (they block for
    // events by design).
    let timeout = std::time::Duration::from_secs(5);
    sock.set_read_timeout(Some(timeout))?;
    sock.set_write_timeout(Some(timeout))?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // Surface a malformed/empty reply (dead or garbage-emitting compositor)
    // as an error so callers exit non-zero, instead of silently returning an
    // Ok sentinel they can't distinguish from a real reply.
    serde_json::from_str(line.trim()).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("malformed reply from margo: {e}"),
        )
    })
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
    validate_request(req)?;
    let mut sock = UnixStream::connect(path)?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    let reader = BufReader::new(sock);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line.trim()) {
            Ok(v) => {
                if !on_frame(v) {
                    break;
                }
            }
            // Keep the stream alive, but say so: a silently dropped frame
            // looks like margo simply not reporting a change.
            Err(e) => eprintln!("mctl: skipping malformed frame from margo: {e}"),
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

    #[test]
    fn socket_path_ignores_a_relative_runtime_dir() {
        let p = |sock: Option<&str>, rt: Option<&str>| {
            socket_path_from(sock.map(Into::into), rt.map(Into::into), 1000)
        };
        assert_eq!(
            p(Some("/tmp/x.sock"), Some("/run/user/1000")),
            std::path::PathBuf::from("/tmp/x.sock")
        );
        assert_eq!(
            p(None, Some("/run/user/1000")),
            std::path::PathBuf::from("/run/user/1000/margo/margo-ipc.sock")
        );
        // Unset, empty or relative: a relative socket path would bind /
        // connect under whatever the cwd is.
        for bad in [None, Some(""), Some("relative/run"), Some("run")] {
            assert_eq!(
                p(None, bad),
                std::path::PathBuf::from("/run/user/1000/margo/margo-ipc.sock"),
                "{bad:?}"
            );
        }
    }

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
    fn multi_line_requests_are_refused_before_sending() {
        let path = temp_sock("inject");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        for req in [
            "dispatch spawn notify-send hi\ndispatch quit",
            "get state\r\ndispatch quit",
            "dispatch view 1\n",
        ] {
            let err = request_once_at(&path, req).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{req:?}");
            let err = watch_stream_at(&path, req, |_| true).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{req:?}");
        }
        assert!(listener.accept().is_err(), "nothing reached the socket");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn request_once_errors_on_garbage_reply() {
        let path = temp_sock("garbage");
        let listener = UnixListener::bind(&path).unwrap();
        let srv = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read the request first: replying and closing before the
            // client has written raced it into a BrokenPipe on the write.
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            stream.write_all(b"not json at all\n").unwrap();
        });

        // A malformed reply (dead or garbage-emitting compositor) must surface
        // as an error so callers exit non-zero — not a silent Ok sentinel.
        let err = request_once_at(&path, "get state").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        srv.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_request_with_a_newline_is_refused_before_it_is_sent() {
        // The protocol is one request per line: an argument carrying `\n`
        // would smuggle a second request (`spawn x\ndispatch quit`).
        let path = temp_sock("inject");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();

        for req in [
            "dispatch spawn a\ndispatch quit",
            "get state\r",
            "dispatch x \u{0}",
        ] {
            let err = request_once_at(&path, req).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{req:?}");
            let err = watch_stream_at(&path, req, |_| true).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{req:?}");
        }
        assert!(
            listener.accept().is_err(),
            "nothing may reach margo for a refused request"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_tab_is_plain_whitespace_and_allowed() {
        assert!(validate_request("dispatch\tview 4").is_ok());
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
