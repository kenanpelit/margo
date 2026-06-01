//! Socket server: accept connections, frame requests, dispatch them.

use crate::state::MargoState;
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use margo_config::Arg;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// Cap on a single connection's pending outbound bytes. A `watch`
/// subscriber that falls this far behind (its kernel send buffer is full
/// *and* we've queued this much on top) is cut loose rather than letting
/// margo's memory grow without bound — it reconnects and re-syncs from a
/// fresh snapshot. 4 MiB is hundreds of state frames.
const IPC_OUT_CAP: usize = 4 * 1024 * 1024;

/// Result of trying to drain a connection's outbound buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum DrainState {
    /// Everything queued was written.
    Drained,
    /// The socket's send buffer is full; some bytes remain queued.
    Blocked,
    /// The peer closed or the write errored — drop the connection.
    Closed,
}

/// Per-connection framing state.
pub struct IpcConn {
    pub stream: UnixStream,
    /// Inbound byte buffer; complete `\n`-terminated lines are popped off.
    pub buf: Vec<u8>,
    /// Outbound byte buffer for frames the socket couldn't take yet
    /// (slow reader). Drained opportunistically + via a WRITE source.
    pub out_buf: Vec<u8>,
    /// calloop token of the WRITE-interest source, present only while
    /// `out_buf` is non-empty (armed on a blocked write, removed once
    /// drained). Keeps level-triggered WRITE from spinning when idle.
    pub write_token: Option<RegistrationToken>,
    /// True once this connection issued a `watch` (stays open, receives
    /// pushed frames).
    pub watching: bool,
}

/// Pop the next complete `\n`-terminated line off `buf` (draining the
/// consumed bytes), trimmed of surrounding whitespace (incl. a trailing
/// `\r`). Returns `None` when no full line is buffered yet, leaving the
/// partial bytes in place for the next read.
pub fn pop_line(buf: &mut Vec<u8>) -> Option<String> {
    let pos = buf.iter().position(|&b| b == b'\n')?;
    let raw: Vec<u8> = buf.drain(..=pos).collect();
    Some(
        String::from_utf8_lossy(&raw[..raw.len() - 1])
            .trim()
            .to_string(),
    )
}

