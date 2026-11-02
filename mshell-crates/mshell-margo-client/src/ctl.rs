//! In-process `mctl`: compositor control over margo's IPC socket.
//!
//! The shell used to fork `mctl` for these (`mctl reload`, `mctl tags …`,
//! `mctl twilight status --json`), from a large-RSS, many-thread process, and
//! often without reaping the child. These helpers speak the same newline
//! JSON protocol `mctl` itself uses (`mctl::ipc_client`) on the socket the
//! shell already holds a `watch state` subscription on.
//!
//! The blocking calls are bounded by a short timeout; call them off the GTK
//! main thread (the `*_detached` variants do that for you).

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::state_json::socket_path;

/// Per-request bound. A config reload is the slowest dispatch margo handles
/// (it re-parses `config.conf`), so this is looser than the 250 ms
/// `get state` cap the main-thread readers use.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Build a `dispatch <action> <args…>` frame (empty args are skipped, like
/// `mctl dispatch`).
fn dispatch_frame(action: &str, args: &[&str]) -> String {
    let mut req = format!("dispatch {action}");
    for a in args.iter().filter(|a| !a.is_empty()) {
        req.push(' ');
        req.push_str(a);
    }
    req
}

/// Map margo's `{"ok": …, "error": …}` reply to a result.
fn check_reply(reply: &serde_json::Value) -> io::Result<()> {
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        let msg = reply
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("margo rejected the request");
        return Err(io::Error::other(msg.to_string()));
    }
    Ok(())
}

/// One request/reply round-trip against the socket at `path`.
pub fn request_at(path: &Path, req: &str) -> io::Result<serde_json::Value> {
    let mut sock = UnixStream::connect(path)?;
    sock.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    sock.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(sock).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("malformed reply from margo: {e}"),
        )
    })
}

/// `dispatch <action> <args…>` against the socket at `path`; an
/// `{"ok": false}` reply is an error.
pub fn dispatch_at(path: &Path, action: &str, args: &[&str]) -> io::Result<()> {
    check_reply(&request_at(path, &dispatch_frame(action, args))?)
}

/// `dispatch <action> <args…>` on margo's socket (blocking, bounded).
pub fn dispatch(action: &str, args: &[&str]) -> io::Result<()> {
    dispatch_at(&socket_path(), action, args)
}

/// [`dispatch`] on a short-lived background thread; failures are logged.
/// For fire-and-forget callers on the GTK main thread.
pub fn dispatch_detached(action: &str, args: &[&str]) {
    let action = action.to_string();
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let spawned = std::thread::Builder::new()
        .name("margo-dispatch".into())
        .spawn(move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            if let Err(e) = dispatch(&action, &refs) {
                tracing::warn!(error = %e, action, ?args, "margo dispatch failed");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "margo dispatch: could not start thread");
    }
}

/// Re-read `config.conf` in the running compositor — the in-process
/// `mctl reload`. margo validates the file itself and keeps the previous
/// config when it has errors (`reload_config` returns early), so skipping
/// `mctl reload`'s client-side pre-check loses nothing but its terminal
/// diagnostics, which nobody saw from inside the shell anyway.
pub fn reload_config_detached() {
    dispatch_detached("reload_config", &[]);
}

/// The `twilight` block of margo's state (what `mctl twilight status --json`
/// prints). Blocking and bounded; `Ok(Null)` from a margo that doesn't
/// advertise twilight.
pub fn twilight_status() -> io::Result<serde_json::Value> {
    let state = request_at(&socket_path(), "get state")?;
    Ok(state
        .get("twilight")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A one-shot fake margo: accepts one connection, records the request
    /// line, answers `reply`.
    fn fake_margo(reply: &'static str) -> (tempfile::TempDir, std::thread::JoinHandle<String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("margo-ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut req = String::new();
            reader.read_line(&mut req).unwrap();
            (&sock).write_all(reply.as_bytes()).unwrap();
            req
        });
        (dir, handle)
    }

    #[test]
    fn dispatch_sends_one_frame_and_accepts_ok() {
        let (dir, server) = fake_margo("{\"ok\":true}\n");
        dispatch_at(&dir.path().join("margo-ipc.sock"), "settagset", &["4", "0"]).unwrap();
        assert_eq!(server.join().unwrap(), "dispatch settagset 4 0\n");
    }

    #[test]
    fn dispatch_surfaces_a_rejection() {
        let (dir, server) = fake_margo("{\"ok\":false,\"error\":\"unknown action\"}\n");
        let err = dispatch_at(&dir.path().join("margo-ipc.sock"), "bogus", &[]).unwrap_err();
        assert_eq!(err.to_string(), "unknown action");
        server.join().unwrap();
    }

    #[test]
    fn dispatch_without_margo_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dispatch_at(&dir.path().join("none.sock"), "reload_config", &[]).is_err());
    }

    #[test]
    fn frame_skips_empty_args() {
        assert_eq!(
            dispatch_frame("reload_config", &[]),
            "dispatch reload_config"
        );
        assert_eq!(
            dispatch_frame("focuswindowid", &["", "7"]),
            "dispatch focuswindowid 7"
        );
    }
}
