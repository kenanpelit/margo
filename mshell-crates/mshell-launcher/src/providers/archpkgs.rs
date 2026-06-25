//! `p <query>` — search official Arch + AUR repos.
//!
//! Subprocess-based search via `pacman -Ss` (always) and
//! `yay -Ss` / `paru -Ss` (when an AUR helper is on PATH). Each
//! hit becomes one row — activating it spawns the user's
//! terminal with `<helper> -S <pkg>` so they can confirm the
//! install interactively. We don't wrap install in a yes/no
//! popover because pacman's confirmation flow already handles
//! conflicts/deps/replacements that a generic popover can't
//! represent.

use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the background search worker waits before actually
/// spawning `pacman -Ss`. A keystroke that supersedes the term within
/// this window aborts the worker first, so a fast typist's
/// intermediate terms (`fir`, `fire`, `firef`…) don't each spawn a
/// subprocess — only the term they pause on does. This is the
/// debounce, run off the main thread so it never delays the launcher's
/// instant in-memory providers.
const SEARCH_SETTLE: Duration = Duration::from_millis(180);

/// Query-keyed snapshot of the last package search, shared with the
/// background worker. `pacman -Ss` is a blocking subprocess (1-3 s on a
/// slow mirror) and used to run inside [`Provider::search`] on the GTK
/// main thread, freezing the whole UI on every keystroke. Now `search`
/// serves from here instantly and the actual lookup runs on a worker
/// thread; when it lands it fills this cache and fires the
/// [`RefreshNotifier`] so the launcher re-queries and the rows appear.
#[derive(Default)]
struct PkgCache {
    /// The term the cache currently holds / is fetching results for.
    term: String,
    /// Parsed hits for `term` (valid only once `ready`).
    hits: Vec<Hit>,
    /// A worker is settling or running `pacman -Ss` for `term`.
    in_flight: bool,
    /// `term`'s search completed — distinguishes "still fetching" from
    /// "ran and genuinely matched nothing".
    ready: bool,
    /// Bumped on every new term of interest. A worker stores its result
    /// only if the generation it captured still matches, so a
    /// superseded (typed-past) search discards its result instead of
    /// clobbering the latest term's rows.
    generation: u64,
}

pub struct ArchLinuxPkgsProvider {
    /// AUR helper found on PATH (`yay` / `paru` / `pamac`). When
    /// `None`, only `pacman -Ss` runs and install spawns the
    /// terminal with plain `sudo pacman -S`.
    helper: Option<String>,
    /// Terminal binary used to spawn install confirmations. We
    /// pick `$TERMINAL` env when set, else `kitty`, `alacritty`,
    /// `foot` in that order. Falls back to `kitty` and lets the
    /// shell discover failures.
    terminal: String,
    /// Optional UI hook used by category browse rows. Activating
    /// the "Arch / AUR package search" row should seed `p ` into
    /// the launcher entry so the user can continue typing; actual
    /// package lookups still require an explicit prefix to avoid
    /// running pacman on every generic Search-tab keystroke.
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
    /// Shared with the background search worker (see [`PkgCache`]).
    cache: Arc<Mutex<PkgCache>>,
    /// Fired (from the worker) when a search lands, so the launcher
    /// re-runs the current query and shows the new rows.
    notifier: Option<RefreshNotifier>,
}

