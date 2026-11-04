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
//!
//! ## Favourite sites (`web`)
//!
//! `web [filter]` lists the user's favourite websites (Settings →
//! Launcher → Favourite websites, `Name|url` lines) with their favicons,
//! most recently opened first, then never-opened ones in configured
//! order. Enter opens the site with `xdg-open`; Ctrl+Enter copies its
//! URL. Favicons are fetched once per domain from Google's s2 favicon
//! service on a worker thread and cached in `~/.cache/margo/favicons/`
//! (the setting can turn fetching off; rows then use the generic browser
//! icon).

use crate::history::CommandHistory;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use mshell_process::SpawnReaped;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

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
        self.url_template.replace("{q}", &url_encode(query))
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

// ── Favourite sites (`web [filter]`) ─────────────────────────────────────

/// Default favourites (Settings → Launcher → Favourite websites).
pub const DEFAULT_SITES: &[&str] = &[
    "GitHub|https://github.com",
    "YouTube|https://www.youtube.com",
    "Gmail|https://mail.google.com",
    "Arch Wiki|https://wiki.archlinux.org",
    "AUR|https://aur.archlinux.org",
];

/// One favourite site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub name: String,
    pub url: String,
    /// Lowercase host, used for the favicon cache key.
    pub domain: String,
}

/// Host part of an http(s) URL, lowercased, without port or userinfo.
pub fn url_domain(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.split(':').next()?.to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    valid.then_some(host)
}

/// Parse `Name|url` lines (a bare `url` or `domain.tld` is accepted too;
/// `https://` is assumed when no scheme is given). Invalid lines are
/// skipped; duplicates by URL keep the first.
pub fn parse_sites(lines: &[String]) -> Vec<Site> {
    let mut out: Vec<Site> = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, url) = match line.split_once('|') {
            Some((n, u)) => (n.trim(), u.trim()),
            None => ("", line),
        };
        let url = if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        let Some(domain) = url_domain(&url) else {
            continue;
        };
        if out.iter().any(|s| s.url == url) {
            continue;
        }
        let name = if name.is_empty() {
            domain.trim_start_matches("www.").to_string()
        } else {
            name.to_string()
        };
        out.push(Site { name, url, domain });
    }
    out
}

/// Most recently used first (by position in `mru`, newest first), then
/// never-used sites in configured order.
pub fn order_sites<'a>(sites: &'a [Site], mru: &[String]) -> Vec<&'a Site> {
    let mut ranked: Vec<(usize, usize, &Site)> = sites
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let used = mru.iter().position(|u| *u == s.url).unwrap_or(usize::MAX);
            (used, i, s)
        })
        .collect();
    ranked.sort_by_key(|(used, i, _)| (*used, *i));
    ranked.into_iter().map(|(_, _, s)| s).collect()
}

/// Favicon source. Google's s2 service returns a PNG for any domain
/// (sites' own /favicon.ico is often missing or an ICO GTK can't size).
pub fn favicon_url(domain: &str) -> String {
    format!(
        "https://www.google.com/s2/favicons?sz=64&domain={}",
        url_encode(domain)
    )
}

/// Largest favicon accepted (a real one is a few KB).
const FAVICON_MAX_BYTES: u64 = 256 * 1024;

