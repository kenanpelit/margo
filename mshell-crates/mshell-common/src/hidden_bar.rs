//! Shared Hidden Bar IPC verb.
//!
//! Travels mshellctl → `mshell-core` (D-Bus) → `relm_app` → `Frame` → `Bar`
//! → the `HiddenBar` widget. Lives here in `mshell-common` because both
//! `mshell-core` (publisher) and `mshell-frame` (consumer) depend on it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenBarVerb {
    /// Toggle expanded/collapsed.
    Toggle,
    /// Expand the drawer.
    Expand,
    /// Collapse the drawer (no-op while pinned).
    Collapse,
    /// Expand and pin open (disable auto-collapse).
    Pin,
    /// Unpin (resume auto-collapse).
    Unpin,
}

impl HiddenBarVerb {
    /// Parse the CLI / IPC action string. `None` for anything unrecognised.
    pub fn from_action(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "toggle" => Some(Self::Toggle),
            "expand" => Some(Self::Expand),
            "collapse" => Some(Self::Collapse),
            "pin" => Some(Self::Pin),
            "unpin" => Some(Self::Unpin),
            _ => None,
        }
    }
}