impl ArchLinuxPkgsProvider {
    pub fn new() -> Self {
        let helper = ["yay", "paru", "pamac"]
            .iter()
            .find(|h| which_exists(h))
            .map(|h| h.to_string());
        let terminal = std::env::var("TERMINAL")
            .ok()
            .or_else(|| {
                ["kitty", "alacritty", "foot", "wezterm"]
                    .iter()
                    .find(|t| which_exists(t))
                    .map(|t| t.to_string())
            })
            .unwrap_or_else(|| "kitty".into());
        Self {
            helper,
            terminal,
            set_search: None,
            cache: Arc::new(Mutex::new(PkgCache::default())),
            notifier: None,
        }
    }

    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
    }

    /// Hits for `term` from the background cache, kicking an off-thread
    /// `pacman -Ss` when the cache doesn't already hold them.
    ///
    /// * `Some(hits)` — results are ready (possibly empty: a genuine
    ///   no-match), served instantly from the cache.
    /// * `None` — a worker is (settling then) fetching this term; the
    ///   caller shows a transient placeholder and the [`RefreshNotifier`]
    ///   re-queries us once the rows land.
    ///
    /// Never blocks: the subprocess always runs on a worker thread.
    fn hits_for(&self, term: &str) -> Option<Vec<Hit>> {
        let generation = {
            // A poisoned lock would mean a worker panicked mid-update;
            // degrade to "no results" rather than take the shell down.
            let Ok(mut cache) = self.cache.lock() else {
                return None;
            };
            if cache.term == term {
                if cache.ready {
                    return Some(cache.hits.clone());
                }
                if cache.in_flight {
                    return None; // already fetching this exact term
                }
            }
            // New term of interest — (re)start a fetch.
            cache.term = term.to_string();
            cache.hits.clear();
            cache.ready = false;
            cache.in_flight = true;
            cache.generation = cache.generation.wrapping_add(1);
            cache.generation
        };

        let cache = Arc::clone(&self.cache);
        let notifier = self.notifier.clone();
        let helper = self.helper.clone();
        let term = term.to_string();
        std::thread::spawn(move || {
            // Debounce: a newer keystroke bumps the generation; if ours
            // is stale after the settle window, abort before running
            // pacman so intermediate terms don't each spawn a search.
            std::thread::sleep(SEARCH_SETTLE);
            match cache.lock() {
                Ok(c) if c.generation == generation => {}
                _ => return, // superseded (or poisoned) — abort
            }

            let hits = search_packages(&term, helper.as_deref());

            let stored = match cache.lock() {
                Ok(mut c) if c.generation == generation => {
                    c.hits = hits;
                    c.ready = true;
                    c.in_flight = false;
                    true
                }
                _ => false, // typed past this term (or poisoned) — discard
            };
            if stored && let Some(notify) = &notifier {
                notify();
            }
        });

        None
    }

    /// Map parsed package hits into activatable launcher rows. Pure
    /// (main-thread) work — the slow `pacman` call already happened on
    /// the worker thread in [`Self::hits_for`].
    fn items_from_hits(&self, hits: Vec<Hit>) -> Vec<LauncherItem> {
        let install_cmd = self.helper.clone().unwrap_or_else(|| "pacman".into());
        let terminal = self.terminal.clone();

        hits.into_iter()
            .take(80)
            .enumerate()
            .map(|(idx, hit)| {
                let pkg = hit.full.clone();
                let pkg_short = pkg.split('/').next_back().unwrap_or(&pkg).to_string();
                let term_clone = terminal.clone();
                let install_cmd_clone = install_cmd.clone();
                let pkg_label = pkg.clone();
                let label = if hit.installed {
                    format!("{}  {}  [installed]", hit.full, hit.version)
                } else {
                    format!("{}  {}", hit.full, hit.version)
                };
                let description = if hit.description.is_empty() {
                    "Press Enter to install in a terminal".into()
                } else {
                    hit.description.clone()
                };
                LauncherItem {
                    id: format!("archpkgs:{}", hit.full),
                    name: label,
                    description,
                    icon: if hit.installed {
                        "emblem-default-symbolic".into()
                    } else {
                        "package-x-generic-symbolic".into()
                    },
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "Arch packages".into(),
                    usage_key: Some(format!("archpkgs:{}", hit.full)),
                    on_activate: Rc::new(move || {
                        spawn_install(&term_clone, &install_cmd_clone, &pkg_short);
                        toast("Installing", pkg_label.clone());
                    }),
                }
            })
            .collect()
    }
}

/// Transient, non-actionable row shown while a background package
/// search is in flight, so the Search tab reads as "working" rather
/// than empty during the ~1 s lookup. Carries no `usage_key` (never
/// recorded) and a no-op activation.
fn searching_row(term: &str) -> LauncherItem {
    LauncherItem {
        id: "archpkgs:searching".into(),
        name: format!("Searching Arch + AUR for “{term}”…"),
        description: "Results will appear in a moment".into(),
        icon: "system-search-symbolic".into(),
        icon_is_path: false,
        score: 200.0,
        provider_name: "Arch packages".into(),
        usage_key: None,
        on_activate: Rc::new(|| {}),
    }
}

