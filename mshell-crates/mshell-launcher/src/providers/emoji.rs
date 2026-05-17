//! `:<query>` — emoji picker.
//!
//! Wraps the [`emojis`] crate's curated Unicode CLDR table
//! (~1900 emoji). Each query searches the official short-name
//! and tag list (`fire`, `joy`, `ok`, `rocket` …). Activation
//! copies the emoji character to the wl-clipboard.
//!
//! ## Behaviour
//!
//! | Query | Result |
//! |---|---|
//! | `:` (bare) | Up to 100 most-common emoji (CLDR order) |
//! | `:<query>` | Substring filter on name + every keyword |

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::rc::Rc;

/// Hard cap on the bare-`:` browse list. The full 1900-entry
/// dump would scroll forever — show enough to be useful, force
/// the rest behind a filter.
const BROWSE_LIMIT: usize = 100;

/// Cap on filtered results too — even narrow queries like `:a`
/// match hundreds. Past 100 the user should refine.
const SEARCH_LIMIT: usize = 200;

pub struct EmojiProvider;

impl EmojiProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmojiProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn match_score(emoji: &emojis::Emoji, query: &str) -> Option<f64> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return Some(0.0);
    }
    let name = emoji.name();
    if name.starts_with(&q) {
        return Some(180.0);
    }
    if name.contains(&q) {
        return Some(130.0);
    }
    if let Some(short) = emoji.shortcode()
        && short.to_ascii_lowercase().contains(&q)
    {
        return Some(150.0);
    }
    None
}

impl Provider for EmojiProvider {
    fn name(&self) -> &str {
        "Emoji"
    }

    fn category(&self) -> &str {
        "Insert"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with(':')
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "emoji:palette".into(),
            name: ":emoji".into(),
            description: "Pick an emoji (search by name or :shortcode)".into(),
            icon: "face-smile-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Emoji".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();
        if !trimmed.starts_with(':') {
            return Vec::new();
        }
        let filter = trimmed.trim_start_matches(':').trim();
        self.build_items(filter)
    }

    /// Insert tab — surface emojis without the `:` prefix and
    /// filter them by the user's query (matched as a substring
    /// against the emoji name + shortcode).
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        self.build_items(filter)
    }
}

impl EmojiProvider {
    fn build_items(&self, filter: &str) -> Vec<LauncherItem> {
        let mut scored: Vec<(f64, &emojis::Emoji)> = emojis::iter()
            .filter_map(|e| match_score(e, filter).map(|s| (s, e)))
            .collect();

        // Sort descending by score; emoji crate's iter() order
        // (CLDR canonical) breaks ties stably so common emoji
        // come first when scores tie.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let cap = if filter.is_empty() {
            BROWSE_LIMIT
        } else {
            SEARCH_LIMIT
        };

        scored
            .into_iter()
            .take(cap)
            .map(|(score, e)| {
                let payload = e.as_str().to_string();
                let name = e.name().to_string();
                let display_name = name.clone();
                LauncherItem {
                    id: format!("emoji:{}", e.as_str()),
                    name: format!("{}  {}", e.as_str(), name),
                    description: e
                        .shortcode()
                        .map(|s| format!(":{s}:"))
                        .unwrap_or_else(|| "Emoji".into()),
                    icon: "face-smile-symbolic".into(),
                    icon_is_path: false,
                    score,
                    provider_name: "Emoji".into(),
                    usage_key: Some(format!("emoji:{}", e.as_str())),
                    on_activate: Rc::new(move || {
                        copy_to_clipboard(&payload);
                        toast(format!("Copied {payload}"), display_name.clone());
                    }),
                }
            })
            .collect()
    }
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    match Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        Ok(mut child) => {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
        Err(err) => tracing::warn!(?err, "emoji wl-copy failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = EmojiProvider::new();
        assert!(p.search("hello").is_empty());
    }

    #[test]
    fn colon_smile_finds_smile() {
        let p = EmojiProvider::new();
        let items = p.search(":smile");
        // At least one entry's name should contain "smile" /
        // shortcode should match.
        assert!(items.iter().any(|i| i.name.to_lowercase().contains("smil")));
    }

    #[test]
    fn bare_colon_returns_browse_list() {
        let p = EmojiProvider::new();
        let items = p.search(":");
        assert!(items.len() <= BROWSE_LIMIT);
        assert!(!items.is_empty());
    }

    #[test]
    fn results_capped_by_search_limit() {
        let p = EmojiProvider::new();
        let items = p.search(":a");
        // "a" matches a lot — should hit the cap.
        assert!(items.len() <= SEARCH_LIMIT);
    }
}
