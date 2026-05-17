//! Open-windows provider — lists every margo client from
//! `state.json` and focuses the chosen one via an `mctl tags <mask>`
//! dispatch.
//!
//! Lives in `mshell-frame` (not `mshell-launcher`) because it
//! pulls `mshell-margo-client` for the `state.json` reader — the
//! launcher crate intentionally stays free of compositor-specific
//! dependencies so its providers cross-compile to non-margo hosts
//! if we ever extract it.
//!
//! ## Focus semantics
//!
//! Margo's IPC doesn't expose "focus this specific window by id"
//! today — the available dispatches are directional
//! (`focusstack`, `focusdir`) or tag-based (`view <mask>`). For
//! Phase 2 we settle on the tag switch:
//!
//! 1. Read the chosen client's `tags` bitmask from state.json.
//! 2. Dispatch `mctl tags <mask>` so the user's view jumps to a
//!    tag that contains the window.
//!
//! When the window was most-recently-focused on that tag (the
//! common case for the switcher) it ends up focused. Cross-output
//! switching isn't handled — the user moves the cursor or alt-tabs
//! the rest of the way. A future iteration can chain a
//! `focusmon` + targeted focus dispatch when margo gains a direct
//! "focus by id" action.

use mshell_launcher::{LauncherItem, Provider};
use mshell_margo_client::read_state_json;
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

/// A single open-window candidate. Snapshotted from state.json
/// on every search so the list always reflects current focus
/// state — desktop entries the user just closed disappear, newly
/// opened ones show up.
struct WindowEntry {
    /// Pre-formatted display label (`<app_id> · <title>`). Kept
    /// pre-built so search loops don't re-allocate per
    /// keystroke.
    label: String,
    /// Description line — monitor + tag number.
    description: String,
    /// Tag bitmask used to build the `mctl tags <mask>` command.
    tag_mask: u32,
    /// Stable id: `windows:<monitor>:<idx>` so re-ordered
    /// state.json snapshots still match the same row.
    id: String,
    /// Whether this entry is currently the focused window —
    /// surfaced in the description so the switcher reads
    /// naturally.
    focused: bool,
}

pub struct WindowsProvider {
    matcher: RefCell<Matcher>,
}

impl WindowsProvider {
    pub fn new() -> Self {
        Self {
            matcher: RefCell::new(Matcher::new(Config::DEFAULT)),
        }
    }

    /// Re-read state.json and project the client list into our
    /// flat `WindowEntry` snapshot. Filters out scratchpad +
    /// minimized + zero-tag clients which don't have a useful
    /// "switch to" semantics in a launcher context.
    fn snapshot(&self) -> Vec<WindowEntry> {
        let Some(state) = read_state_json() else {
            return Vec::new();
        };
        state
            .clients
            .iter()
            .filter(|c| !c.scratchpad && !c.minimized && c.tags != 0)
            .map(|c| {
                let title = if c.title.is_empty() {
                    "(untitled)".to_string()
                } else {
                    c.title.clone()
                };
                let label = if c.app_id.is_empty() {
                    title.clone()
                } else {
                    format!("{} · {}", c.app_id, title)
                };
                let tag_num = first_tag_index(c.tags);
                let description = format!(
                    "{}{} · tag {}",
                    c.monitor,
                    if c.focused { " · focused" } else { "" },
                    tag_num + 1
                );
                WindowEntry {
                    label,
                    description,
                    tag_mask: c.tags,
                    id: format!("windows:{}:{}", c.monitor, c.idx),
                    focused: c.focused,
                }
            })
            .collect()
    }

    fn make_item(&self, entry: &WindowEntry, score: f64) -> LauncherItem {
        let tag_mask = entry.tag_mask;
        let monitor = entry.id.clone();
        LauncherItem {
            id: entry.id.clone(),
            name: entry.label.clone(),
            description: entry.description.clone(),
            icon: if entry.focused {
                "go-jump-symbolic".into()
            } else {
                "window-symbolic".into()
            },
            icon_is_path: false,
            score,
            provider_name: "Windows".into(),
            usage_key: Some(monitor),
            on_activate: Rc::new(move || {
                if let Err(err) = Command::new("mctl")
                    .arg("tags")
                    .arg(tag_mask.to_string())
                    .spawn()
                {
                    tracing::warn!(?err, mask = tag_mask, "windows provider focus dispatch failed");
                }
            }),
        }
    }
}

impl Default for WindowsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for WindowsProvider {
    fn name(&self) -> &str {
        "Windows"
    }

    fn category(&self) -> &str {
        "Compositor"
    }

    fn handles_command(&self, query: &str) -> bool {
        // Explicit `win` / `windows` prefix as an alternative to
        // the empty-browse + fuzzy access path. Useful when the
        // user wants the alt-tab style switcher without
        // scrolling past calculator / mctl / settings rows.
        let q = query.trim_start();
        q == "win" || q.starts_with("win ") || q == "windows" || q.starts_with("windows ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "windows:palette".into(),
            name: "win".into(),
            description: "List open windows (alt-tab style)".into(),
            icon: "window-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Windows".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let entries = self.snapshot();
        let trimmed = query.trim();

        // Strip the optional `win` / `windows` prefix so the
        // command-mode path treats `win firefox` and bare
        // `firefox` identically — the prefix is a discoverability
        // hint, not a different query language.
        let filter = if let Some(rest) = trimmed.strip_prefix("windows") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("win") {
            rest.trim()
        } else {
            trimmed
        };

        // Empty-query browse: surface every open window in
        // state.json order so users can use the launcher as an
        // alt-tab replacement (open launcher → see windows →
        // pick).
        if filter.is_empty() {
            return entries
                .iter()
                .map(|e| self.make_item(e, 0.0))
                .collect();
        }

        let mut matcher = self.matcher.borrow_mut();
        let pattern = Pattern::parse(filter, CaseMatching::Ignore, Normalization::Smart);

        entries
            .iter()
            .filter_map(|entry| {
                let mut buf = Vec::new();
                let hay = Utf32Str::new(&entry.label, &mut buf);
                pattern
                    .score(hay, &mut matcher)
                    .map(|raw| self.make_item(entry, raw as f64))
            })
            .collect()
    }
}

/// Lowest set-bit index in `mask`. Margo encodes tag membership
/// as `1 << (tag_num - 1)` so `mask=4` → bit 2 → tag 3. Returns
/// 0 for an empty mask (caller filters zero-tag clients
/// upstream).
fn first_tag_index(mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    mask.trailing_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tag_index_decodes_bitmask() {
        assert_eq!(first_tag_index(0b0001), 0); // tag 1
        assert_eq!(first_tag_index(0b0010), 1); // tag 2
        assert_eq!(first_tag_index(0b0100), 2); // tag 3
        assert_eq!(first_tag_index(0b10000000), 7); // tag 8
        assert_eq!(first_tag_index(0), 0); // fallback
    }
}