impl Default for ArchLinuxPkgsProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn which_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// One package row.
#[derive(Clone)]
struct Hit {
    /// `repo/name` (e.g. `extra/firefox` or `aur/spotify`).
    full: String,
    /// Version string (right after the name).
    version: String,
    /// Description line (the indented second line of pacman/yay
    /// output).
    description: String,
    /// `[installed]` marker present in the output.
    installed: bool,
}

/// Parse `pacman -Ss` / `yay -Ss` style output. Each match is
/// two lines: a header (`repo/name ver [installed]`) and an
/// indented description.
fn parse_search(stdout: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut iter = stdout.lines().peekable();
    while let Some(header) = iter.next() {
        let header = header.trim_end();
        if header.is_empty() {
            continue;
        }
        // Description lines start with whitespace; if the next
        // header arrives before a description, this entry has
        // none.
        let description = match iter.peek() {
            Some(next) if next.starts_with(' ') || next.starts_with('\t') => {
                iter.next().unwrap().trim().to_string()
            }
            _ => String::new(),
        };
        let installed = header.contains("[installed");
        // Header form: "repo/name version <tags>"
        let mut tokens = header.split_whitespace();
        let full = tokens.next().unwrap_or("").to_string();
        let version = tokens.next().unwrap_or("").to_string();
        if !full.is_empty() {
            hits.push(Hit {
                full,
                version,
                description,
                installed,
            });
        }
    }
    hits
}

fn search_packages(query: &str, helper: Option<&str>) -> Vec<Hit> {
    let mut hits = Vec::new();
    // Prefer the AUR helper because it also searches official
    // repos in one shot; fall back to pacman when no helper is
    // installed.
    let bin = helper.unwrap_or("pacman");
    if let Ok(out) = Command::new(bin).args(["-Ss", query]).output()
        && out.status.success()
    {
        hits.extend(parse_search(&String::from_utf8_lossy(&out.stdout)));
    }
    hits
}

impl Provider for ArchLinuxPkgsProvider {
    fn name(&self) -> &str {
        "Arch packages"
    }

    fn category(&self) -> &str {
        "Search"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "p"
            || q.starts_with("p ")
            || q == "pacman"
            || q.starts_with("pacman ")
            || q == "aur"
            || q.starts_with("aur ")
    }

    fn bypasses_category_for_query(&self, query: &str) -> bool {
        let q = query.trim_start();
        (q.starts_with("p ") || q.starts_with("pacman ") || q.starts_with("aur "))
            && q.split_once(' ')
                .map(|(_, rest)| rest.trim().len() >= 3)
                .unwrap_or(false)
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "archpkgs:palette".into(),
            name: "p <query>".into(),
            description: format!(
                "Search Arch + AUR ({}install via terminal)",
                if self.helper.is_some() {
                    "AUR helper found — "
                } else {
                    ""
                }
            ),
            icon: "package-x-generic-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Arch packages".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        // Strip prefix to get the actual search term.
        let term = if let Some(rest) = q.strip_prefix("pacman ") {
            rest
        } else if let Some(rest) = q.strip_prefix("aur ") {
            rest
        } else if let Some(rest) = q.strip_prefix("p ") {
            rest
        } else {
            return Vec::new();
        };
        let term = term.trim();
        // Searches with <3 chars produce too much noise from
        // pacman — wait for a real query.
        if term.len() < 3 {
            return Vec::new();
        }

        // Serve from the background cache; never run `pacman` on this
        // (main) thread. `None` = a worker is fetching this term — show
        // a transient "searching" row until the notifier re-queries us.
        match self.hits_for(term) {
            Some(hits) => self.items_from_hits(hits),
            None => vec![searching_row(term)],
        }
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.notifier = Some(notifier);
    }

    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        let trimmed = filter.trim();
        if self.bypasses_category_for_query(trimmed) {
            return self.search(trimmed);
        }

