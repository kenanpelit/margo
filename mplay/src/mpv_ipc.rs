//! Client for mpv's JSON IPC socket (`--input-ipc-server`). One
//! `{"command":[…]}` line per request. We don't read replies — these are
//! fire-and-forget control commands.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// mpv IPC socket path: `$MARGO_MPV_SOCKET` else `/tmp/mpvsocket`.
pub fn socket_path() -> PathBuf {
    std::env::var_os("MARGO_MPV_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/mpvsocket"))
}

/// True if the IPC socket file exists (mpv is up with IPC enabled).
pub fn socket_ready() -> bool {
    socket_path().exists()
}

/// Send one `{"command":[args…]}` line to the mpv socket at `path`.
pub fn send_command(path: impl AsRef<Path>, args: &[&str]) -> std::io::Result<()> {
    let mut sock = UnixStream::connect(path)?;
    let line = serde_json::json!({ "command": args }).to_string();
    sock.write_all(line.as_bytes())?;
    sock.write_all(b"\n")?;
    Ok(())
}

/// Toggle play/pause on the default socket.
pub fn toggle_pause() -> std::io::Result<()> {
    send_command(socket_path(), &["cycle", "pause"])
}

/// Load `target` into the running mpv (`replace`/`append`) on the default
/// socket.
pub fn loadfile(target: &str, mode: &str) -> std::io::Result<()> {
    send_command(socket_path(), &["loadfile", target, mode])
}

/// Ask mpv to quit on the default socket.
pub fn quit() -> std::io::Result<()> {
    send_command(socket_path(), &["quit"])
}

/// Read one mpv property via `get_property` on the default socket,
/// returning its JSON value. Reads replies (unlike the fire-and-forget
/// commands), skipping any interleaved async events; bounded by a read
/// timeout + line cap so it never hangs.
pub fn get_property(name: &str) -> Option<serde_json::Value> {
    let sock = UnixStream::connect(socket_path()).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))
        .ok()?;
    const REQ_ID: i64 = 7;
    let cmd = serde_json::json!({ "command": ["get_property", name], "request_id": REQ_ID });
    {
        let mut w = &sock;
        w.write_all(cmd.to_string().as_bytes()).ok()?;
        w.write_all(b"\n").ok()?;
    }
    let reader = BufReader::new(&sock);
    for line in reader.lines().take(64) {
        let line = line.ok()?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("request_id").and_then(|x| x.as_i64()) == Some(REQ_ID) {
            return if v.get("error").and_then(|e| e.as_str()) == Some("success") {
                v.get("data").cloned()
            } else {
                None
            };
        }
    }
    None
}

/// Convenience: a boolean property (e.g. `pause`).
pub fn get_bool(name: &str) -> Option<bool> {
    get_property(name)?.as_bool()
}

/// Convenience: a string property (e.g. `media-title`).
pub fn get_string(name: &str) -> Option<String> {
    Some(get_property(name)?.as_str()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn sends_command_line() {
        let p = std::env::temp_dir().join(format!("mplay-mpv-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let l = UnixListener::bind(&p).unwrap();
        let h = thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            line
        });

        send_command(&p, &["cycle", "pause"]).unwrap();
        let got = h.join().unwrap();
        let v: serde_json::Value = serde_json::from_str(got.trim()).unwrap();
        assert_eq!(v["command"], serde_json::json!(["cycle", "pause"]));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn socket_path_honours_env() {
        // Default when unset.
        unsafe { std::env::remove_var("MARGO_MPV_SOCKET") };
        assert_eq!(socket_path(), PathBuf::from("/tmp/mpvsocket"));
    }
}
