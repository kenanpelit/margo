//! Apps provider — wraps `gio::DesktopAppInfo` so the launcher
//! runtime can search desktop entries through the same trait every
//! other provider uses.
//!
//! Lives in `mshell-frame` rather than `mshell-launcher` because
//! `DesktopAppInfo` is a GTK type that doesn't cross thread / crate
//! boundaries cleanly — the type itself is `'static` but its
//! associated icon lookup is meaningfully easier here, next to the
//! existing `set_icon` helper.

use mshell_cache::hidden_apps::is_hidden;
use mshell_launcher::{LauncherItem, Provider};
use mshell_utils::launch::launch_detached;
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use relm4::gtk::gio::{self, prelude::*};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

/// Snapshot of a single desktop entry kept by the provider. We
/// cache the lowercased name + the original `DesktopAppInfo` so
/// fuzzy match runs without re-allocating per keystroke.
struct AppEntry {
    info: gio::DesktopAppInfo,
    /// Pre-computed lowercase name for case-insensitive fallback
    /// substring matching (used when fuzzy returns nothing).
    name: String,
    /// Stable id used for both the LauncherItem id and the
    /// frecency / hidden-apps lookup key.
    id: String,
    hidden: bool,
}

pub struct AppsProvider {
    /// All desktop entries we know about. Refreshed on
    /// `refresh()` and `on_opened()`.
    entries: RefCell<Vec<AppEntry>>,
    /// When true, hidden entries are surfaced (the user toggled
    /// the eye icon in the launcher). When false they're filtered
    /// out before fuzzy match runs.
    show_hidden: Arc<AtomicBool>,
    /// Reused across queries — nucleo amortises its internal
    /// buffers across calls.
    matcher: RefCell<Matcher>,
}

impl AppsProvider {
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
            show_hidden: Arc::new(AtomicBool::new(false)),
            matcher: RefCell::new(Matcher::new(Config::DEFAULT)),
        }
    }

    /// Toggle whether hidden entries appear in results.
    pub fn set_show_hidden(&self, show: bool) {
        self.show_hidden
            .store(show, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Re-scan installed desktop entries. Cheap — gio caches the
    /// list, so this is essentially a fast filter pass.
    pub fn refresh(&self) {
        let mut new_entries: Vec<AppEntry> = gio::AppInfo::all()
            .into_iter()
            .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok())
            .filter(|info| !info.is_hidden() && !info.is_nodisplay())
            .map(|info| {
                let id = info
                    .id()
                    .map(|g| g.to_string())
                    .or_else(|| info.filename().map(|p| p.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| info.name().to_string());
                let hidden = is_hidden(&id);
                let name = info.name().to_string().to_lowercase();
                AppEntry {
                    info,
                    name,
                    id,
                    hidden,
                }
            })
            .collect();

        // Alphabetical by display name keeps the empty-query
        // browse list predictable.
        new_entries.sort_by(|a, b| a.name.cmp(&b.name));
        *self.entries.borrow_mut() = new_entries;
    }

    fn make_item(&self, entry: &AppEntry, score: f64) -> LauncherItem {
        let app_clone = entry.info.clone();
        LauncherItem {
            id: format!("apps:{}", entry.id),
            name: entry.info.name().to_string(),
            description: entry
                .info
                .generic_name()
                .map(|s| s.to_string())
                .or_else(|| entry.info.description().map(|s| s.to_string()))
                .unwrap_or_default(),
            // The widget renders apps via its own icon-themed
            // image (matches mshell's app_icon_theme + matugen
            // filters); we still set a fallback name here so a
            // generic row can render an Adwaita icon if it has to.
            icon: "application-x-executable-symbolic".into(),
            icon_is_path: false,
            score,
            provider_name: "Apps".into(),
            usage_key: Some(format!("apps:{}", entry.id)),
            // Just launch — closing the launcher is handled
            // centrally in `app_launcher.rs::activate_id` after
            // the closure returns, so every provider gets the
            // same dismiss-on-activate behaviour without each
            // one having to wire its own close callback.
            on_activate: Rc::new(move || launch_detached(&app_clone)),
        }
    }
}

impl Default for AppsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for AppsProvider {
    fn name(&self) -> &str {
        "Apps"
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let entries = self.entries.borrow();
        let show_hidden = self.show_hidden();
        let trimmed = query.trim();

        // Empty query → browse list (everything alphabetical,
        // filtered by show-hidden).
        if trimmed.is_empty() {
            return entries
                .iter()
                .filter(|e| show_hidden || !e.hidden)
                .map(|e| self.make_item(e, 0.0))
                .collect();
        }

        let mut matcher = self.matcher.borrow_mut();
        let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);

        let mut results: Vec<LauncherItem> = entries
            .iter()
            .filter(|e| show_hidden || !e.hidden)
            .filter_map(|entry| {
                let name = entry.info.name();
                let mut buf = Vec::new();
                let hay = Utf32Str::new(name.as_str(), &mut buf);
                pattern
                    .score(hay, &mut matcher)
                    .map(|raw| self.make_item(entry, raw as f64))
            })
            .collect();

        // Within a single keystroke the runtime sorts globally;
        // we still sort here so the Apps slice is stable even
        // when the runtime is bypassed (tests, debug pages).
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    fn on_opened(&mut self) {
        // Refresh on every open — desktop entries can change
        // while the launcher is closed (the user installs a
        // package, edits a .desktop file). Cheap because the gio
        // monitor backs the list.
        self.refresh();
    }
}