        let needle = trimmed.to_ascii_lowercase();
        let matches = needle.is_empty()
            || "p".contains(&needle)
            || "arch".contains(&needle)
            || "aur".contains(&needle)
            || "package".contains(&needle)
            || "packages".contains(&needle)
            || "pacman".contains(&needle);
        if !matches {
            return Vec::new();
        }

        let setter = self.set_search.clone();
        vec![LauncherItem {
            id: "archpkgs:engine".into(),
            name: "Arch / AUR package search".into(),
            description: format!(
                "p <query>{}",
                if self.helper.is_some() {
                    " — AUR helper found"
                } else {
                    ""
                }
            ),
            icon: "package-x-generic-symbolic".into(),
            icon_is_path: false,
            score: 205.0,
            provider_name: "Arch packages".into(),
            usage_key: None,
            on_activate: Rc::new(move || {
                if let Some(setter) = &setter {
                    setter("p ");
                }
            }),
        }]
    }
}

/// Spawn `<terminal> -e <helper> -S <pkg>` so the user sees the
/// confirmation prompt. When using bare pacman we wrap with
/// `sudo`; AUR helpers handle their own privilege escalation.
fn spawn_install(terminal: &str, helper: &str, pkg: &str) {
    let inner: Vec<String> = if helper == "pacman" {
        vec!["sudo".into(), "pacman".into(), "-S".into(), pkg.into()]
    } else {
        vec![helper.into(), "-S".into(), pkg.into()]
    };

    // `kitty` / `alacritty` / `foot` all accept `<term> -e <cmd>
    // <args...>`. wezterm wants `start --`, so handle it
    // specially.
    let result = if terminal == "wezterm" {
        let mut cmd = Command::new(terminal);
        cmd.arg("start").arg("--");
        for arg in &inner {
            cmd.arg(arg);
        }
        cmd.spawn()
    } else {
        let mut cmd = Command::new(terminal);
        cmd.arg("-e");
        for arg in &inner {
            cmd.arg(arg);
        }
        cmd.spawn()
    };
    if let Err(err) = result {
        tracing::warn!(?err, terminal, pkg, "archpkgs install spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn does_not_handle_regular_search() {
        let p = ArchLinuxPkgsProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn p_prefix_requires_min_3_chars() {
        let p = ArchLinuxPkgsProvider::new();
        assert!(p.search("p ab").is_empty());
        assert!(p.search("p").is_empty());
    }

    #[test]
    fn browse_lists_package_search_entry_for_search_tab() {
        let p = ArchLinuxPkgsProvider::new();
        let items = p.browse("");
        assert!(
            items
                .iter()
                .any(|i| i.name == "Arch / AUR package search" && i.description.starts_with("p "))
        );
    }

    #[test]
    fn browse_package_entry_activation_seeds_prefix() {
        let captured = Rc::new(RefCell::new(String::new()));
        let setter_capture = captured.clone();
        let p = ArchLinuxPkgsProvider::new().with_search_setter(Rc::new(move |text| {
            *setter_capture.borrow_mut() = text.to_string();
        }));

        let items = p.browse("");
        let arch = items
            .iter()
            .find(|i| i.name == "Arch / AUR package search")
            .unwrap();
        (arch.on_activate)();

        assert_eq!(*captured.borrow(), "p ");
    }

    #[test]
    fn parse_search_handles_pacman_output() {
        let sample = "extra/firefox 145.0.1-1\n    Standalone web browser from mozilla.org\nextra/firefox-i18n-tr 145.0.1-1\n    Turkish language pack for Firefox\n";
        let hits = parse_search(sample);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].full, "extra/firefox");
        assert_eq!(hits[0].version, "145.0.1-1");
        assert!(!hits[0].installed);
        assert!(hits[0].description.contains("Standalone"));
    }

    #[test]
    fn parse_search_detects_installed_marker() {
        let sample = "extra/firefox 145.0.1-1 [installed]\n    Browser\n";
        let hits = parse_search(sample);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].installed);
    }
}
