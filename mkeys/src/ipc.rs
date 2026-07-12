use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

use tracing::{error, info};

use crate::service::IPCHandle;

/// Hard cap on a single IPC message. Toggle commands are a handful of bytes;
/// this only exists so a misbehaving client can't stream unbounded data into
/// the single-threaded service loop and grow its memory without limit.
const MAX_MSG_LEN: u64 = 4096;

/// How long to wait for a connected client to finish sending before giving up,
/// so a peer that connects and then stalls can't wedge the accept loop. The
/// real client writes then drops the stream, so EOF normally arrives at once
/// and this never fires.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-session control socket. Prefer the user-private `$XDG_RUNTIME_DIR/margo/`
/// (mode 0700) over the world-writable `/tmp`, falling back to `/tmp` only when
/// the runtime dir is unset. `WAYLAND_DISPLAY` is only a session discriminator
/// here; it is validated so a stray value can't inject a path component
/// (`/` or `..`) into the socket path.
fn socket_path() -> String {
    let raw = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let display = if raw.is_empty() || raw.contains('/') || raw.contains("..") {
        "wayland-0".to_string()
    } else {
        raw
    };
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR")
        && !rt.is_empty()
    {
        let dir = format!("{rt}/margo");
        // Best-effort: create the parent 0700 (owner-only). If this fails the
        // subsequent bind surfaces the real error.
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        return format!("{dir}/mkeys-{display}.sock");
    }
    format!("/tmp/margo-mkeys-{display}.sock")
}

/// Bind the listener, then lock the socket file down to owner-only (0600) so no
/// other local user can drive the on-screen keyboard.
fn bind_secured(path: &str) -> std::io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

pub struct Ipc {
    socket: Option<UnixListener>,
}

impl Ipc {
    pub fn init() -> Self {
        let path = socket_path();
        match bind_secured(&path) {
            Ok(listener) => Self {
                socket: Some(listener),
            },
            Err(e) if e.kind() == ErrorKind::AddrInUse => {
                // The path exists: either a live instance, or a stale socket
                // from a crash. If nothing accepts a connection, it's stale.
                if UnixStream::connect(&path).is_ok() {
                    info!("mkeys: another instance is already running");
                    Self { socket: None }
                } else {
                    let _ = std::fs::remove_file(&path);
                    match bind_secured(&path) {
                        Ok(listener) => Self {
                            socket: Some(listener),
                        },
                        Err(e) => {
                            error!("mkeys: cannot bind {path}: {e}");
                            Self { socket: None }
                        }
                    }
                }
            }
            Err(e) => {
                error!("mkeys: cannot bind {path}: {e}");
                Self { socket: None }
            }
        }
    }

    pub fn is_single_instance(&self) -> bool {
        self.socket.is_some()
    }

    pub fn clean_up() {
        if let Err(e) = std::fs::remove_file(socket_path())
            && e.kind() != ErrorKind::NotFound
        {
            error!("mkeys: socket cleanup failed: {e}");
        }
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if self.socket.is_some() {
            Ipc::clean_up();
        }
    }
}

impl IPCHandle for Ipc {
    fn send(&self, data: &[u8]) {
        if let Ok(mut stream) = UnixStream::connect(socket_path()) {
            let _ = stream.write_all(data);
        }
    }

    fn read(&self) -> Vec<u8> {
        let Some(listener) = &self.socket else {
            return vec![];
        };
        match listener.accept() {
            Ok((stream, _)) => {
                // Bound both time and size so a client that connects then stalls
                // (or floods) can't wedge the single-threaded loop or grow
                // memory without limit. On timeout/cap `read_to_end` errors but
                // leaves whatever arrived in `data`, which we still return.
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let mut data = Vec::new();
                let _ = stream.take(MAX_MSG_LEN).read_to_end(&mut data);
                data
            }
            Err(e) => {
                info!("mkeys: accept failed: {e}");
                vec![]
            }
        }
    }
}
