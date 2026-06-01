# margo Socket IPC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace margo's compositor IPC (the `dwl-ipc-unstable-v2` Wayland protocol + the polled `state.json` file) with a single, scriptable Unix-domain-socket JSON protocol exposing `get` / `watch` / `dispatch`, and migrate every consumer (mctl, the shell, mlock, launcher providers) onto it. No backwards compatibility — the old paths are removed entirely.

**Architecture:** margo opens a `SOCK_STREAM` Unix socket on its calloop event loop. Clients send newline-delimited text requests (`get state`, `watch state`, `dispatch view 4`, `get clients`, …) and receive newline-delimited JSON replies. `get` returns one reply and the connection stays usable; `watch` returns an initial frame then streams a frame on every state change; `dispatch` runs a compositor action. The existing `build_state_snapshot()` serde_json builder is reused verbatim as the `state` payload, so mctl and the shell keep their current JSON parsing and only swap transport. `watch` push hooks into the existing once-per-loop "state dirty" flush point. `mctl` becomes a plain `UnixStream` client; the shell's `mshell-margo-client` replaces its inotify/file-poll loop with a `watch state` subscription feeding the same `apply_snapshot`.

**Tech Stack:** Rust, Smithay, calloop (`Generic` event source), `serde_json`, `tokio` (shell side), Unix domain sockets (`std::os::unix::net` server-side, `tokio::net::UnixStream` shell-side).

---

## Protocol specification (v1)

Authoritative reference for every task below.

**Socket path:** `$XDG_RUNTIME_DIR/margo/margo-ipc.sock` (fallback `/run/user/<uid>/margo/margo-ipc.sock`). margo exports `MARGO_SOCKET=<path>` into its own environment and into spawned children so scripts and clients can find it without re-deriving the path.

**Transport:** `AF_UNIX` / `SOCK_STREAM`. Requests are UTF-8, one request per `\n`-terminated line. Replies are one JSON object per `\n`-terminated line.

**Requests** (first whitespace-separated token is the verb):

| Request | Reply | Connection |
|---|---|---|
| `get <topic> [args…]` | one JSON frame | stays open, reusable |
| `dispatch <action> [a1..a5]` | one JSON frame `{"ok":true}` / `{"ok":false,"error":…}` | stays open |
| `watch <topic> [args…]` | initial frame, then a frame on every change | stays open until client disconnects |

**Topics** (`get` and `watch` share the topic set):

