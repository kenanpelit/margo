//! Client for margo's Unix-socket IPC. One request per line; replies
//! are newline-delimited JSON.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

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
    let mut sock = connect()?;
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
    mut on_frame: impl FnMut(serde_json::Value) -> bool,
) -> std::io::Result<()> {
    let mut sock = connect()?;
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
