//! Clipboard history configuration.
//!
//! The clipboard watcher (`mshell-clipboard`) records every copy
//! that lands on the Wayland clipboard via `ext-data-control-v1`.
//! These knobs control how much is kept, whether it survives a
//! restart, when it's auto-cleared, and whether password-manager
//! copies are skipped.

use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What part of the history is written to disk (and reloaded on
/// the next launch). Default `All` matches what people expect from
/// a clipboard manager — the full rolling history survives a reboot.
/// `skip_sensitive` (on by default) already keeps password-manager
/// copies out of the store, so persisting everything stays safe;
/// drop to `FavoritesOnly` or `None` for a stricter privacy posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Store, JsonSchema)]
pub enum ClipboardPersist {
    /// Nothing on disk — history is RAM-only (clears on restart).
    None,
    /// Only pinned (favourite) entries persist across restarts.
    FavoritesOnly,
    /// The full rolling history persists across restarts.
    #[default]
    All,
}

impl PatchField for ClipboardPersist {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl ClipboardPersist {
    pub fn display_name(&self) -> &'static str {
        match self {
            ClipboardPersist::None => "Nothing (RAM only)",
            ClipboardPersist::FavoritesOnly => "Favorites only",
            ClipboardPersist::All => "Everything",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec!["Nothing (RAM only)", "Favorites only", "Everything"]
    }

    pub fn from_index(i: u32) -> Self {
        match i {
            0 => ClipboardPersist::None,
            2 => ClipboardPersist::All,
            _ => ClipboardPersist::FavoritesOnly,
        }
    }

    pub fn to_index(self) -> u32 {
        match self {
            ClipboardPersist::None => 0,
            ClipboardPersist::FavoritesOnly => 1,
            ClipboardPersist::All => 2,
        }
    }
}

/// When non-pinned history is automatically discarded. Default
/// `Never` — entries only leave when they fall off the
/// `max_entries` tail or the user clears manually. Pinned entries
/// are exempt from every policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Store, JsonSchema)]
pub enum ClipboardClearPolicy {
    /// Never auto-clear (only manual + max_entries tail eviction).
    #[default]
    Never,
    /// Drop non-pinned entries older than `clear_after_hours` (on
    /// startup and periodically).
    AfterHours,
    /// Drop non-pinned entries from the previous session on the
    /// next startup (a "fresh session each login" posture).
    OnLogout,
}

impl PatchField for ClipboardClearPolicy {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl ClipboardClearPolicy {
    pub fn display_name(&self) -> &'static str {
        match self {
            ClipboardClearPolicy::Never => "Never (manual only)",
            ClipboardClearPolicy::AfterHours => "After N hours",
            ClipboardClearPolicy::OnLogout => "On logout / restart",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec![
            "Never (manual only)",
            "After N hours",
            "On logout / restart",
        ]
    }

    pub fn from_index(i: u32) -> Self {
        match i {
            1 => ClipboardClearPolicy::AfterHours,
            2 => ClipboardClearPolicy::OnLogout,
            _ => ClipboardClearPolicy::Never,
        }
    }

    pub fn to_index(self) -> u32 {
        match self {
            ClipboardClearPolicy::Never => 0,
            ClipboardClearPolicy::AfterHours => 1,
            ClipboardClearPolicy::OnLogout => 2,
        }
    }
}

/// Row density of the clipboard history panel. `Comfortable` (default)
/// keeps the roomy padding; `Compact` tightens it so more entries fit
/// on screen at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Store, JsonSchema)]
pub enum ClipboardDensity {
    #[default]
    Comfortable,
    Compact,
}

impl PatchField for ClipboardDensity {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl ClipboardDensity {
    pub fn display_name(&self) -> &'static str {
        match self {
            ClipboardDensity::Comfortable => "Comfortable",
            ClipboardDensity::Compact => "Compact",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec!["Comfortable", "Compact"]
    }

    pub fn from_index(i: u32) -> Self {
        match i {
            1 => ClipboardDensity::Compact,
            _ => ClipboardDensity::Comfortable,
        }
    }

    pub fn to_index(self) -> u32 {
        match self {
            ClipboardDensity::Comfortable => 0,
            ClipboardDensity::Compact => 1,
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, reactive_stores::Patch, JsonSchema,
)]
#[serde(default)]
pub struct Clipboard {
    /// Max rolling-history entries before the oldest non-pinned
    /// entry is evicted. Pinned entries don't count against this.
    pub max_entries: usize,
    /// What persists to disk across restarts.
    pub persist: ClipboardPersist,
    /// Auto-clear policy for non-pinned entries.
    pub clear_policy: ClipboardClearPolicy,
    /// Hours threshold used when `clear_policy == AfterHours`.
    pub clear_after_hours: u32,
    /// Skip copies a password manager marked sensitive
    /// (`x-kde-passwordManagerHint` — KeePassXC / Bitwarden / KDE).
    pub skip_sensitive: bool,
    /// Keep image copies in history (off = text/binary only).
    pub image_history: bool,
    /// Skip image copies larger than this many KB (0 = no limit).
    /// Guards against huge screenshots flooding the rolling history.
    pub image_max_kb: u32,
    /// Row density of the history panel.
    pub density: ClipboardDensity,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            max_entries: 100,
            persist: ClipboardPersist::All,
            clear_policy: ClipboardClearPolicy::Never,
            clear_after_hours: 24,
            skip_sensitive: true,
            image_history: true,
            image_max_kb: 0,
            density: ClipboardDensity::Comfortable,
        }
    }
}
