//! margo binding-mode indicator — Noctalia's
//! `plugin:mango-keymode-indicator`.
//!
//! margo doesn't yet expose the current keybind mode through
//! state.json, so this stays a placeholder: we read whatever the
//! optional `~/.cache/mshell/keymode` file says (a future margo
//! patch can write the active mode there on every change), and
//! default to "default" otherwise.

use std::path::PathBuf;

pub fn current() -> String {
    let path = cache_path();
    std::fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "default".to_string())
}

fn cache_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("mshell").join("keymode")
}