fn download_favicon(domain: &str, dest: &std::path::Path) -> Result<(), String> {
    use std::io::Read as _;
    let resp = ureq::get(&favicon_url(domain))
        .timeout(std::time::Duration::from_secs(6))
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(FAVICON_MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    // PNG magic — never cache an HTML error page as an icon.
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("not a PNG".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("png.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

/// On-disk favicon cache; downloads missing icons off the GTK thread,
/// once per domain per session, and asks the launcher to repaint.
struct Favicons {
    dir: PathBuf,
    enabled: bool,
    attempted: Arc<Mutex<HashSet<String>>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl Favicons {
    fn new(dir: PathBuf, enabled: bool) -> Self {
        Self {
            dir,
            enabled,
            attempted: Arc::default(),
            notifier: Arc::default(),
        }
    }

    fn path(&self, domain: &str) -> PathBuf {
        self.dir.join(format!("{domain}.png"))
    }

    /// Cached icon path, kicking a background download when missing.
    fn get(&self, domain: &str) -> Option<PathBuf> {
        let path = self.path(domain);
        if path.is_file() {
            return Some(path);
        }
        if !self.enabled {
            return None;
        }
        let first = self
            .attempted
            .lock()
            .map(|mut set| set.insert(domain.to_string()))
            .unwrap_or(false);
        if first {
            let domain = domain.to_string();
            let notifier = Arc::clone(&self.notifier);
            std::thread::spawn(move || match download_favicon(&domain, &path) {
                Ok(()) => {
                    if let Ok(slot) = notifier.lock()
                        && let Some(notify) = slot.as_ref()
                    {
                        notify();
                    }
                }
                Err(err) => tracing::debug!(%domain, %err, "websearch: favicon fetch failed"),
            });
        }
        None
    }
}

fn favicon_dir() -> PathBuf {
    margo_fs::xdg::cache_home().join("margo").join("favicons")
}

fn site_mru_path() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("margo")
        .join("launcher_web_mru.json")
}

/// Live site configuration: the `Name|url` lines and whether favicons
/// may be fetched.
pub type SitesResolver = Box<dyn Fn() -> (Vec<String>, bool)>;

pub struct WebsearchProvider {
    engines: Vec<Engine>,
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
    sites: RefCell<Vec<Site>>,
    sites_resolver: Option<SitesResolver>,
    favicons: RefCell<Favicons>,
    /// Opened site URLs, newest first.
    site_mru: Rc<RefCell<CommandHistory>>,
}

impl WebsearchProvider {
    pub fn new() -> Self {
        Self::with_engines(default_engines())
    }

    pub fn with_engines(engines: Vec<Engine>) -> Self {
        let defaults: Vec<String> = DEFAULT_SITES.iter().map(|s| s.to_string()).collect();
        Self {
            engines,
            set_search: None,
            sites: RefCell::new(parse_sites(&defaults)),
            sites_resolver: None,
            favicons: RefCell::new(Favicons::new(favicon_dir(), true)),
            site_mru: Rc::new(RefCell::new(CommandHistory::load_from(site_mru_path()))),
        }
    }

    /// Follow the live config for the favourite-site list (re-read on
    /// every launcher open).
    pub fn with_sites_resolver(mut self, resolver: SitesResolver) -> Self {
        self.apply_sites(&resolver);
        self.sites_resolver = Some(resolver);
        self
    }

    fn apply_sites(&self, resolver: &SitesResolver) {
        let (lines, favicons) = resolver();
        *self.sites.borrow_mut() = parse_sites(&lines);
        self.favicons.borrow_mut().enabled = favicons;
    }

    #[cfg(test)]
    fn with_test_state(mut self, sites: &[&str], mru: PathBuf, favicon_dir: PathBuf) -> Self {
        let lines: Vec<String> = sites.iter().map(|s| s.to_string()).collect();
        self.sites = RefCell::new(parse_sites(&lines));
        self.favicons = RefCell::new(Favicons::new(favicon_dir, false));
        self.site_mru = Rc::new(RefCell::new(CommandHistory::load_from(mru)));
        self
    }

    /// `web [filter]` rows: substring filter on name + URL, MRU order.
    fn site_rows(&self, filter: &str) -> Vec<LauncherItem> {
        let sites = self.sites.borrow();
        if sites.is_empty() {
            return vec![LauncherItem {
                id: "websearch:site:none".into(),
                name: "No favourite websites".into(),
                description: "Add Name|url lines in Settings → Launcher".into(),
                icon: "dialog-information-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Web search".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }
        let needle = filter.trim().to_lowercase();
        let mru = self.site_mru.borrow().entries().to_vec();
        let favicons = self.favicons.borrow();
        order_sites(&sites, &mru)
            .into_iter()
            .filter(|s| {
                needle.is_empty()
                    || s.name.to_lowercase().contains(&needle)
                    || s.url.to_lowercase().contains(&needle)
            })
            .enumerate()
            .map(|(idx, site)| {
                let icon = favicons.get(&site.domain);
                let url = site.url.clone();
                let name = site.name.clone();
                let mru = Rc::clone(&self.site_mru);
                LauncherItem {
                    id: format!("websearch:site:{}", site.url),
                    name: site.name.clone(),
                    description: site.url.clone(),
                    icon_is_path: icon.is_some(),
                    icon: icon
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "web-browser-symbolic".into()),
                    // Wide steps keep recency ahead of the runtime's
                    // frequency boost (≤ ~33).
                    score: 3000.0 - 50.0 * idx as f64,
                    provider_name: "Web search".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        {
                            let mut mru = mru.borrow_mut();
                            mru.bump(&url);
                            mru.flush();
                        }
                        if let Err(err) = Command::new("xdg-open").arg(&url).spawn_reaped() {
                            tracing::warn!(?err, %url, "xdg-open failed");
                        } else {
                            toast("Web", format!("Opening {name}"));
                        }
                    }),
                }
            })
            .collect()
    }

    /// Wire the UI search entry setter used by the Search tab browse rows.
    /// Activating "Google" there should put `g ` into the entry so the user
    /// can continue typing the query, not launch an empty web search.
    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
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

    fn category(&self) -> &str {
        "Search"
    }

    fn handles_command(&self, query: &str) -> bool {
        site_filter(query).is_some()
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.favicons.borrow().notifier.lock() {
            *slot = Some(notifier);
        }
    }

    fn on_opened(&mut self) {
        if let Some(resolver) = &self.sites_resolver {
            self.apply_sites(resolver);
        }
    }

    fn bypasses_category_for_query(&self, query: &str) -> bool {
        let trimmed = query.trim();
        let Some((keyword, rest)) = trimmed.split_once(' ') else {
            return false;
        };
        !rest.trim().is_empty() && self.engines.iter().any(|e| e.keyword == keyword)
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        if let Some(filter) = site_filter(query) {
            return self.site_rows(filter);
        }
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
                        if let Err(err) = Command::new("xdg-open").arg(&url).spawn_reaped() {
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
        std::iter::once(LauncherItem {
            id: "websearch:palette:web".into(),
            name: "web [site]".into(),
            description: "Open a favourite website".into(),
            icon: "web-browser-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Web search".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        })
        .chain(self.engines.iter().map(|engine| LauncherItem {
            id: format!("websearch:palette:{}", engine.keyword),
            name: format!("{} <query>", engine.keyword),
            description: format!("Search {}", engine.label),
            icon: engine.icon.clone(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Web search".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }))
        .collect()
    }

    /// Search tab browse mode: show the actual web engines as first-class
    /// providers. The regular `search()` path intentionally requires
    /// `<keyword> <query>`, but an empty Search tab should still advertise
    /// "Google", "DuckDuckGo", etc. Activating a row seeds the entry with
    /// the matching prefix.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        let trimmed = filter.trim();
        if self.bypasses_category_for_query(trimmed) {
            return self.search(trimmed);
        }

        let needle = trimmed.to_ascii_lowercase();
        self.engines
            .iter()
            .filter(|engine| {
                needle.is_empty()
                    || engine.keyword.to_ascii_lowercase().contains(&needle)
                    || engine.label.to_ascii_lowercase().contains(&needle)
            })
            .enumerate()
            .map(|(idx, engine)| {
                let keyword = engine.keyword.clone();
                let setter = self.set_search.clone();
                LauncherItem {
                    id: format!("websearch:engine:{}", engine.keyword),
                    name: format!("{} search", engine.label),
                    description: format!("{} <query>", engine.keyword),
                    icon: engine.icon.clone(),
                    icon_is_path: false,
                    score: 210.0 - idx as f64,
                    provider_name: "Web search".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        if let Some(setter) = &setter {
                            setter(&format!("{keyword} "));
                        }
                    }),
                }
            })
            .collect()
    }

    /// Ctrl+Enter on a websearch row → copy the URL to the
    /// clipboard instead of opening the browser. Useful for
    /// "share this link" style flows or pasting into a different
    /// browser than xdg-open's default.
    fn alt_action(&self, item: &LauncherItem) -> Option<std::rc::Rc<dyn Fn() + 'static>> {
        // Recover the engine + query from the item. The display
        // name is `"{label}: {query}"`; we reconstruct the URL by
        // looking up the matching engine by keyword (suffix of
        // the item's id) and re-encoding the query.
        if let Some(url) = item.id.strip_prefix("websearch:site:") {
            if !url.contains("://") {
                return None;
            }
            let url = url.to_string();
            return Some(std::rc::Rc::new(move || copy_url(url.clone())));
        }
        let keyword = item.id.strip_prefix("websearch:")?;
        let engine = self.engines.iter().find(|e| e.keyword == keyword)?.clone();
        // The query is everything after `"{engine.label}: "`.
        let query = item.name.split_once(": ").map(|(_, q)| q.to_string())?;
        Some(std::rc::Rc::new(move || {
            copy_url(engine.url_for(&query));
        }))
    }
}

/// Text after the `web` prefix, if the query is a `web` invocation.
fn site_filter(query: &str) -> Option<&str> {
    let q = query.trim_start();
    if q == "web" {
        Some("")
    } else {
        q.strip_prefix("web ")
    }
}

/// Copy `url` with wl-copy and toast. Spawn, write, wait and toast all
/// off the GTK main thread — activation runs there, and even though
/// `wl-copy` forks and exits quickly, it's a real subprocess spawn +
/// blocking wait; `notify_rust`'s D-Bus call doesn't need the GTK main
/// thread either.
fn copy_url(url: String) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let mut child = match Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(?err, "wl-copy spawn failed for websearch alt-action");
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write as _;
            let _ = stdin.write_all(url.as_bytes());
        }
        let _ = child.wait();
        toast("URL copied", url);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

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
    fn browse_lists_engines_for_search_tab() {
        let p = WebsearchProvider::new();
        let items = p.browse("");
        assert!(items.iter().any(|i| i.name == "Google search"));
        assert!(
            items
                .iter()
                .any(|i| i.description == "ddg <query>" && i.name == "DuckDuckGo search")
        );
    }

    #[test]
    fn browse_engine_activation_seeds_prefix() {
        let captured = Rc::new(RefCell::new(String::new()));
        let setter_capture = captured.clone();
        let p = WebsearchProvider::new().with_search_setter(Rc::new(move |text| {
            *setter_capture.borrow_mut() = text.to_string();
        }));

        let items = p.browse("");
        let google = items.iter().find(|i| i.name == "Google search").unwrap();
        (google.on_activate)();

        assert_eq!(*captured.borrow(), "g ");
    }

    #[test]
    fn url_encoding_handles_spaces_and_specials() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("c++"), "c%2B%2B");
        assert_eq!(url_encode("hello/world"), "hello%2Fworld");
    }

    #[test]
    fn commands_lists_every_engine_and_sites() {
        let p = WebsearchProvider::new();
        assert_eq!(p.commands().len(), default_engines().len() + 1);
    }

    #[test]
    fn parses_site_lines() {
        let lines: Vec<String> = [
            "GitHub|https://github.com",
            "  wiki.archlinux.org  ",
            "Mail | https://User@Mail.Google.com:443/mail/u/0",
            "# comment",
            "bad|ftp://x.org",
            "dup|https://github.com",
            "nodot|https://localhost",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let sites = parse_sites(&lines);
        let got: Vec<(&str, &str, &str)> = sites
            .iter()
            .map(|s| (s.name.as_str(), s.url.as_str(), s.domain.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("GitHub", "https://github.com", "github.com"),
                (
                    "wiki.archlinux.org",
                    "https://wiki.archlinux.org",
                    "wiki.archlinux.org"
                ),
                (
                    "Mail",
                    "https://User@Mail.Google.com:443/mail/u/0",
                    "mail.google.com"
                ),
            ]
        );
        assert_eq!(
            parse_sites(
                &DEFAULT_SITES
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            )
            .len(),
            DEFAULT_SITES.len()
        );
    }

    #[test]
    fn sites_ordered_by_recent_use_then_config() {
        let lines: Vec<String> = ["a|https://a.com", "b|https://b.com", "c|https://c.com"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sites = parse_sites(&lines);
        let mru = vec![
            "https://c.com".to_string(),
            "https://gone.com".into(),
            "https://b.com".into(),
        ];
        let names: Vec<&str> = order_sites(&sites, &mru)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn favicon_url_encodes_domain() {
        assert_eq!(
            favicon_url("github.com"),
            "https://www.google.com/s2/favicons?sz=64&domain=github.com"
        );
    }

    #[test]
    fn web_prefix_lists_sites_with_cached_favicons_and_records_use() {
        let base = std::env::temp_dir().join(format!("mshell-web-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let icons = base.join("icons");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(icons.join("b.com.png"), b"\x89PNG").unwrap();
        let p = WebsearchProvider::new().with_test_state(
            &["A site|https://a.com", "B site|https://b.com"],
            base.join("mru.json"),
            icons.clone(),
        );
        assert!(p.handles_command("web"));
        assert!(p.handles_command("web b"));
        assert!(!p.handles_command("webcam"));
        let rows = p.search("web");
        assert_eq!(rows[0].name, "A site");
        assert_eq!(rows[0].icon, "web-browser-symbolic");
        assert!(!rows[0].icon_is_path);
        assert!(rows[1].icon_is_path);
        assert_eq!(rows[1].icon, icons.join("b.com.png").to_string_lossy());
        // Opening B moves it first (the activation closure would also
        // spawn xdg-open, so record the use directly).
        p.site_mru.borrow_mut().bump("https://b.com");
        let names: Vec<String> = p.search("web").into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["B site", "A site"]);
        assert_eq!(p.search("web a.co")[0].name, "A site");
        assert!(p.alt_action(&p.search("web")[0]).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
