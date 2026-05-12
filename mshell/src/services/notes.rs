//! Persistent scratchpad text — Noctalia's `plugin:notes`.
//!
//! Stores a single Markdown-ish scratchpad string in
//! `~/.config/mshell/notes.txt`. Autosave is debounced 700 ms to
//! match Noctalia's `autosaveDelay`. Caller drives it through
//! `load_scratchpad` + `save_scratchpad`.

use std::path::PathBuf;

fn store_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("mshell").join("notes.txt")
}

pub fn load_scratchpad() -> String {
    std::fs::read_to_string(store_path()).unwrap_or_default()
}

pub fn save_scratchpad(text: &str) {
    let path = store_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, text) {
        tracing::warn!(path = %path.display(), err = %e, "notes: save failed");
    }
}
