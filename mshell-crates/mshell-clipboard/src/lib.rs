mod entry;
mod history;
mod persist;
mod settings;
mod thumbnail;
mod watcher;

pub use entry::{ClipCategory, ClipboardEntry, EntryPreview, EntryView};
pub use history::ClipboardHistory;
pub use settings::{ClearPolicy, ClipboardSettings, PersistMode};
pub use watcher::{ClipboardEvent, ClipboardWatcher};

use std::sync::OnceLock;

static CLIPBOARD: OnceLock<ClipboardWatcher> = OnceLock::new();
static SETTINGS: OnceLock<ClipboardSettings> = OnceLock::new();

/// Seed the clipboard settings from the shell's config. Must be
/// called before the first `clipboard_service()` access (i.e. early
/// in mshell startup); a later call is a no-op because the watcher
/// is already running with whatever was set first. Use
/// `clipboard_service().apply_settings(..)` for live changes after
/// startup.
pub fn init_settings(settings: ClipboardSettings) {
    let _ = SETTINGS.set(settings);
}

pub fn clipboard_service() -> &'static ClipboardWatcher {
    CLIPBOARD.get_or_init(|| {
        let settings = SETTINGS.get().copied().unwrap_or_default();
        ClipboardWatcher::start(settings).expect("Failed to start clipboard watcher")
    })
}