/// Write as much of `buf` as a non-blocking socket will accept, draining
/// the written prefix in place. Never blocks.
pub fn drain_to(mut stream: &UnixStream, buf: &mut Vec<u8>) -> DrainState {
    while !buf.is_empty() {
        match stream.write(buf) {
            Ok(0) => return DrainState::Closed,
            Ok(n) => {
                buf.drain(..n);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return DrainState::Blocked,
            Err(_) => return DrainState::Closed,
        }
    }
    DrainState::Drained
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
                out_buf: Vec::new(),
                write_token: None,
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
                match pop_line(&mut c.buf) {
                    Some(l) => l,
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

    /// Queue a single JSON frame (newline-terminated) for a connection
    /// and try to flush it immediately. Never blocks the event loop: a
    /// frame the socket can't take right now stays buffered and is
    /// retried via a WRITE source. A connection whose backlog exceeds
    /// [`IPC_OUT_CAP`] is dropped (slow consumer → reconnect + re-sync).
    pub fn ipc_send(&mut self, token: u32, value: &serde_json::Value) {
        let over_cap = {
            let Some(c) = self.ipc_conns.get_mut(&token) else {
                return;
            };
            let mut line = value.to_string();
            line.push('\n');
            if c.out_buf.len().saturating_add(line.len()) > IPC_OUT_CAP {
                true
            } else {
                c.out_buf.extend_from_slice(line.as_bytes());
                false
            }
        };
        if over_cap {
            tracing::warn!(
                token,
                "ipc: outbound backlog over cap, dropping slow client"
            );
            self.ipc_drop_conn(token);
            return;
        }
        self.ipc_flush_out(token);
    }

    /// Drain a connection's outbound buffer as far as the socket allows,
    /// then (dis)arm the WRITE source to match what's left. Called from
    /// the request/push path (never from inside the WRITE source — see
    /// [`Self::ipc_flush_out_from_write_source`]).
    fn ipc_flush_out(&mut self, token: u32) {
        let state = {
            let Some(c) = self.ipc_conns.get_mut(&token) else {
                return;
            };
            drain_to(&c.stream, &mut c.out_buf)
        };
        match state {
            DrainState::Closed => self.ipc_drop_conn(token),
            DrainState::Drained => self.ipc_disarm_write(token),
            DrainState::Blocked => self.ipc_arm_write(token),
        }
    }

    /// Flush variant invoked *by* the connection's WRITE source. Returns
    /// the `PostAction` for that source so calloop removes it on drain —
    /// we must not `remove()` it from under ourselves here.
    fn ipc_flush_out_from_write_source(&mut self, token: u32) -> PostAction {
        let state = match self.ipc_conns.get_mut(&token) {
            Some(c) => drain_to(&c.stream, &mut c.out_buf),
            None => return PostAction::Remove,
        };
        match state {
            DrainState::Blocked => PostAction::Continue,
            DrainState::Drained => {
                if let Some(c) = self.ipc_conns.get_mut(&token) {
                    c.write_token = None;
                }
                PostAction::Remove
            }
            DrainState::Closed => {
                // Forget the token first so `ipc_drop_conn` doesn't try to
                // remove this still-running source; calloop removes it via
                // the returned `Remove`.
                if let Some(c) = self.ipc_conns.get_mut(&token) {
                    c.write_token = None;
                }
                self.ipc_drop_conn(token);
                PostAction::Remove
            }
        }
    }

    /// Register a level-triggered WRITE source for a backed-up connection
    /// (idempotent). Uses a cloned fd so it coexists with the READ source
    /// in the same epoll instance.
    fn ipc_arm_write(&mut self, token: u32) {
        let already = self
            .ipc_conns
            .get(&token)
            .map(|c| c.write_token.is_some())
            .unwrap_or(true);
        if already {
            return;
        }
        let fd = match self
            .ipc_conns
            .get(&token)
            .and_then(|c| c.stream.try_clone().ok())
        {
            Some(fd) => fd,
            None => {
                self.ipc_drop_conn(token);
                return;
            }
        };
        let source = Generic::new(fd, Interest::WRITE, Mode::Level);
        let res = self
            .loop_handle
            .insert_source(source, move |_, _fd, state: &mut MargoState| {
                Ok(state.ipc_flush_out_from_write_source(token))
            });
        match res {
            Ok(rtoken) => {
                if let Some(c) = self.ipc_conns.get_mut(&token) {
                    c.write_token = Some(rtoken);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "ipc: arm write source");
                self.ipc_drop_conn(token);
            }
        }
    }

    /// Remove a connection's WRITE source once its backlog has drained.
    fn ipc_disarm_write(&mut self, token: u32) {
        let rt = self
            .ipc_conns
            .get_mut(&token)
            .and_then(|c| c.write_token.take());
        if let Some(rt) = rt {
            self.loop_handle.remove(rt);
        }
    }

    pub fn ipc_drop_conn(&mut self, token: u32) {
        if let Some(c) = self.ipc_conns.remove(&token)
            && let Some(rt) = c.write_token
        {
            self.loop_handle.remove(rt);
        }
        self.ipc_watches.remove_conn(token);
    }
}

#[cfg(test)]
mod tests {
    use super::{DrainState, args_to_dispatch_arg, drain_to, pop_line};
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    // ── framing: pop_line ──────────────────────────────────

    #[test]
    fn pop_line_none_until_newline() {
        let mut buf = b"get sta".to_vec();
        assert_eq!(pop_line(&mut buf), None);
        // Partial bytes stay buffered for the next read.
        assert_eq!(buf, b"get sta");
        buf.extend_from_slice(b"te\n");
        assert_eq!(pop_line(&mut buf).as_deref(), Some("get state"));
        assert!(buf.is_empty());
    }

    #[test]
    fn pop_line_drains_multiple_lines_in_order() {
        let mut buf = b"get state\ndispatch view 4\n".to_vec();
        assert_eq!(pop_line(&mut buf).as_deref(), Some("get state"));
        assert_eq!(pop_line(&mut buf).as_deref(), Some("dispatch view 4"));
        assert_eq!(pop_line(&mut buf), None);
    }

    #[test]
    fn pop_line_trims_crlf_and_whitespace() {
        let mut buf = b"  watch state  \r\n".to_vec();
        assert_eq!(pop_line(&mut buf).as_deref(), Some("watch state"));
    }

    #[test]
    fn pop_line_keeps_partial_after_complete() {
        let mut buf = b"get state\ndispatch vi".to_vec();
        assert_eq!(pop_line(&mut buf).as_deref(), Some("get state"));
        assert_eq!(pop_line(&mut buf), None);
        assert_eq!(buf, b"dispatch vi");
    }

    // ── outbound: drain_to ─────────────────────────────────

    #[test]
    fn drain_to_writes_everything_when_socket_has_room() {
        let (a, mut b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let mut buf = b"hello\n".to_vec();
        assert_eq!(drain_to(&a, &mut buf), DrainState::Drained);
        assert!(buf.is_empty());
        let mut got = [0u8; 6];
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hello\n");
    }

    #[test]
    fn drain_to_blocks_and_keeps_remainder_when_buffer_full() {
        // Nobody reads `b`; a big payload overruns the kernel send buffer.
        let (a, _b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let mut buf = vec![b'x'; 16 * 1024 * 1024];
        let before = buf.len();
        let state = drain_to(&a, &mut buf);
        assert_eq!(state, DrainState::Blocked);
        // Some bytes left the buffer, but not all — back-pressure, no spin.
        assert!(!buf.is_empty(), "should retain the unsent tail");
        assert!(
            buf.len() < before,
            "should have drained the writable prefix"
        );
    }

    #[test]
    fn drain_to_reports_closed_when_peer_is_gone() {
        let (a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        drop(b);
        // Rust ignores SIGPIPE process-wide, so the write surfaces as an
        // error rather than killing the test.
        let mut buf = vec![b'x'; 16 * 1024 * 1024];
        assert_eq!(drain_to(&a, &mut buf), DrainState::Closed);
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
