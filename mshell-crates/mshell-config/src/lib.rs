pub mod atomic_write;
pub mod config_manager;
pub mod config_utils;
pub mod doc_store;
mod errors;
pub mod migration;
pub mod paths;
pub mod plugin_bridge;
pub mod schema;
pub mod toast;

/// Tests that touch `$HOME` (every path in `paths.rs` derives from it) share
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
        prev: Option<std::ffi::OsString>,
        _guard: MutexGuard<'static, ()>,
    }

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
            let prev = std::env::var_os("HOME");
            // SAFETY: every $HOME-mutating test holds HOME_LOCK.
            unsafe { std::env::set_var("HOME", &path) };
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
            match self.prev.take() {
                Some(v) => unsafe { std::env::set_var("HOME", v) },
                None => unsafe { std::env::remove_var("HOME") },
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