| Topic | Args | Payload |
|---|---|---|
| `state` | — | the full snapshot (today's `build_state_snapshot()` document) |
| `clients` | — | `{"clients":[…]}` (the `clients` array of the snapshot) |
| `client` | `<id>` | one client object, or `{"error":"no such client"}` |
| `monitors` | — | `{"monitors":[…]}` (snapshot `outputs`) |
| `monitor` | `<name>` | one monitor object, or error |
| `tags` | `<monitor>` | `{"tags":[…]}` for that monitor |
| `focused` | — | `{"focused":{…}}` or `{"focused":null}` |
| `layouts` | — | `{"layouts":[…]}` |
| `keyboard-layout` | — | `{"keyboard_layout":"…"}` |
| `twilight` | — | the snapshot `twilight` object |
| `config-errors` | — | `{"config_errors":[…]}` |

**Reply envelope:** success frames are the payload object directly. Error frames are `{"error":"<message>"}`. `dispatch` success is `{"ok":true}`. Unknown verb/topic → `{"error":"…"}` and the connection stays open.

**Watch semantics:** every `watch` frame for any topic is the same shape as the matching `get`. The server pushes a fresh frame whenever the compositor marks state dirty (the same signal that drove `state.json` writes). `watch state` is the shell's firehose.

---

## File Structure

**margo (compositor) — new:**
- `margo/src/ipc/mod.rs` — module root; socket path resolver, `MARGO_SOCKET` export, server setup entry `insert_ipc_source()`.
- `margo/src/ipc/server.rs` — `UnixListener` accept loop as a calloop `Generic` source; per-client `IpcConn` read-buffer state; line framing; request dispatch.
- `margo/src/ipc/protocol.rs` — request parsing (`Request` enum + `parse_request`), reply serialization helpers.
- `margo/src/ipc/topics.rs` — topic → `serde_json::Value` builders (reuse `build_state_snapshot`; add per-topic projections).
- `margo/src/ipc/watch.rs` — watch-subscription registry + `push_to_watchers()` fan-out.

**margo — modified:**
- `margo/src/state/state_file.rs` → renamed/repurposed: keep `build_state_snapshot()` (move into `ipc/topics.rs`), delete the file-writing (`write_state_file_inner`, `flush_state_file_if_dirty` becomes `flush_ipc_if_dirty`).
- `margo/src/state.rs` — `state_dirty` flag stays; `write_state_file()` renamed `mark_state_dirty()`; the per-loop flush now pushes to watchers instead of writing a file. Remove `dwl_ipc` field on `Monitor`.
- `margo/src/main.rs` — register the IPC source; stop creating dwl-ipc global; export `MARGO_SOCKET`.
- `margo/src/protocols/mod.rs` — drop `dwl_ipc`.
- **Delete:** `margo/src/protocols/dwl_ipc.rs`, the dwl-ipc entries in `margo/src/protocols/generated.rs`, `protocols/dwl-ipc-unstable-v2.xml`.

**mctl — rewritten transport:**
- `mctl/src/ipc_client.rs` — new: `UnixStream` client (`request_once`, `watch_stream`), socket-path resolver.
- `mctl/src/bin/mctl.rs` — subcommands now call `ipc_client`; new `get` / `watch` subcommands.
- **Delete:** `mctl/src/protocols/mod.rs` (wayland bindings), dwl-ipc bits in `mctl/src/lib.rs`.

**shell — modified:**
- `mshell-crates/mshell-margo-client/src/sync.rs` — replace inotify/file loop with a `watch state` socket subscription.
- `mshell-crates/mshell-margo-client/src/state_json.rs` — keep the `StateJson` types + parsing; replace `read()`/`read_raw()`/`state_json_path()` with a `connect()` + socket read.
- `mshell-crates/mshell-utils/src/margo.rs`, `mshell-crates/mshell-launcher/src/providers/mctl.rs` — read via the socket (shell out to `mctl get state` or the new client helper).

**other consumers — modified:**
- `mlock/src/wallpaper.rs` — resolve wallpaper via `mctl get state` (one-shot) instead of reading `state.json`.

---

## Phase 0 — Protocol crate-local module + parser (margo, no socket yet)

### Task 0.1: IPC module skeleton + socket path

**Files:**
- Create: `margo/src/ipc/mod.rs`
- Modify: `margo/src/lib.rs` or `margo/src/main.rs` (add `mod ipc;`)

- [ ] **Step 1: Create the module with the path resolver + env export**

```rust
// margo/src/ipc/mod.rs
//! margo's Unix-domain-socket IPC. A newline-delimited text request /
//! JSON reply protocol exposing `get`, `watch`, and `dispatch`.
//! Replaces the legacy dwl-ipc-v2 Wayland protocol and the polled
//! state.json file — there is no backwards-compatible path.

pub mod protocol;
pub mod server;
pub mod topics;
pub mod watch;

use std::path::PathBuf;

/// Conventional socket path: `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`,
/// falling back to `/run/user/<uid>/margo/...` when XDG is unset —
/// same base dir the old state.json used.
pub fn socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    runtime.join("margo").join("margo-ipc.sock")
}

/// Export `MARGO_SOCKET` so clients and spawned children can find the
/// socket without re-deriving the path.
pub fn export_socket_env() {
    // SAFETY: called once during single-threaded startup, before any
    // threads that read the environment are spawned.
    unsafe { std::env::set_var("MARGO_SOCKET", socket_path()) };
}
```

- [ ] **Step 2: Register the module**

Add `mod ipc;` near the other `mod` declarations in `margo/src/main.rs` (check whether modules are declared in `main.rs` or a `lib.rs`; match the existing style).

- [ ] **Step 3: Verify it compiles (stubs for submodules first)**

Create empty `margo/src/ipc/{protocol,server,topics,watch}.rs` with a `// placeholder` line so `cargo check -p margo` resolves the `pub mod` lines. Run: `cargo check -p margo` — Expected: compiles (unused-warnings OK).

- [ ] **Step 4: Commit**

```bash
git add margo/src/ipc/ margo/src/main.rs
git commit -m "feat(ipc): scaffold socket-IPC module + path/env"
```

### Task 0.2: Request grammar + parser (TDD)

**Files:**
- Modify: `margo/src/ipc/protocol.rs`

- [ ] **Step 1: Write the failing test**

```rust
// margo/src/ipc/protocol.rs
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Verb {
    Get,
    Watch,
    Dispatch,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Request {
    pub verb: Verb,
    /// For get/watch: the topic. For dispatch: the action name.
    pub head: String,
    /// Remaining whitespace-separated tokens.
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_state() {
        let r = parse_request("get state").unwrap();
        assert_eq!(r.verb, Verb::Get);
        assert_eq!(r.head, "state");
        assert!(r.args.is_empty());
    }

    #[test]
    fn parses_dispatch_with_args() {
        let r = parse_request("dispatch view 4").unwrap();
        assert_eq!(r.verb, Verb::Dispatch);
        assert_eq!(r.head, "view");
        assert_eq!(r.args, vec!["4".to_string()]);
    }

    #[test]
    fn rejects_unknown_verb() {
        assert!(parse_request("frobnicate state").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_request("   ").is_err());
    }
}
```

- [ ] **Step 2: Run it to verify failure**

Run: `cargo test -p margo ipc::protocol -- --nocapture`
Expected: FAIL — `parse_request` not found.

- [ ] **Step 3: Implement the parser**

```rust
pub fn parse_request(line: &str) -> Result<Request, String> {
    let mut toks = line.split_whitespace();
    let verb = match toks.next() {
        Some("get") => Verb::Get,
        Some("watch") => Verb::Watch,
        Some("dispatch") => Verb::Dispatch,
        Some(other) => return Err(format!("unknown verb: {other}")),
        None => return Err("empty request".into()),
    };
    let head = toks
        .next()
        .ok_or_else(|| "missing topic/action".to_string())?
        .to_string();
    let args = toks.map(str::to_string).collect();
    Ok(Request { verb, head, args })
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p margo ipc::protocol`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add margo/src/ipc/protocol.rs
git commit -m "feat(ipc): request grammar + parser (get/watch/dispatch)"
```

---

## Phase 1 — Topic payloads (reuse the snapshot builder)

### Task 1.1: Move `build_state_snapshot` into `ipc/topics.rs`

**Files:**
- Modify: `margo/src/ipc/topics.rs`
- Modify: `margo/src/state/state_file.rs:61-289` (the `build_state_snapshot` method)

- [ ] **Step 1: Move the method**

Cut the entire `fn build_state_snapshot(&self) -> serde_json::Value` body from `margo/src/state/state_file.rs` and paste it into `margo/src/ipc/topics.rs` inside an `impl MargoState` block:

```rust
// margo/src/ipc/topics.rs
use crate::state::MargoState;
use serde_json::{Value, json};

impl MargoState {
    /// The full state snapshot — the `state` topic payload. (Moved
    /// verbatim from the old state_file.rs; this is the same document
    /// the shell + mctl already parse.)
    pub fn ipc_state_snapshot(&self) -> Value {
        // … paste the existing build_state_snapshot body here, renamed …
    }
}
```

Rename the method `ipc_state_snapshot`. Update its one caller (the old `write_state_file_inner`) — that caller is deleted in Task 6.x, so for now leave a temporary `pub fn build_state_snapshot(&self) -> Value { self.ipc_state_snapshot() }` shim in state_file.rs to keep it compiling.

- [ ] **Step 2: Verify compile**

Run: `cargo check -p margo`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add margo/src/ipc/topics.rs margo/src/state/state_file.rs
git commit -m "refactor(ipc): move snapshot builder into ipc::topics"
```

### Task 1.2: Per-topic projections

**Files:**
- Modify: `margo/src/ipc/topics.rs`

- [ ] **Step 1: Add the topic dispatcher**

```rust
impl MargoState {
    /// Build the JSON payload for a `get`/`watch` topic. Returns an
    /// error frame value (`{"error":…}`) for unknown topics / bad args.
    pub fn ipc_topic(&self, topic: &str, args: &[String]) -> Value {
        let snap = self.ipc_state_snapshot();
        match topic {
            "state" => snap,
            "clients" => json!({ "clients": snap["clients"].clone() }),
            "monitors" => json!({ "monitors": snap["outputs"].clone() }),
            "layouts" => json!({ "layouts": snap["layouts"].clone() }),
            "twilight" => snap["twilight"].clone(),
            "config-errors" => json!({ "config_errors": snap["config_errors"].clone() }),
            "keyboard-layout" => json!({ "keyboard_layout": self.current_kb_layout }),
            "focused" => {
                let f = snap["clients"]
                    .as_array()
                    .and_then(|cs| cs.iter().find(|c| c["focused"] == json!(true)))
                    .cloned()
                    .unwrap_or(Value::Null);
                json!({ "focused": f })
            }
            "client" => match args.first().and_then(|s| s.parse::<i64>().ok()) {
                Some(id) => snap["clients"]
                    .as_array()
                    .and_then(|cs| cs.iter().find(|c| c["idx"] == json!(id)).cloned())
                    .unwrap_or_else(|| json!({ "error": "no such client" })),
                None => json!({ "error": "usage: get client <id>" }),
            },
            "monitor" => match args.first() {
                Some(name) => snap["outputs"]
                    .as_array()
                    .and_then(|ms| ms.iter().find(|m| m["name"] == json!(name)).cloned())
                    .unwrap_or_else(|| json!({ "error": "no such monitor" })),
                None => json!({ "error": "usage: get monitor <name>" }),
            },
            "tags" => match args.first() {
                Some(name) => snap["outputs"]
                    .as_array()
                    .and_then(|ms| ms.iter().find(|m| m["name"] == json!(name)))
                    .map(|m| json!({ "monitor": name, "active_tag_mask": m["active_tag_mask"].clone(), "occupied_tag_mask": m["occupied_tag_mask"].clone() }))
                    .unwrap_or_else(|| json!({ "error": "no such monitor" })),
                None => json!({ "error": "usage: get tags <monitor>" }),
            },
            other => json!({ "error": format!("unknown topic: {other}") }),
        }
    }
}
```

- [ ] **Step 2: Compile check**

Run: `cargo check -p margo`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add margo/src/ipc/topics.rs
git commit -m "feat(ipc): per-topic JSON projections (clients/monitors/tags/…)"
```

---

## Phase 2 — Socket server on the calloop loop

### Task 2.1: Watch registry

**Files:**
- Modify: `margo/src/ipc/watch.rs`

- [ ] **Step 1: Implement the registry**

```rust
// margo/src/ipc/watch.rs
//! Tracks `watch`-mode client connections so a single state change can
//! be fanned out to every subscriber.

/// One active `watch` subscription.
pub struct Watch {
    /// calloop token identifying the client connection.
    pub token: u32,
    /// Topic the client subscribed to (e.g. "state").
    pub topic: String,
    /// Topic args (e.g. monitor name for `watch tags <mon>`).
    pub args: Vec<String>,
}

#[derive(Default)]
pub struct WatchRegistry {
    pub watches: Vec<Watch>,
}

impl WatchRegistry {
    pub fn add(&mut self, token: u32, topic: String, args: Vec<String>) {
        self.watches.push(Watch { token, topic, args });
    }
    pub fn remove_conn(&mut self, token: u32) {
        self.watches.retain(|w| w.token != token);
    }
}
```

- [ ] **Step 2: Hold a `WatchRegistry` + connection map on `MargoState`**

In `margo/src/state.rs`, add fields near `state_dirty`:

```rust
    /// Active IPC connections, keyed by a monotonic token.
    pub ipc_conns: std::collections::HashMap<u32, crate::ipc::server::IpcConn>,
    /// Next IPC connection token.
    pub ipc_next_token: u32,
    /// `watch`-mode subscriptions.
    pub ipc_watches: crate::ipc::watch::WatchRegistry,
```

Initialize in the constructor (`Self { … }`): `ipc_conns: Default::default(), ipc_next_token: 0, ipc_watches: Default::default(),`.

- [ ] **Step 3: Compile check + commit**

Run: `cargo check -p margo` (will fail until `IpcConn` exists — define a placeholder `pub struct IpcConn;` in server.rs for now). Expected: compiles.

```bash
git add margo/src/ipc/watch.rs margo/src/ipc/server.rs margo/src/state.rs
git commit -m "feat(ipc): watch registry + per-connection state on MargoState"
```

### Task 2.2: Listener + accept as a calloop source

**Files:**
- Modify: `margo/src/ipc/server.rs`
- Modify: `margo/src/ipc/mod.rs` (export `insert_ipc_source`)

- [ ] **Step 1: Implement the connection struct + listener insertion**

```rust
// margo/src/ipc/server.rs
use crate::state::MargoState;
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// Per-connection read buffer + framing.
pub struct IpcConn {
    pub stream: UnixStream,
    pub buf: Vec<u8>,
    /// True once this connection has issued a `watch` (stays open,
    /// receives pushed frames).
    pub watching: bool,
}

/// Bind the socket and register accept + per-client sources on the loop.
/// Removes any stale socket file first. Best-effort: a bind failure is
/// logged but never fatal (the compositor still runs).
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
    listener.set_nonblocking(true).ok();
    let source = Generic::new(listener, Interest::READ, Mode::Level);
    let res = handle.insert_source(source, |_, listener, state: &mut MargoState| {
        loop {
            match listener.accept() {
                Ok((stream, _)) => state.ipc_accept(stream),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => tracing::warn!(error = %e, "ipc: accept"),
            }
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = res {
        tracing::error!(error = %e, "ipc: insert accept source");
    }
}
```

- [ ] **Step 2: Re-export from mod.rs**

```rust
// in margo/src/ipc/mod.rs
pub use server::insert_ipc_source;
```

- [ ] **Step 3: Compile check + commit**

Run: `cargo check -p margo` (fails until `ipc_accept` exists — implement in next task; for now stub `impl MargoState { pub fn ipc_accept(&mut self, _s: UnixStream) {} }` in server.rs). Expected: compiles.

```bash
git add margo/src/ipc/server.rs margo/src/ipc/mod.rs
git commit -m "feat(ipc): bind socket + accept loop as calloop source"
```

### Task 2.3: Accept → per-client read source → request handling

**Files:**
- Modify: `margo/src/ipc/server.rs`

- [ ] **Step 1: Implement accept, read framing, and request handling**

```rust
impl MargoState {
    /// Register a freshly-accepted connection: stash it under a new
    /// token and add a calloop READ source that drains complete lines.
    pub fn ipc_accept(&mut self, stream: UnixStream) {
        stream.set_nonblocking(true).ok();
        let token = self.ipc_next_token;
        self.ipc_next_token = self.ipc_next_token.wrapping_add(1);
        let raw_fd = stream.try_clone().expect("ipc: clone fd");
        self.ipc_conns.insert(
            token,
            IpcConn { stream, buf: Vec::new(), watching: false },
        );
        let source = Generic::new(raw_fd, Interest::READ, Mode::Level);
        let res = self.loop_handle.insert_source(source, move |_, fd, state: &mut MargoState| {
            Ok(state.ipc_readable(token, fd))
        });
        if let Err(e) = res {
            tracing::warn!(error = %e, "ipc: insert client source");
            self.ipc_drop_conn(token);
        }
    }

    /// Drain readable bytes for one connection, handling each complete
    /// `\n`-terminated request line.
    fn ipc_readable(&mut self, token: u32, fd: &mut UnixStream) -> PostAction {
        let mut chunk = [0u8; 4096];
        loop {
            match fd.read(&mut chunk) {
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
                let Some(c) = self.ipc_conns.get_mut(&token) else { return };
                match c.buf.iter().position(|&b| b == b'\n') {
                    Some(pos) => {
                        let line: Vec<u8> = c.buf.drain(..=pos).collect();
                        String::from_utf8_lossy(&line[..line.len() - 1]).trim().to_string()
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
                let arg = crate::ipc::server::args_to_dispatch_arg(&req.args);
                crate::dispatch::dispatch_action(self, &req.head, &arg);
                self.ipc_send(token, &serde_json::json!({ "ok": true }));
            }
            Verb::Watch => {
                if let Some(c) = self.ipc_conns.get_mut(&token) {
                    c.watching = true;
                }
                self.ipc_watches.add(token, req.head.clone(), req.args.clone());
                // Prime with the current value immediately.
                let payload = self.ipc_topic(&req.head, &req.args);
                self.ipc_send(token, &payload);
            }
        }
    }

    /// Serialize + write a single JSON frame (newline-terminated) to a
    /// connection. Drops the connection on write error.
    pub fn ipc_send(&mut self, token: u32, value: &serde_json::Value) {
        let Some(c) = self.ipc_conns.get_mut(&token) else { return };
        let mut line = value.to_string();
        line.push('\n');
        if c.stream.write_all(line.as_bytes()).is_err() {
            self.ipc_drop_conn(token);
        }
    }

    pub fn ipc_drop_conn(&mut self, token: u32) {
        self.ipc_conns.remove(&token);
        self.ipc_watches.remove_conn(token);
    }
}
```

- [ ] **Step 2: Add the dispatch-arg helper**

```rust
// margo/src/ipc/server.rs (free fn)
use margo_config::Arg;

/// Map up to 5 positional dispatch args onto margo's `Arg` struct,
/// mirroring the old dwl-ipc dispatch slot semantics: slots 1-3 parse
/// as integers (i / i2 / f), slot 4 is the primary string (`v`),
/// slot 5 the secondary string (`v2`).
pub fn args_to_dispatch_arg(args: &[String]) -> Arg {
    let mut arg = Arg::default();
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
    // The single-string actions (spawn/theme/run_script) take the first
    // arg as `v` when it isn't numeric.
    if arg.v.is_none()
        && let Some(s) = args.first()
        && s.parse::<i64>().is_err()
    {
        arg.v = Some(s.clone());
    }
    arg
}
```

> Verify the `Arg` field names against `margo-config/src/types.rs` before implementing (`i`, `i2`, `f`, `v`, `v2`); adjust if the struct differs.

- [ ] **Step 3: Compile check**

Run: `cargo check -p margo`
Expected: compiles (`self.loop_handle` must exist on `MargoState` — it does; confirm field name).

- [ ] **Step 4: Commit**

```bash
git add margo/src/ipc/server.rs
git commit -m "feat(ipc): per-client read framing + get/dispatch/watch handling"
```

### Task 2.4: Register the IPC source at startup + export env

**Files:**
- Modify: `margo/src/main.rs:268-335` (after the event loop + loop_handle exist, alongside the wayland socket source)

- [ ] **Step 1: Insert the source + export env**

After `let loop_handle = event_loop.handle();` and once `MargoState` is constructed (the `margo` value), add:

```rust
crate::ipc::export_socket_env();
crate::ipc::insert_ipc_source(&loop_handle);
```

Place `export_socket_env()` BEFORE any child process is spawned (so `MARGO_SOCKET` is inherited). `insert_ipc_source` needs the loop handle that is stored on the state as `loop_handle` — confirm the state already holds a clone (`state.loop_handle`); the accept closure uses `self.loop_handle` to add per-client sources.

- [ ] **Step 2: Build + smoke test manually**

```bash
cargo build -p margo -p mctl
# In a nested session:
cargo run -p margo -- --winit &
sleep 2
printf 'get state\n' | socat - "UNIX-CONNECT:$XDG_RUNTIME_DIR/margo/margo-ipc.sock"
```
Expected: one line of JSON containing `"tag_count"`, `"clients"`, `"outputs"`.

- [ ] **Step 3: Commit**

```bash
git add margo/src/main.rs
git commit -m "feat(ipc): bind socket + export MARGO_SOCKET at startup"
```

---

## Phase 3 — Watch push on state change

### Task 3.1: Fan out frames when state is marked dirty

**Files:**
- Modify: `margo/src/state.rs` (the `state_dirty` flush point)
- Modify: `margo/src/ipc/watch.rs`

- [ ] **Step 1: Rename `write_state_file` → `mark_state_dirty`**

In `margo/src/state.rs` / `state/state_file.rs`, rename the public `write_state_file(&self)` setter to `mark_state_dirty(&self)` (it still just sets `state_dirty`). Update all call sites (grep `write_state_file`):

```bash
rg -l 'write_state_file' margo/src | xargs sed -i 's/write_state_file/mark_state_dirty/g'
```
(Leave `flush_state_file_if_dirty` for the next step.)

- [ ] **Step 2: Replace the flush to push to watchers instead of writing a file**

Rename `flush_state_file_if_dirty` → `flush_ipc_if_dirty` and change the body:

```rust
// margo/src/state/state_file.rs (or wherever the flush lives)
pub fn flush_ipc_if_dirty(&mut self) {
    if !self.state_dirty.replace(false) {
        return;
    }
    self.ipc_push_watches();
}
```

Add the fan-out:

```rust
// margo/src/ipc/watch.rs (impl on MargoState)
impl crate::state::MargoState {
    /// Push a fresh frame to every active watch subscription. Called
    /// once per loop iteration when state changed.
    pub fn ipc_push_watches(&mut self) {
        if self.ipc_watches.watches.is_empty() {
            return;
        }
        // Snapshot (topic, args, token) first to avoid borrow conflicts.
        let subs: Vec<(u32, String, Vec<String>)> = self
            .ipc_watches
            .watches
            .iter()
            .map(|w| (w.token, w.topic.clone(), w.args.clone()))
            .collect();
        for (token, topic, args) in subs {
            let payload = self.ipc_topic(&topic, &args);
            self.ipc_send(token, &payload);
        }
    }
}
```

- [ ] **Step 3: Wire the flush into the loop**

Find where `flush_state_file_if_dirty()` was called once per loop iteration (grep) and rename that call to `flush_ipc_if_dirty()`.

- [ ] **Step 4: Build + manual watch test**

```bash
cargo run -p margo -- --winit &
sleep 2
socat - "UNIX-CONNECT:$XDG_RUNTIME_DIR/margo/margo-ipc.sock" <<< 'watch state' &
# In another terminal, open/close a window or switch tags; the socat
# stream should print a fresh JSON line per change.
```
Expected: initial frame, then a frame on each change.

- [ ] **Step 5: Commit**

```bash
git add margo/src/state.rs margo/src/state/state_file.rs margo/src/ipc/watch.rs
git commit -m "feat(ipc): push watch frames on state change (replaces file write)"
```

### Task 3.2: Integration test in margo's test fixture

**Files:**
- Create: `margo/src/tests/ipc.rs`
- Modify: `margo/src/tests/mod.rs` (add `mod ipc;`)

- [ ] **Step 1: Write the test**

```rust
// margo/src/tests/ipc.rs
//! Drives the real socket server through the test Fixture: connect a
//! UnixStream, send `get state`, assert a well-formed snapshot comes
//! back; send `dispatch view 2`, assert the active tag changes on a
//! follow-up `get state`.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[test]
fn get_state_returns_snapshot() {
    let mut fx = crate::tests::Fixture::new();
    fx.add_output("WL-1", 1920, 1080);
    fx.dispatch();

    let mut sock = UnixStream::connect(crate::ipc::socket_path())
        .expect("connect ipc socket");
    sock.write_all(b"get state\n").unwrap();
    fx.dispatch();

    let mut reader = BufReader::new(sock.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert!(v["outputs"].as_array().unwrap().iter().any(|m| m["name"] == "WL-1"));
}
```

> Confirm the Fixture spins the real event loop (so `insert_ipc_source` ran). If the Fixture uses a headless test loop that doesn't call `insert_ipc_source`, add a `Fixture::enable_ipc()` helper that calls it, and invoke it in the test. Mirror the existing `margo/src/tests/` patterns (see `fixture.rs`).

- [ ] **Step 2: Run it**

Run: `cargo test -p margo ipc::get_state_returns_snapshot`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add margo/src/tests/ipc.rs margo/src/tests/mod.rs
git commit -m "test(ipc): end-to-end get state over the socket via fixture"
```

---

## Phase 4 — mctl rewrite onto the socket

### Task 4.1: Socket client helper (TDD for path resolution)

**Files:**
- Create: `mctl/src/ipc_client.rs`
- Modify: `mctl/src/lib.rs` (add `pub mod ipc_client;`)

- [ ] **Step 1: Implement the client**

```rust
// mctl/src/ipc_client.rs
//! Talks to margo's Unix-socket IPC. One request per line; replies are
//! newline-delimited JSON.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

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
    Ok(serde_json::from_str(line.trim()).unwrap_or_else(|_| serde_json::json!({ "error": "bad reply" })))
}

/// Send a `watch …` request and invoke `on_frame` for every JSON line
/// until the connection closes or the callback returns `false`.
pub fn watch_stream(req: &str, mut on_frame: impl FnMut(serde_json::Value) -> bool) -> std::io::Result<()> {
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
```

- [ ] **Step 2: Compile check + commit**

Run: `cargo check -p mctl`
Expected: compiles.

```bash
git add mctl/src/ipc_client.rs mctl/src/lib.rs
git commit -m "feat(mctl): Unix-socket client (request_once + watch_stream)"
```

### Task 4.2: Repoint existing subcommands + add `get` / `watch`

**Files:**
- Modify: `mctl/src/bin/mctl.rs`

- [ ] **Step 1: Replace the query path**

Everywhere mctl read `state.json` (via `mctl::read_state_json()` / file reads), replace with `ipc_client::request_once("get state")`. The downstream rendering (`status`, `clients`, `outputs`, `focused`) already parses the snapshot shape — keep that, just feed it the socket reply instead of the file. Example for `status`:

```rust
// old: let snap = read_state_json()?;
let snap = mctl::ipc_client::request_once("get state")?;
```

- [ ] **Step 2: Replace the dispatch path**

The `Dispatch { name, args }` arm previously sent a dwl-ipc Wayland request. Replace with:

```rust
Command::Dispatch { name, args } => {
    let mut req = format!("dispatch {name}");
    for a in &args {
        req.push(' ');
        req.push_str(a);
    }
    let reply = mctl::ipc_client::request_once(&req)?;
    if reply.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        eprintln!("dispatch failed: {reply}");
        std::process::exit(1);
    }
}
```

Do the same for the typed convenience subcommands that used to build dwl-ipc requests (`tags`, `layout`, `theme`, `reload`, `quit`, `twilight`, `session-save`, `session-load`, `client-tags`): translate each to the matching `dispatch <action> …` (e.g. `reload` → `request_once("dispatch reload")`, `layout N` → `request_once(&format!("dispatch setlayout {n}"))`). Confirm each action name against `mctl actions --names` / the dispatch table.

- [ ] **Step 3: Add `get` and `watch` subcommands**

```rust
/// Raw IPC query — one JSON line.
Get {
    /// Topic: state | clients | client <id> | monitors | monitor <name>
    ///        | tags <monitor> | focused | layouts | keyboard-layout | twilight
    #[arg(required = true, num_args = 1..)]
    topic: Vec<String>,
},
/// Stream a topic until interrupted (Ctrl-C).
Watch {
    #[arg(required = true, num_args = 1..)]
    topic: Vec<String>,
},
```

Handlers:

```rust
Command::Get { topic } => {
    let reply = mctl::ipc_client::request_once(&format!("get {}", topic.join(" ")))?;
    println!("{}", serde_json::to_string_pretty(&reply)?);
}
Command::Watch { topic } => {
    mctl::ipc_client::watch_stream(&format!("watch {}", topic.join(" ")), |frame| {
        println!("{frame}");
        true
    })?;
}
```

The existing `watch` (state stream) subcommand becomes `watch state` under the hood.

- [ ] **Step 4: Build + manual test**

```bash
cargo build -p mctl
mctl get state | head
mctl get clients
mctl dispatch view 2
mctl watch tags WL-1   # Ctrl-C to stop
```
Expected: JSON output; dispatch changes the tag; watch streams.

- [ ] **Step 5: Commit**

```bash
git add mctl/src/bin/mctl.rs
git commit -m "feat(mctl): route all commands over the socket + add get/watch"
```

---

## Phase 5 — Shell: subscribe instead of poll

### Task 5.1: Replace the file loop with a socket `watch state` subscription

**Files:**
- Modify: `mshell-crates/mshell-margo-client/src/state_json.rs`
- Modify: `mshell-crates/mshell-margo-client/src/sync.rs`

- [ ] **Step 1: Add a socket reader to state_json.rs (keep the `StateJson` types)**

Keep the `StateJson` / `RawClient` / `RawOutput` structs + `serde` parsing. Replace `read()` / `read_raw()` / `state_json_path()` with:

```rust
// mshell-crates/mshell-margo-client/src/state_json.rs
use std::path::PathBuf;

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
```

- [ ] **Step 2: Rewrite `sync::spawn` to connect + `watch state`**

```rust
// mshell-crates/mshell-margo-client/src/sync.rs
pub(crate) fn spawn(service: &Arc<MargoService>) {
    let weak = Arc::downgrade(service);
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        loop {
            // Reconnect loop — survive compositor restarts.
            let stream = match tokio::net::UnixStream::connect(crate::state_json::socket_path()).await {
                Ok(s) => s,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }
            };
            let (rd, mut wr) = stream.into_split();
            if wr.write_all(b"watch state\n").await.is_err() {
                continue;
            }
            let mut lines = BufReader::new(rd).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(service) = weak.upgrade() else { return };
                if let Ok(snap) = serde_json::from_str::<StateJson>(&line) {
                    apply_snapshot(&service, &snap);
                }
            }
            // Connection dropped → loop and reconnect after a short wait.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
}
```

Delete the inotify watcher + `FALLBACK_POLL_INTERVAL` ticker + `read_raw` usage. `apply_snapshot` is unchanged.

- [ ] **Step 3: Drop the inotify dependency if now unused**

Check `mshell-crates/mshell-margo-client/Cargo.toml` — if `inotify`/`notify` was only used by sync.rs, remove it.

- [ ] **Step 4: Build + run the shell against a live margo**

Run: `cargo build -p mshell` then restart mshell against a margo built from this branch. Expected: bar/tags/active-window update live with no file polling.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-margo-client/
git commit -m "feat(shell): subscribe to margo IPC socket (drop state.json polling)"
```

---

## Phase 6 — Migrate the remaining consumers

### Task 6.1: mlock wallpaper resolution

**Files:**
- Modify: `mlock/src/wallpaper.rs`

- [ ] **Step 1: Replace the state.json read with a socket `get state`**

Where mlock read `state.json` to find the per-tag wallpaper, shell out to mctl (mlock is short-lived, spawned at lock time):

```rust
// mlock/src/wallpaper.rs
fn margo_state() -> Option<serde_json::Value> {
    let out = std::process::Command::new("mctl").args(["get", "state"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}
```

Replace the previous file-read call with `margo_state()` and keep the same field access (`outputs[..].wallpaper`).

- [ ] **Step 2: Build + commit**

Run: `cargo build -p mlock`
```bash
git add mlock/src/wallpaper.rs
git commit -m "refactor(mlock): resolve wallpaper via mctl get state"
```

### Task 6.2: launcher providers + mshell-utils margo helper

**Files:**
- Modify: `mshell-crates/mshell-launcher/src/providers/mctl.rs`
- Modify: `mshell-crates/mshell-utils/src/margo.rs`
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/{tags_provider,windows_provider}.rs`

- [ ] **Step 1: Audit each reader**

For each file, find the `read_state_json()` / state.json file read and replace with the in-process `MargoService` reactive where one already exists (preferred — these run inside the shell, which already holds `margo_service()`), or `mctl get state` for out-of-process helpers. Prefer reading `margo_service().clients.get()` / `.monitors.get()` over re-reading the socket: the shell already mirrors the full snapshot.

- [ ] **Step 2: Build + commit**

Run: `cargo check -p mshell-launcher -p mshell-utils -p mshell-frame`
```bash
git add mshell-crates/
git commit -m "refactor(shell): read compositor state from MargoService, not state.json"
```

---

## Phase 7 — Rip out dwl-ipc-v2 + state.json

### Task 7.1: Delete the dwl-ipc Wayland protocol (server)

**Files:**
- Delete: `margo/src/protocols/dwl_ipc.rs`
- Delete: `protocols/dwl-ipc-unstable-v2.xml`
- Modify: `margo/src/protocols/mod.rs`, `margo/src/protocols/generated.rs`
- Modify: `margo/src/state.rs` (remove `Monitor.dwl_ipc` field + its updates)
- Modify: `margo/src/main.rs` (stop creating the `ZdwlIpcManagerV2` global)

- [ ] **Step 1: Remove registration + global**

Grep for `dwl_ipc`, `DwlIpc`, `ZdwlIpc` across `margo/src` and remove: the `delegate`/`GlobalDispatch`/`Dispatch` impls, the global creation in `main.rs`, the `Monitor.dwl_ipc` field and every `state.monitors[..].dwl_ipc…` mutation, and the `mod dwl_ipc;` line. Remove the dwl-ipc block from `generated.rs`.

- [ ] **Step 2: Delete files**

```bash
git rm margo/src/protocols/dwl_ipc.rs protocols/dwl-ipc-unstable-v2.xml
```

- [ ] **Step 3: Compile**

Run: `cargo check -p margo`
Expected: compiles with no `dwl_ipc` references remaining (`rg dwl_ipc margo/src` → empty).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(margo): remove dwl-ipc-unstable-v2 protocol entirely"
```

### Task 7.2: Delete the dwl-ipc client bindings from mctl

**Files:**
- Delete: `mctl/src/protocols/mod.rs` (and the `protocols/` dir if now empty)
- Modify: `mctl/src/lib.rs` (remove `IpcError`/`IpcEvent`/`IpcRequest` dwl-ipc types + `mod protocols`)

- [ ] **Step 1: Remove + compile**

```bash
git rm -r mctl/src/protocols
```
Remove `pub mod protocols;` and the now-dead dwl-ipc types from `mctl/src/lib.rs` / `mctl/src/client.rs`. Run: `cargo check -p mctl` — Expected: compiles (`rg dwl_ipc mctl/src` → empty).

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "refactor(mctl): drop dwl-ipc Wayland client bindings"
```

### Task 7.3: Delete the state.json file writer + readers

**Files:**
- Modify: `margo/src/state/state_file.rs` (remove `write_state_file_inner`, `state_file_path`, the temporary `build_state_snapshot` shim)
- Modify: `mshell-crates/mshell-margo-client/src/state_json.rs` (remove any leftover `read()`/`read_raw()`)

- [ ] **Step 1: Remove file I/O**

Delete `write_state_file_inner`, `state_file_path()`, and the `build_state_snapshot` shim (now that `ipc_state_snapshot` is canonical). Run `rg 'state\.json|state_file_path|write_state_file_inner' margo/src` → expected empty (only doc-comment mentions remain, which you should update).

- [ ] **Step 2: Compile the whole workspace**

Run: `cargo check --workspace`
Expected: compiles. `rg -l 'state_json|state\.json' --type rust` should now only match the shell's `state_json.rs` (the `StateJson` parse types, which we keep) and docs.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor(margo): delete state.json file writer (socket is the only IPC)"
```

---

## Phase 8 — Docs, man pages, completions, CI

### Task 8.1: Update the actions catalogue + completions note

**Files:**
- Modify: `mctl/src/actions.rs` (no protocol change, but the header doc references dwl-ipc — update it)
- Modify: `mctl/src/bin/mctl.rs` completions (the static completion text mentions dwl-ipc)

- [ ] **Step 1:** Update doc comments / help text that say "dwl-ipc-v2" to describe the socket protocol. Run: `mctl --help` and confirm no stale dwl-ipc references.
- [ ] **Step 2: Commit**

```bash
git add mctl/src
git commit -m "docs(mctl): describe socket IPC, drop dwl-ipc references"
```

### Task 8.2: Man pages + protocol doc

**Files:**
- Modify: `man/margo.1`, `man/mctl.1` (the IPC sections)
- Create: `docs/ipc.md` (the protocol spec from the top of this plan)
- Modify: `docs/protocol-matrix.md` (remove dwl-ipc-v2 advertised line; note it's gone)

- [ ] **Step 1:** Rewrite the IPC sections of `margo.1` and `mctl.1` to document the socket (path, `MARGO_SOCKET`, `get`/`watch`/`dispatch`, topics). Add `mctl get`/`mctl watch` to `mctl.1` COMMANDS. Copy the protocol spec into `docs/ipc.md`.
- [ ] **Step 2:** Lint: `groff -man -z man/margo.1 man/mctl.1` → no warnings.
- [ ] **Step 3: Commit**

```bash
git add man/ docs/ipc.md docs/protocol-matrix.md
git commit -m "docs(ipc): document the socket protocol (man + docs/ipc.md)"
```

### Task 8.3: Final workspace gate

- [ ] **Step 1:** `cargo fmt --all && cargo fmt --all -- --check` → clean.
- [ ] **Step 2:** `cargo clippy --workspace -- -D warnings` → clean (fix any lints inline).
- [ ] **Step 3:** `cargo test -p margo` → green (incl. the new `ipc` test).
- [ ] **Step 4:** Manual end-to-end: build margo + mctl + mshell from this branch, run a session, verify: bar updates live, `mctl get state` works, `mctl dispatch view 3` switches tags, `mctl watch tags <mon>` streams, lock screen wallpaper resolves.
- [ ] **Step 5: Commit any fixups + push**

```bash
git add -A && git commit -m "chore(ipc): workspace fmt/clippy/test green"
git push origin main
```

---

## Self-Review

**Spec coverage:**
- "Deprecate old dwl IPC, no trace" → Phase 7 (7.1 server, 7.2 client, 7.3 state.json). ✓
- "mctl gains all capabilities (get all-clients/monitors/tags, watch, dispatch)" → Phase 1 topics + Phase 4 (`get`/`watch` + repointed subcommands). ✓
- "Socket restructuring / single powerful syntax" → Protocol spec + Phase 2 server. ✓
- "Everything must work in the best way" → Phase 5 (shell push, no polling), Phase 6 (mlock/launcher migration), Phase 8 (tests/docs). ✓
- "Start from scratch, may break things" → no compat layer anywhere; old paths deleted in Phase 7. ✓

**Open verification points flagged for the implementer (not gaps — confirm against code):**
- `Arg` field names in `margo-config/src/types.rs` (Task 2.3).
- `MargoState.loop_handle` field name (Task 2.3 / 2.4).
- Whether the test `Fixture` runs `insert_ipc_source` (Task 3.2 — add `enable_ipc()` if not).
- Exact dispatch action names for the repointed typed subcommands (Task 4.2 — cross-check `mctl actions --names`).
- inotify/notify dependency removal (Task 5.1 step 3).

**Type consistency:** `ipc_state_snapshot` (1.1) is used by `ipc_topic` (1.2), `ipc_push_watches` (3.1), and the test (3.2). `request_once`/`watch_stream` (4.1) are used in 4.2. `socket_path()` defined in margo (0.1), mctl (4.1), and shell (5.1) — three copies by design (no shared crate dependency between compositor and shell); all resolve `MARGO_SOCKET` → `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`. Consistent.
