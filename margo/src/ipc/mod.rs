//! margo's Unix-domain-socket IPC. A newline-delimited text request /
//! JSON reply protocol exposing `get`, `watch`, and `dispatch`.
//!
//! Replaces the legacy dwl-ipc-v2 Wayland protocol and the polled
//! state snapshot file — clients connect to `$XDG_RUNTIME_DIR/margo/
//! margo-ipc.sock` (also exported as `MARGO_SOCKET`) and speak the
//! line protocol documented in `docs/ipc.md`.

pub mod protocol;
pub mod server;
pub mod topics;
pub mod watch;

pub use server::insert_ipc_source;

use std::path::PathBuf;

/// Resolve the IPC socket path. `$MARGO_SOCKET` wins when set — this
/// is what `mctl` / `mshellctl` already honour, so the server MUST
/// honour it too, otherwise a nested/dev margo started with a custom
/// `MARGO_SOCKET` would silently bind (and clobber, since we
/// `remove_file` first) the *live* session's socket. Without the
/// override it's `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`, falling back
/// to `/run/user/<uid>/margo/...` when XDG is unset — the same base
/// dir the old state snapshot used.
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

/// Export `MARGO_SOCKET` so clients and spawned children can find the
/// socket without re-deriving the path.
pub fn export_socket_env() {
    // SAFETY: called once during single-threaded startup, before any
    // threads that read the environment are spawned.
    unsafe { std::env::set_var("MARGO_SOCKET", socket_path()) };
}
