//! Socket server: accept connections, frame requests, dispatch them.

use crate::state::MargoState;
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use margo_config::Arg;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// Per-connection read buffer + framing state.
pub struct IpcConn {
    pub stream: UnixStream,
    pub buf: Vec<u8>,
    /// True once this connection issued a `watch` (stays open, receives
    /// pushed frames).
    pub watching: bool,
}

/// Bind the socket and register the accept source on the loop. Removes
/// any stale socket file first. Best-effort: a bind failure is logged
/// but never fatal.
pub fn insert_ipc_source(handle: &LoopHandle<'static, MargoState>) {
    let path = super::socket_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(?path, error = %e, "ipc: bind failed");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        tracing::warn!(error = %e, "ipc: set_nonblocking");
    }
    let source = Generic::new(listener, Interest::READ, Mode::Level);
    let res = handle.insert_source(source, |_, listener, state: &mut MargoState| {
        loop {
            match listener.accept() {
                Ok((stream, _)) => state.ipc_accept(stream),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    tracing::warn!(error = %e, "ipc: accept");
                    break;
                }
            }
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = res {
        tracing::error!(error = %e, "ipc: insert accept source");
    } else {
        tracing::info!(?path, "ipc: listening");
    }
}

/// Map dispatch args onto margo's `Arg`. margo's actions come in two
/// shapes and never mix them, so we branch on the first token:
///
/// * **String-payload** (spawn / theme / run_script / twilight_set): the
///   first token is non-numeric. The line protocol is whitespace-split,
///   so the *whole* remainder is rejoined into `v` — otherwise a command
///   like `spawn kitty -e htop` would drop everything past `kitty`.
/// * **Numeric/positional** (view / settagset / twilight_preview / …):
///   slots 1-3 parse as numbers (i / i2 / f), slots 4-5 are strings
///   (`v` / `v2`), mirroring the old dwl-ipc dispatch slot semantics.
pub fn args_to_dispatch_arg(args: &[String]) -> Arg {
    let mut arg = Arg::default();
    let first_numeric = args.first().is_some_and(|s| s.parse::<i64>().is_ok());
    if !first_numeric {
        if !args.is_empty() {
            arg.v = Some(args.join(" "));
        }
        return arg;
    }
    if let Some(a) = args.first().and_then(|s| s.parse::<i32>().ok()) {
        arg.i = a;
    }
    if let Some(a) = args.get(1).and_then(|s| s.parse::<i32>().ok()) {
        arg.i2 = a;
    }
    if let Some(a) = args.get(2).and_then(|s| s.parse::<f32>().ok()) {
        arg.f = a;
    }
    if let Some(s) = args.get(3) {
        arg.v = Some(s.clone());
    }
    if let Some(s) = args.get(4) {
        arg.v2 = Some(s.clone());
    }
    arg
}

impl MargoState {
    /// Register a freshly-accepted connection: stash it under a new
    /// token and add a calloop READ source that drains complete lines.
    pub fn ipc_accept(&mut self, stream: UnixStream) {
        if let Err(e) = stream.set_nonblocking(true) {
            tracing::warn!(error = %e, "ipc: client set_nonblocking");
        }
        let token = self.ipc_next_token;
        self.ipc_next_token = self.ipc_next_token.wrapping_add(1);
        let read_fd = match stream.try_clone() {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!(error = %e, "ipc: clone fd");
                return;
            }
        };
        self.ipc_conns.insert(
            token,
            IpcConn {
                stream,
                buf: Vec::new(),
                watching: false,
            },
        );
        let source = Generic::new(read_fd, Interest::READ, Mode::Level);
        let res = self
            .loop_handle
            .insert_source(source, move |_, fd, state: &mut MargoState| {
                // calloop hands a `&mut NoIoDrop<UnixStream>` (Deref only,
                // no DerefMut). `UnixStream` implements `Read for
                // &UnixStream`, so read through the shared reference.
                Ok(state.ipc_readable(token, fd))
            });
        if let Err(e) = res {
            tracing::warn!(error = %e, "ipc: insert client source");
            self.ipc_drop_conn(token);
        }
    }

    /// Drain readable bytes for one connection, handling each complete
    /// `\n`-terminated request line.
    fn ipc_readable(&mut self, token: u32, fd: &UnixStream) -> PostAction {
        let mut chunk = [0u8; 4096];
        // `impl Read for &UnixStream` — read through a shared ref.
        let mut reader: &UnixStream = fd;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    self.ipc_drop_conn(token);
                    return PostAction::Remove;
                }
                Ok(n) => {
                    if let Some(c) = self.ipc_conns.get_mut(&token) {
                        c.buf.extend_from_slice(&chunk[..n]);
                    }
                    self.ipc_process_lines(token);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return PostAction::Continue,
                Err(_) => {
                    self.ipc_drop_conn(token);
                    return PostAction::Remove;
                }
            }
        }
    }

    fn ipc_process_lines(&mut self, token: u32) {
        loop {
            let line = {
                let Some(c) = self.ipc_conns.get_mut(&token) else {
                    return;
                };
                match c.buf.iter().position(|&b| b == b'\n') {
                    Some(pos) => {
                        let raw: Vec<u8> = c.buf.drain(..=pos).collect();
                        String::from_utf8_lossy(&raw[..raw.len() - 1])
                            .trim()
                            .to_string()
                    }
                    None => return,
                }
            };
            if line.is_empty() {
                continue;
            }
            self.ipc_handle_request(token, &line);
        }
    }

    fn ipc_handle_request(&mut self, token: u32, line: &str) {
        use super::protocol::{Verb, parse_request};
        let req = match parse_request(line) {
            Ok(r) => r,
            Err(e) => return self.ipc_send(token, &serde_json::json!({ "error": e })),
        };
        match req.verb {
            Verb::Get => {
                let payload = self.ipc_topic(&req.head, &req.args);
                self.ipc_send(token, &payload);
            }
            Verb::Dispatch => {
                let arg = args_to_dispatch_arg(&req.args);
                crate::dispatch::dispatch_action(self, &req.head, &arg);
                self.ipc_send(token, &serde_json::json!({ "ok": true }));
            }
            Verb::Watch => {
                if let Some(c) = self.ipc_conns.get_mut(&token) {
                    c.watching = true;
                }
                self.ipc_watches
                    .add(token, req.head.clone(), req.args.clone());
                let payload = self.ipc_topic(&req.head, &req.args);
                self.ipc_send(token, &payload);
            }
        }
    }

    /// Serialize + write a single JSON frame (newline-terminated) to a
    /// connection. Drops the connection on write error.
    pub fn ipc_send(&mut self, token: u32, value: &serde_json::Value) {
        let drop = {
            let Some(c) = self.ipc_conns.get_mut(&token) else {
                return;
            };
            let mut line = value.to_string();
            line.push('\n');
            c.stream.write_all(line.as_bytes()).is_err()
        };
        if drop {
            self.ipc_drop_conn(token);
        }
    }

    pub fn ipc_drop_conn(&mut self, token: u32) {
        self.ipc_conns.remove(&token);
        self.ipc_watches.remove_conn(token);
    }
}

