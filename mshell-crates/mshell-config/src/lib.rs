pub mod atomic_write;
pub mod config_manager;
pub mod config_utils;
pub mod control_center_tiles;
pub mod doc_store;
mod errors;
pub mod migration;
pub mod paths;
mod persist;
pub mod plugin_bridge;
pub mod schema;
pub mod toast;

/// Tests that touch `$HOME` / `$XDG_*` (every path in `paths.rs` derives from them) share
/// one lock so they never race each other.
#[cfg(test)]
pub(crate) mod test_home {
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    /// Points `$HOME` at a fresh temp dir for the guard's lifetime and
    /// restores the previous value (and removes the dir) on drop.
    pub(crate) struct TempHome {
        pub(crate) path: PathBuf,
        prev: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _guard: MutexGuard<'static, ()>,
    }

    /// Every variable `margo_fs::xdg` reads for the paths in `paths.rs`.
    /// The XDG ones are cleared so a developer's own `$XDG_CONFIG_HOME`
    /// can never send a test's writes into the real config dir.
    const VARS: [&str; 3] = ["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"];

    impl TempHome {
        pub(crate) fn new(tag: &str) -> Self {
            let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("margo-{tag}-{}-{nanos}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            let prev = VARS.iter().map(|v| (*v, std::env::var_os(v))).collect();
            // SAFETY: every $HOME-mutating test holds HOME_LOCK.
            unsafe {
                std::env::set_var("HOME", &path);
                std::env::remove_var("XDG_CONFIG_HOME");
                std::env::remove_var("XDG_CACHE_HOME");
            }
            Self {
                path,
                prev,
                _guard: guard,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            // SAFETY: still holding HOME_LOCK (the guard drops after this).
            for (var, prev) in self.prev.drain(..) {
                match prev {
                    Some(v) => unsafe { std::env::set_var(var, v) },
                    None => unsafe { std::env::remove_var(var) },
                }
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
