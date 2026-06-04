//! Runtime clipboard settings.
//!
//! Mirrors `mshell_config::schema::clipboard::Clipboard` but lives
//! here so this crate stays free of a config dependency. The shell
//! maps its YAML config onto this struct at startup via
//! [`crate::init_settings`] and on live changes via
//! [`crate::ClipboardWatcher::apply_settings`].

/// What part of history persists to disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistMode {
    None,
    FavoritesOnly,
    All,
}

/// When non-pinned history is auto-discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearPolicy {
    Never,
    AfterHours,
    OnLogout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardSettings {
    pub max_entries: usize,
    pub persist: PersistMode,
    pub clear_policy: ClearPolicy,
    pub clear_after_hours: u32,
    pub skip_sensitive: bool,
    pub image_history: bool,
    /// Skip image copies larger than this many KB (0 = no limit).
    pub image_max_kb: u32,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            max_entries: 100,
            persist: PersistMode::FavoritesOnly,
            clear_policy: ClearPolicy::Never,
            clear_after_hours: 24,
            skip_sensitive: true,
            image_history: true,
            image_max_kb: 0,
        }
    }
}
