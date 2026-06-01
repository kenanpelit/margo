//! Client for mpv's JSON IPC socket (`--input-ipc-server`). One
//! `{"command":[…]}` line per request. We don't read replies — these are
//! fire-and-forget control commands.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

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