#[cfg(test)]
mod tests {
    use super::args_to_dispatch_arg;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn numeric_first_maps_positional_slots() {
        // `dispatch settagset 256 1`
        let a = args_to_dispatch_arg(&args(&["256", "1"]));
        assert_eq!(a.i, 256);
        assert_eq!(a.i2, 1);
        assert_eq!(a.f, 0.0);
        assert!(a.v.is_none());
    }

    #[test]
    fn numeric_first_parses_float_third_slot() {
        let a = args_to_dispatch_arg(&args(&["1", "2", "0.5"]));
        assert_eq!(a.i, 1);
        assert_eq!(a.i2, 2);
        assert_eq!(a.f, 0.5);
    }

    #[test]
    fn single_numeric_arg() {
        // `dispatch view 4`
        let a = args_to_dispatch_arg(&args(&["4"]));
        assert_eq!(a.i, 4);
        assert!(a.v.is_none());
    }

    #[test]
    fn string_payload_single_word() {
        // `dispatch theme default`
        let a = args_to_dispatch_arg(&args(&["default"]));
        assert_eq!(a.v.as_deref(), Some("default"));
        assert_eq!(a.i, 0);
    }

    #[test]
    fn string_payload_rejoins_multiword_command() {
        // `dispatch spawn kitty -e htop` — regression: must not drop
        // everything past the first token.
        let a = args_to_dispatch_arg(&args(&["kitty", "-e", "htop"]));
        assert_eq!(a.v.as_deref(), Some("kitty -e htop"));
        assert_eq!(a.i, 0);
        assert_eq!(a.i2, 0);
    }

    #[test]
    fn string_payload_preserves_flags_and_paths() {
        let a = args_to_dispatch_arg(&args(&["run_helper", "--flag", "/abs/path with space"]));
        assert_eq!(
            a.v.as_deref(),
            Some("run_helper --flag /abs/path with space")
        );
    }

    #[test]
    fn no_args_is_default() {
        // `dispatch reload`
        let a = args_to_dispatch_arg(&[]);
        assert_eq!(a.i, 0);
        assert!(a.v.is_none());
    }

    #[test]
    fn positional_string_slots_for_numeric_action() {
        // numeric-first action that also carries string slots 4/5
        let a = args_to_dispatch_arg(&args(&["1", "2", "3", "slot4", "slot5"]));
        assert_eq!(a.i, 1);
        assert_eq!(a.v.as_deref(), Some("slot4"));
        assert_eq!(a.v2.as_deref(), Some("slot5"));
    }
}
