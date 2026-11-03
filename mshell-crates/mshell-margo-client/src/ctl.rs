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

/// [`dispatch`] on tokio's blocking pool, for async callers that want the
/// result (e.g. to re-probe state after the request landed).
pub async fn dispatch_async(action: String, args: Vec<String>) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        dispatch(&action, &refs)
    })
    .await
    .map_err(io::Error::other)?
}

/// The socket request an `mctl <args…>` invocation (argv without the
/// leading `mctl`) boils down to, for the subcommands that are a plain
/// IPC dispatch in `mctl` itself (`mctl/src/bin/mctl.rs`): `dispatch`,
/// `reload`, `layout`, `tags` and the live `twilight` verbs. Argument
/// defaults and clamps mirror mctl's clap definitions. `None` for
/// anything else (`twilight preset …` edits files client-side, unknown
/// verbs, malformed numbers) — the caller then runs the real `mctl`.
pub fn mctl_dispatch_for<S: AsRef<str>>(args: &[S]) -> Option<(String, Vec<String>)> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let num = |s: &str| s.parse::<u32>().ok();
    let owned = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let (action, rest): (&str, Vec<String>) = match a.as_slice() {
        ["dispatch", action, rest @ ..] => (*action, owned(rest)),
        ["reload"] | ["reload", "--force"] => ("reload_config", Vec::new()),
        ["layout", idx] => ("setlayoutindex", vec![num(idx)?.to_string()]),
        ["tags", mask] => ("settagset", vec![num(mask)?.to_string(), "0".into()]),
        ["tags", mask, toggle] => {
            let toggle = if num(toggle)? != 0 { "1" } else { "0" };
            ("settagset", vec![num(mask)?.to_string(), toggle.into()])
        }
        ["twilight", "toggle"] => ("twilight_toggle", Vec::new()),
        ["twilight", "reset"] => ("twilight_reset", Vec::new()),
        ["twilight", "preview", k] => ("twilight_preview", vec![num(k)?.to_string(), "100".into()]),
        ["twilight", "preview", k, g] => (
            "twilight_preview",
            vec![num(k)?.to_string(), num(g)?.to_string()],
        ),
        ["twilight", "test"] => ("twilight_test", vec!["5".into()]),
        ["twilight", "test", s] => ("twilight_test", vec![num(s)?.clamp(1, 60).to_string()]),
        ["twilight", "set", spec] => ("twilight_set", vec![(*spec).to_string()]),
        _ => return None,
    };
    Some((action.to_string(), rest))
}

/// Run `mctl <args…>` for the shell, awaiting completion: over the socket
/// when [`mctl_dispatch_for`] maps it to a dispatch (no fork from the big
/// shell process), otherwise by spawning and reaping the real `mctl`.
/// `Err` carries a one-line description for logs / toasts.
pub async fn run_mctl(args: Vec<String>) -> Result<(), String> {
    if let Some((action, rest)) = mctl_dispatch_for(&args) {
        return dispatch_async(action, rest)
            .await
            .map_err(|e| format!("mctl {}: {e}", args.join(" ")));
    }
    match tokio::process::Command::new("mctl")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
    {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "mctl {}: {} {}",
            args.join(" "),
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("mctl {}: spawn failed: {e}", args.join(" "))),
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

    fn mapped(args: &[&str]) -> Option<(String, Vec<String>)> {
        mctl_dispatch_for(args)
    }

    fn d(action: &str, args: &[&str]) -> Option<(String, Vec<String>)> {
        Some((
            action.to_string(),
            args.iter().map(|s| (*s).to_string()).collect(),
        ))
    }

    #[test]
    fn mctl_argv_maps_to_the_same_dispatch_mctl_sends() {
        assert_eq!(mapped(&["reload"]), d("reload_config", &[]));
        assert_eq!(mapped(&["dispatch", "view", "16"]), d("view", &["16"]));
        assert_eq!(
            mapped(&["dispatch", "reload_config"]),
            d("reload_config", &[])
        );
        assert_eq!(mapped(&["layout", "3"]), d("setlayoutindex", &["3"]));
        assert_eq!(mapped(&["tags", "4"]), d("settagset", &["4", "0"]));
        assert_eq!(mapped(&["tags", "4", "7"]), d("settagset", &["4", "1"]));
        assert_eq!(mapped(&["twilight", "toggle"]), d("twilight_toggle", &[]));
        assert_eq!(mapped(&["twilight", "reset"]), d("twilight_reset", &[]));
        assert_eq!(
            mapped(&["twilight", "preview", "3500"]),
            d("twilight_preview", &["3500", "100"])
        );
        assert_eq!(
            mapped(&["twilight", "preview", "3500", "80"]),
            d("twilight_preview", &["3500", "80"])
        );
        assert_eq!(mapped(&["twilight", "test"]), d("twilight_test", &["5"]));
        assert_eq!(
            mapped(&["twilight", "test", "600"]),
            d("twilight_test", &["60"])
        );
        assert_eq!(
            mapped(&["twilight", "set", "mode=schedule"]),
            d("twilight_set", &["mode=schedule"])
        );
    }

    #[test]
    fn client_side_or_malformed_mctl_argv_is_not_mapped() {
        // `twilight preset …` edits files inside mctl, no IPC.
        assert_eq!(
            mapped(&["twilight", "preset", "set", "night", "3000", "90"]),
            None
        );
        assert_eq!(mapped(&["twilight", "status", "--json"]), None);
        // mctl would reject these at argument parsing.
        assert_eq!(mapped(&["layout", "tile"]), None);
        assert_eq!(mapped(&["twilight", "preview", "warm"]), None);
        assert_eq!(mapped(&["twilight", "set"]), None);
        assert_eq!(mapped(&["dispatch"]), None);
        assert_eq!(mapped(&[]), None);
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
