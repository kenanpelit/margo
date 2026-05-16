//! Custom-keyword web search.
//!
//! Type `<engine> <query>` and the browser opens with the
//! url-encoded query substituted into the engine's template.
//! Defaults cover the engines most users reach for; the
//! constructor takes a list so callers can extend with their own
//! (e.g. a project-internal wiki) without forking the provider.
//!
//! `handles_search` returns `true` so a `g rust` query produces
//! a "Google: rust" row alongside any apps that fuzzy-match `g`.
//! The engine prefix is checked at the start of the query so
//! it doesn't trigger inside a longer string like `git`.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

/// One configured search engine.
#[derive(Debug, Clone)]
pub struct Engine {
    /// Prefix the user types to invoke this engine. Must NOT
    /// contain whitespace (it's matched as the first token).
    pub keyword: String,
    /// Display label shown in the result row.
    pub label: String,
    /// URL template with `{q}` as the substitution point. The
    /// query is url-encoded before substitution.
    pub url_template: String,
    /// Icon name from the user's icon theme.
    pub icon: String,
}

impl Engine {
    fn url_for(&self, query: &str) -> String {
        self.url_template
            .replace("{q}", &url_encode(query))
    }
}

/// Minimal url-encoder. Encodes everything outside the
/// "unreserved" set of RFC 3986. We avoid pulling a full
/// percent-encoding crate because the alphabet is small and we
/// only ever encode user-typed search strings.
fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Default engines — the common 4 most users want without
/// having to type a config file. Override via
/// `WebsearchProvider::with_engines`.
pub fn default_engines() -> Vec<Engine> {
    vec![
        Engine {
            keyword: "g".into(),
            label: "Google".into(),
            url_template: "https://www.google.com/search?q={q}".into(),
            icon: "system-search-symbolic".into(),
        },
        Engine {
            keyword: "y".into(),
            label: "YouTube".into(),
            url_template: "https://www.youtube.com/results?search_query={q}".into(),
            icon: "applications-multimedia-symbolic".into(),
        },
        Engine {
            keyword: "ddg".into(),
            label: "DuckDuckGo".into(),
            url_template: "https://duckduckgo.com/?q={q}".into(),
            icon: "system-search-symbolic".into(),
        },
        Engine {
            keyword: "gh".into(),
            label: "GitHub".into(),
            url_template: "https://github.com/search?q={q}".into(),
            icon: "applications-development-symbolic".into(),
        },
        Engine {
            keyword: "aur".into(),
            label: "AUR".into(),
            url_template: "https://aur.archlinux.org/packages?K={q}".into(),
            icon: "package-x-generic-symbolic".into(),
        },
        Engine {
            keyword: "arch".into(),
            label: "Arch Wiki".into(),
            url_template: "https://wiki.archlinux.org/index.php?search={q}".into(),
            icon: "help-browser-symbolic".into(),
        },
        Engine {
            keyword: "wiki".into(),
            label: "Wikipedia".into(),
            url_template: "https://en.wikipedia.org/w/index.php?search={q}".into(),
            icon: "applications-science-symbolic".into(),
        },
    ]
}

pub struct WebsearchProvider {
    engines: Vec<Engine>,
}

impl WebsearchProvider {
    pub fn new() -> Self {
        Self::with_engines(default_engines())
    }

    pub fn with_engines(engines: Vec<Engine>) -> Self {
        Self { engines }
    }

    /// Read-only access to the configured engine list — useful
    /// for the Settings UI when it grows.
    pub fn engines(&self) -> &[Engine] {
        &self.engines
    }
}

impl Default for WebsearchProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for WebsearchProvider {
    fn name(&self) -> &str {
        "Web search"
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim();
        // We need at least "<keyword> something". A keyword
        // alone is too noisy.
        let (keyword, rest) = match trimmed.split_once(' ') {
            Some(pair) => pair,
            None => return Vec::new(),
        };
        let rest = rest.trim();
        if rest.is_empty() {
            return Vec::new();
        }

        self.engines
            .iter()
            .filter(|e| e.keyword == keyword)
            .map(|engine| {
                let url = engine.url_for(rest);
                let display = format!("{}: {}", engine.label, rest);
                let display_for_toast = display.clone();
                LauncherItem {
                    id: format!("websearch:{}", engine.keyword),
                    name: display,
                    description: format!("Open {} in browser", engine.label),
                    icon: engine.icon.clone(),
                    icon_is_path: false,
                    // Above typical app fuzzy scores so a known
                    // keyword always surfaces above accidental
                    // app matches like `git status` from the
                    // `g` prefix.
                    score: 220.0,
                    provider_name: "Web search".into(),
                    usage_key: Some(format!("websearch:{}", engine.keyword)),
                    on_activate: Rc::new(move || {
                        if let Err(err) = Command::new("xdg-open").arg(&url).spawn() {
                            tracing::warn!(?err, %url, "xdg-open failed");
                        } else {
                            toast("Web search", display_for_toast.clone());
                        }
                    }),
                }
            })
            .collect()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        // Surface every keyword in the bare `>` palette so the
        // user can discover the configured engines.
        self.engines
            .iter()
            .map(|engine| LauncherItem {
                id: format!("websearch:palette:{}", engine.keyword),
                name: format!("{} <query>", engine.keyword),
                description: format!("Search {}", engine.label),
                icon: engine.icon.clone(),
                icon_is_path: false,
                score: 0.0,
                provider_name: "Web search".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_alone_does_not_match() {
        let p = WebsearchProvider::new();
        assert!(p.search("g").is_empty());
        assert!(p.search("g ").is_empty());
    }

    #[test]
    fn google_keyword_produces_item() {
        let p = WebsearchProvider::new();
        let items = p.search("g rust async");
        assert!(items.iter().any(|i| i.name == "Google: rust async"));
    }

    #[test]
    fn unknown_keyword_returns_empty() {
        let p = WebsearchProvider::new();
        assert!(p.search("zz foo bar").is_empty());
    }

    #[test]
    fn url_encoding_handles_spaces_and_specials() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("c++"), "c%2B%2B");
        assert_eq!(url_encode("hello/world"), "hello%2Fworld");
    }

    #[test]
    fn commands_lists_every_engine() {
        let p = WebsearchProvider::new();
        assert_eq!(p.commands().len(), default_engines().len());
    }
}
