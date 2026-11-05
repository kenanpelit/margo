//! Notes & social config: the Notes Hub's Markdown folder.
//!
//! Every key is inert by default (a blank folder hides nothing and starts
//! no I/O), so an existing profile behaves exactly as before.

use super::*;

/// Notes Hub settings beyond the `notes.json` cards (Settings → Widgets →
/// Notes Hub).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct NotesConfig {
    /// Folder of Markdown notes for the "Files" tab (`~` is expanded).
    /// Blank = the tab only explains how to turn it on.
    pub folder: String,
    /// Commit the folder with `git` after edits (runs `git init` once if it
    /// is not a repository yet; never pushes). Refused for `$HOME` itself.
    pub git_history: bool,
    /// Let the AI provider (Settings → AI) tidy a capture into one todo line
    /// before it is appended to `Inbox.md`. Off = the raw text is appended.
    pub ai_capture: bool,
    /// How many of the best-matching notes a question sends as context.
    /// Clamped 1..=12 when used.
    pub ai_context_notes: u32,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            folder: String::new(),
            git_history: false,
            ai_capture: false,
            ai_context_notes: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_defaults_are_inert_and_partial_yaml_fills_the_rest() {
        let cfg: Config = yaml_serde::from_str("{}").unwrap();
        assert_eq!(cfg.notes, NotesConfig::default());
        assert!(cfg.notes.folder.is_empty() && !cfg.notes.git_history);
        let cfg: Config = yaml_serde::from_str("notes:\n  folder: ~/notes\n").unwrap();
        assert_eq!(cfg.notes.folder, "~/notes");
        assert_eq!(cfg.notes.ai_context_notes, 4);
    }
}
