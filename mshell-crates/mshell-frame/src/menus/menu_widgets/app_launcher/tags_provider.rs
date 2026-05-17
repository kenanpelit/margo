//! `tag` palette — switch the focused output to a specific tag.
//!
//! Reads `state.json` to project the per-output active /
//! occupied tag masks, builds one row per tag (1-9), and on
//! activation runs `mctl tags <mask>` to swap the focused
//! output's view. Active and occupied tags get glyph indicators
//! so the user can spot at a glance which tags actually have
//! windows.
//!
//! Sits in `mshell-frame` (not `mshell-launcher`) because it
//! pulls `mshell-margo-client` for the state.json reader —
//! mirrors the pattern `WindowsProvider` already uses.

use mshell_launcher::{LauncherItem, Provider};
use mshell_margo_client::read_state_json;
use std::process::Command;
use std::rc::Rc;

/// Margo's per-output tag count. `mctl status` confirms 9 tags
/// per output; we hardcode this rather than reading
/// `state.tag_count` to stay deterministic when the file is
/// briefly unavailable (booting, config reload race).
const TAG_COUNT: u32 = 9;

pub struct TagsProvider;

impl TagsProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TagsProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of the focused output's tag state.
struct TagSnapshot {
    /// Output name (used in the description).
    monitor: String,
    /// `1 << (N-1)` bits — N is set iff tag N is currently
    /// visible on the output.
    active_mask: u32,
    /// `1 << (N-1)` bits — N is set iff tag N has at least one
    /// client.
    occupied_mask: u32,
}

fn snapshot() -> Option<TagSnapshot> {
    let state = read_state_json()?;
    let active = state.active_output.clone();
    let out = state.outputs.iter().find(|o| o.name == active)?;
    Some(TagSnapshot {
        monitor: out.name.clone(),
        active_mask: out.active_tag_mask,
        occupied_mask: out.occupied_tag_mask,
    })
}

impl Provider for TagsProvider {
    fn name(&self) -> &str {
        "Tags"
    }

    fn category(&self) -> &str {
        "Compositor"
    }

    fn handles_search(&self) -> bool {
        // Don't bleed nine tag rows into the regular app
        // browse. Reach tags via the `tag` prefix.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "tag" || q.starts_with("tag ") || q == "tags" || q.starts_with("tags ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "tags:palette".into(),
            name: "tag".into(),
            description: "Switch the focused output to a specific tag (1-9)".into(),
            icon: "view-grid-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Tags".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "tag" || q.starts_with("tag ") || q == "tags" || q.starts_with("tags ")) {
            return Vec::new();
        }
        let filter = q
            .trim_start_matches("tags")
            .trim_start_matches("tag")
            .trim()
            .to_ascii_lowercase();

        let snap = snapshot();
        let monitor = snap
            .as_ref()
            .map(|s| s.monitor.clone())
            .unwrap_or_else(|| "(no monitor)".into());

        (1..=TAG_COUNT)
            .filter(|n| {
                // Numeric filter — `tag 3` shows only tag 3.
                // Empty filter shows everything.
                if filter.is_empty() {
                    return true;
                }
                filter == n.to_string() || format!("tag {n}").contains(&filter)
            })
            .enumerate()
            .map(|(idx, n)| {
                let mask: u32 = 1 << (n - 1);
                let active = snap.as_ref().map(|s| s.active_mask & mask != 0).unwrap_or(false);
                let occupied = snap.as_ref().map(|s| s.occupied_mask & mask != 0).unwrap_or(false);
                let glyph = match (active, occupied) {
                    (true, _) => "● ",     // currently visible
                    (false, true) => "◐ ", // has windows but hidden
                    (false, false) => "○ ", // empty
                };
                let descr = format!(
                    "{monitor} · {}",
                    match (active, occupied) {
                        (true, true) => "active · has windows",
                        (true, false) => "active · empty",
                        (false, true) => "hidden · has windows",
                        (false, false) => "hidden · empty",
                    }
                );
                let mask_str = mask.to_string();
                LauncherItem {
                    id: format!("tags:{n}"),
                    name: format!("{glyph}Tag {n}"),
                    description: descr,
                    icon: if active {
                        "view-grid-symbolic".into()
                    } else if occupied {
                        "view-grid-symbolic".into()
                    } else {
                        "view-grid-symbolic".into()
                    },
                    icon_is_path: false,
                    // Ordered 1..9; score puts tag 1 at top.
                    score: 200.0 - idx as f64,
                    provider_name: "Tags".into(),
                    usage_key: Some(format!("tags:{n}")),
                    on_activate: Rc::new(move || {
                        if let Err(err) = Command::new("mctl").args(["tags", &mask_str]).spawn() {
                            tracing::warn!(?err, mask = mask_str, "tags provider switch failed");
                        }
                    }),
                }
            })
            .collect()
    }

    /// Compositor tab — show the 9 tag rows without the `tag`
    /// prefix; `filter` narrows to the matching tag number.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("tag")
        } else {
            self.search(&format!("tag {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = TagsProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn handles_command_only_for_tag_prefix() {
        let p = TagsProvider::new();
        assert!(p.handles_command("tag"));
        assert!(p.handles_command("tag 3"));
        assert!(p.handles_command("tags"));
        assert!(!p.handles_command("tagged"));
        assert!(!p.handles_command("firefox"));
    }
}
