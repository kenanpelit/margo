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
/// to `/run/user/<uid>/margo/...` when XDG is unset, empty or relative
/// (`margo_fs::xdg::runtime_dir`) — the same base dir the old state
/// snapshot used.
pub fn socket_path() -> PathBuf {
    // SAFETY: getuid() has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    socket_path_from(
        std::env::var_os("MARGO_SOCKET"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        uid,
    )
}

/// [`socket_path`] from explicit inputs (the testable core).
fn socket_path_from(
    margo_socket: Option<std::ffi::OsString>,
    xdg_runtime_dir: Option<std::ffi::OsString>,
    uid: u32,
) -> PathBuf {
    if let Some(p) = margo_socket {
        return PathBuf::from(p);
    }
    // Unset, empty or relative falls back (margo_fs::xdg::runtime_dir,
    // the same rule the shell's client uses).
    margo_fs::xdg::runtime_dir_from(xdg_runtime_dir)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")))
        .join("margo")
        .join("margo-ipc.sock")
}

/// Export `MARGO_SOCKET` so clients and spawned children can find the
/// socket without re-deriving the path.
///
/// If another margo session is already serving the resolved path (a
/// nested/dev margo started from inside a live session inherits its
/// `MARGO_SOCKET`), this one moves to a pid-suffixed sibling instead of
/// taking the live session's socket, and exports that, so its own children
/// talk to it and the live session keeps working.
pub fn export_socket_env() {
    let base = socket_path();
    let path = choose_socket_path(&base, std::process::id());
    if path != base {
        tracing::warn!(
            live = ?base,
            ours = ?path,
            "ipc: another margo is serving the socket; using a separate one"
        );
    }
    // SAFETY: called once during single-threaded startup, before any
    // threads that read the environment are spawned.
    unsafe { std::env::set_var("MARGO_SOCKET", path) };
}

/// Whether a server is accepting connections on `path` right now.
pub fn socket_is_live(path: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// The path this margo binds: `base`, unless a live server already answers
/// there, then `<stem>-<pid>.sock` next to it.
pub fn choose_socket_path(base: &std::path::Path, pid: u32) -> PathBuf {
    if !socket_is_live(base) {
        return base.to_path_buf();
    }
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "margo-ipc".to_string());
    base.with_file_name(format!("{stem}-{pid}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    #[test]
    fn socket_path_ignores_a_relative_runtime_dir() {
        let p = |sock: Option<&str>, rt: Option<&str>| {
            socket_path_from(sock.map(Into::into), rt.map(Into::into), 1000)
        };
        assert_eq!(
            p(Some("/tmp/x.sock"), Some("/run/user/1000")),
            std::path::PathBuf::from("/tmp/x.sock")
        );
        assert_eq!(
            p(None, Some("/run/user/1000")),
            std::path::PathBuf::from("/run/user/1000/margo/margo-ipc.sock")
        );
        // Unset, empty or relative: a relative socket path would bind /
        // connect under whatever the cwd is.
        for bad in [None, Some(""), Some("relative/run"), Some("run")] {
            assert_eq!(
                p(None, bad),
                std::path::PathBuf::from("/run/user/1000/margo/margo-ipc.sock"),
                "{bad:?}"
            );
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("margo-ipc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_free_or_stale_path_is_used_as_is() {
        let dir = temp_dir("choose-free");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("margo-ipc.sock");
        assert_eq!(choose_socket_path(&base, 42), base);
        // A stale socket file: bound once, nobody accepting any more.
        drop(UnixListener::bind(&base).unwrap());
        assert!(base.exists());
        assert_eq!(choose_socket_path(&base, 42), base);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_socket_is_not_reused() {
        let dir = temp_dir("choose-live");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("margo-ipc.sock");
        let _live = UnixListener::bind(&base).unwrap();
        assert_eq!(choose_socket_path(&base, 42), dir.join("margo-ipc-42.sock"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_refuses_to_clobber_a_live_socket() {
        let dir = temp_dir("bind-live");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("margo-ipc.sock");
        let live = UnixListener::bind(&path).unwrap();

        let err = server::bind_listener(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        // The live session's socket still works.
        assert!(socket_is_live(&path));
        drop(live);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_replaces_a_stale_socket_and_is_owner_only() {
        let dir = temp_dir("bind-perms");
        let path = dir.join("sub").join("margo-ipc.sock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(UnixListener::bind(&path).unwrap());

        let _l = server::bind_listener(&path).unwrap();

        assert!(socket_is_live(&path));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket mode {mode:o}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_creates_a_missing_directory_owner_only() {
        let dir = temp_dir("bind-dir");
        let path = dir.join("margo").join("margo-ipc.sock");

        let _l = server::bind_listener(&path).unwrap();

        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "dir mode {mode:o}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
