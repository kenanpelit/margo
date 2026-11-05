//! Notes & social config: the Notes Hub's Markdown folder and the GitHub
//! pill / menu.
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

/// What the GitHub bar pill shows beside its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum GithubPillMetric {
    /// Unread notifications (with the corner dot) — the original pill.
    #[default]
    Unread,
    /// Contributions made today.
    ContribToday,
    /// The current contribution streak, in days.
    ContribStreak,
    /// Contributions in the last year.
    ContribTotal,
}

impl GithubPillMetric {
    pub fn all() -> &'static [Self] {
        &[
            Self::Unread,
            Self::ContribToday,
            Self::ContribStreak,
            Self::ContribTotal,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unread => "Unread notifications",
            Self::ContribToday => "Contributions today",
            Self::ContribStreak => "Contribution streak",
            Self::ContribTotal => "Contributions this year",
        }
    }

    /// Needs the contribution calendar (a slower, separate poll).
    pub fn uses_contributions(self) -> bool {
        matches!(
            self,
            Self::ContribToday | Self::ContribStreak | Self::ContribTotal
        )
    }
}

impl PatchField for GithubPillMetric {
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

/// GitHub pill + menu settings (Settings → Widgets → GitHub).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct GithubConfig {
    /// What the bar pill counts.
    pub pill_metric: GithubPillMetric,
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

    #[test]
    fn github_defaults_keep_the_unread_pill() {
        let cfg: Config = yaml_serde::from_str("{}").unwrap();
        assert_eq!(cfg.github.pill_metric, GithubPillMetric::Unread);
        let cfg: Config = yaml_serde::from_str("github:\n  pill_metric: ContribStreak\n").unwrap();
        assert!(cfg.github.pill_metric.uses_contributions());
    }
}
