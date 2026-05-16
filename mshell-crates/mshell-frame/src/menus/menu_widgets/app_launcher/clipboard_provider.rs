//! Clipboard history provider — `>clip` to list, Enter to
//! re-copy, `>clear` to wipe.
//!
//! Reuses the singleton `clipboard_service()` from
//! `mshell-clipboard` (already running because mshell's
//! notification/quick-actions code depends on it), so this
//! provider is essentially a thin search-and-dispatch facade.
//!
//! Lives in `mshell-frame` (not `mshell-launcher`) for the same
//! reason `AppsProvider` does: `mshell-clipboard` pulls
//! wayland-client / tokio that the launcher crate intentionally
//! stays clear of so its pure-data providers remain portable.

use mshell_clipboard::{ClipboardEntry, EntryPreview, clipboard_service};
use mshell_launcher::{LauncherItem, Provider};
use std::rc::Rc;

pub struct ClipboardProvider;

impl ClipboardProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClipboardProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Format the entry into a single line for the launcher row.
/// Strips newlines, collapses whitespace, truncates to 80 chars.
fn render_label(entry: &ClipboardEntry) -> String {
    match &entry.preview {
        EntryPreview::Text(text) => {
            let collapsed: String = text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            truncate(&collapsed, 80)
        }
        EntryPreview::Image { width, height, .. } => {
            format!("[image] {width}×{height}")
        }
        EntryPreview::Binary { mime_type, size } => {
            format!("[binary] {mime_type} · {} bytes", size)
        }
    }
}

fn render_description(entry: &ClipboardEntry) -> String {
    let kind = match &entry.preview {
        EntryPreview::Text(text) => format!("{} chars", text.chars().count()),
        EntryPreview::Image { .. } => "image".to_string(),
        EntryPreview::Binary { .. } => "binary".to_string(),
    };
    format!("Clipboard · {kind}")
}

fn icon_for_entry(entry: &ClipboardEntry) -> &'static str {
    match &entry.preview {
        EntryPreview::Text(_) => "edit-paste-symbolic",
        EntryPreview::Image { .. } => "image-x-generic-symbolic",
        EntryPreview::Binary { .. } => "application-octet-stream-symbolic",
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars));
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

impl Provider for ClipboardProvider {
    fn name(&self) -> &str {
        "Clipboard"
    }

    fn handles_search(&self) -> bool {
        // Only contributes through commands() / handles_command —
        // raw clipboard items would flood the empty-query browse
        // with noise the user didn't ask for.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q.starts_with(">clip") || q.starts_with(">clear")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![
            LauncherItem {
                id: "clipboard:palette".into(),
                name: ">clip".into(),
                description: "Browse clipboard history".into(),
                icon: "edit-paste-symbolic".into(),
                icon_is_path: false,
                score: 0.0,
                provider_name: "Clipboard".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            },
            LauncherItem {
                id: "clipboard:clear-palette".into(),
                name: ">clear".into(),
                description: "Wipe clipboard history".into(),
                icon: "edit-clear-all-symbolic".into(),
                icon_is_path: false,
                score: 0.0,
                provider_name: "Clipboard".into(),
                usage_key: None,
                on_activate: Rc::new(|| {
                    clipboard_service().clear_history();
                    mshell_launcher::notify::toast(
                        "Clipboard cleared",
                        "All entries removed",
                    );
                }),
            },
        ]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();

        // `>clear` shows the single "clear" confirm row.
        if q.starts_with(">clear") {
            return vec![LauncherItem {
                id: "clipboard:clear".into(),
                name: "Clear clipboard history".into(),
                description: "Press Enter to wipe every entry".into(),
                icon: "edit-clear-all-symbolic".into(),
                icon_is_path: false,
                score: 200.0,
                provider_name: "Clipboard".into(),
                usage_key: None,
                on_activate: Rc::new(|| {
                    clipboard_service().clear_history();
                    mshell_launcher::notify::toast(
                        "Clipboard cleared",
                        "All entries removed",
                    );
                }),
            }];
        }

        if !q.starts_with(">clip") {
            return Vec::new();
        }

        let filter = q.trim_start_matches(">clip").trim().to_ascii_lowercase();
        let entries = clipboard_service().history().entries();

        // History is returned MRU-first by the underlying store
        // (entries are pushed when copied). We preserve that
        // order in the results.
        entries
            .into_iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let label = render_label(&entry);
                if !filter.is_empty()
                    && !label.to_ascii_lowercase().contains(&filter)
                {
                    return None;
                }
                let entry_id = entry.id;
                let icon = icon_for_entry(&entry);
                let description = render_description(&entry);
                let preview_label = label.clone();
                // Score MRU: top entry 190, dropping 1 per slot
                // down to 90. Slightly under the live "Run: …"
                // command row (200) so a hot palette navigation
                // doesn't bury a fresh paste.
                let score = (190.0 - idx as f64).max(90.0);
                Some(LauncherItem {
                    id: format!("clipboard:entry:{entry_id}"),
                    name: label,
                    description,
                    icon: icon.into(),
                    icon_is_path: false,
                    score,
                    provider_name: "Clipboard".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        clipboard_service().copy_entry(entry_id);
                        mshell_launcher::notify::toast(
                            "Copied to clipboard",
                            preview_label.clone(),
                        );
                    }),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 80), "hello");
    }

    #[test]
    fn truncate_long_string_ellipsised() {
        let long = "x".repeat(200);
        let out = truncate(&long, 10);
        assert_eq!(out.chars().count(), 11); // 10 chars + ellipsis
        assert!(out.ends_with('…'));
    }

    #[test]
    fn handles_command_only_for_clip_prefixes() {
        let p = ClipboardProvider::new();
        assert!(p.handles_command(">clip"));
        assert!(p.handles_command(">clip foo"));
        assert!(p.handles_command(">clear"));
        assert!(!p.handles_command(">cmd"));
        assert!(!p.handles_command("clipboard"));
    }
}
