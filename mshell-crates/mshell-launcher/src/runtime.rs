//! The runtime that glues providers, scoring, frecency, pins,
//! exact-search toggling, and the Tab/category cycle together.
//!
//! The UI layer constructs a `LauncherRuntime`, registers
//! providers in priority order, and calls `query()` on every
//! keystroke. The runtime is single-threaded — it lives on the
//! GTK main loop and does not need to be `Send`.
//!
//! ## Result composition
//!
//! Given query `q` the runtime does one of three things:
//!
//! 1. **`q.is_empty()`** — browse mode. Every provider that
//!    `handles_search()` is asked for its empty-query results
//!    (Apps returns the alphabetical/pinned list, Calculator
//!    returns nothing, etc.). If [`LauncherRuntime::active_category`]
//!    is set, only providers in that category contribute.
//!
//! 2. **`q == ">"`** — command palette. The runtime concatenates
//!    every provider's `commands()` and returns them as-is (no
//!    scoring).
//!
//! 3. **`q.starts_with('>')`** — command mode. The runtime asks
//!    each provider's `handles_command()` until one returns true,
//!    then delegates result generation to that provider alone.
//!
//! 4. **otherwise** — regular search. Every searching provider
//!    contributes results, the runtime applies usage boost, sorts
//!    by descending score.
//!
//! ## Decorations the runtime stamps
//!
//! After scoring/sorting the runtime wraps each [`LauncherItem`] in
//! a [`DisplayItem`] carrying:
//!
//! * `pinned` — true if the item's `usage_key` is in [`PinStore`].
//!   Pinned items bubble to the top of empty-browse regardless of
//!   their raw score / frecency.
//! * `quick_key` — `"1"`..`"9"` for the first nine rows, empty
//!   string thereafter. The UI renders this next to the row and
//!   binds Alt+N to "activate the row whose quick_key is N".

use crate::{
    frecency::FrecencyStore,
    hidden::HiddenStore,
    item::{DisplayItem, LauncherItem},
    pin::PinStore,
    provider::Provider,
    scoring::{recency_boost, usage_boost},
};

/// Bonus added to a pinned item's score in browse mode so pinned
/// rows always rank above any frecency-only entry. Far larger than
/// the `usage_boost(count)` ceiling (~`5 * log2(1+u16::MAX) ≈ 80`).
const PIN_BONUS: f64 = 10_000.0;

/// A coarse provider grouping used by the **Tab/Shift+Tab**
/// category cycle and the visual tab strip rendered above the
/// result list.
///
/// Built once during [`LauncherRuntime::categories()`] from the
/// distinct display groups each provider maps to, in registration
/// order. `"All"` is implicit and always available even if no
/// provider declares it explicitly — selecting it lets every provider
/// contribute, which is the default state when the launcher opens.
#[derive(Debug, Clone)]
pub struct ProviderCategory {
    /// User-facing label drawn on the tab pill.
    pub label: String,
}

fn display_category_for(provider_category: &str) -> &str {
    match provider_category {
        // Keep provider ownership precise internally, but collapse the UI
        // strip: compositor actions, shell/system actions, command runners,
        // and connections are all "things to do", not separate top-level
        // launcher modes.
        "Compositor" | "Run" | "System" | "Connect" => "Actions",
        other => other,
    }
}

fn provider_matches_display_category(provider: &dyn Provider, display_category: &str) -> bool {
    display_category_for(provider.category()) == display_category
}

/// Owns the provider list + the frecency store + the pin set +
/// the user-toggled search-mode flags. The UI layer constructs one
/// of these in `init`, registers providers, then calls `query()`
/// on every keystroke and `record_usage` / `toggle_pin` /
/// `cycle_category` from key handlers.
pub struct LauncherRuntime {
    providers: Vec<Box<dyn Provider>>,
    frecency: FrecencyStore,
    pins: PinStore,
    hidden: HiddenStore,
    /// When `Some`, only providers whose display category matches this
    /// label contribute to empty-browse + regular-search results.
    /// `None` ("All" tab) lets every provider participate. Cycled
    /// by `cycle_category` / `cycle_category_back` (Tab /
    /// Shift+Tab in the UI).
    active_category: Option<String>,
    /// Substring-only matching when true; fuzzy (provider-defined)
    /// when false. Toggled by Ctrl+E in the UI. Providers that
    /// honour this read it from `is_exact_search()`. Default false.
    exact_search: bool,
    /// Snapshot of the user's last query before they closed the
    /// launcher — restored by Ctrl+R via [`Self::last_query`].
    last_query: String,
}

impl LauncherRuntime {
    /// Construct a runtime with default frecency + pin stores
    /// (both loaded from disk).
    pub fn new(frecency: FrecencyStore) -> Self {
        Self {
            providers: Vec::new(),
            frecency,
            pins: PinStore::load(),
            hidden: HiddenStore::load(),
            active_category: None,
            exact_search: false,
            last_query: String::new(),
        }
    }

    /// Construct with caller-supplied frecency *and* pin stores
    /// (hidden store still loads from disk). Used by integration
    /// tests so they can hand the runtime ephemeral state files.
    pub fn with_stores(frecency: FrecencyStore, pins: PinStore) -> Self {
        Self::with_all_stores(frecency, pins, HiddenStore::load())
    }

    /// Construct with caller-supplied frecency / pin / hidden
    /// stores. Tests pass ephemeral files for full isolation.
    pub fn with_all_stores(frecency: FrecencyStore, pins: PinStore, hidden: HiddenStore) -> Self {
        Self {
            providers: Vec::new(),
            frecency,
            pins,
            hidden,
            active_category: None,
            exact_search: false,
            last_query: String::new(),
        }
    }

    /// Add a provider. Order matters for command-mode dispatch
    /// (first matching provider wins), for empty-query browse
    /// (results are concatenated in registration order), and for
    /// the category tab order (first provider in each category
    /// determines that category's position in the strip).
    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    /// Hand every registered provider a notifier to call when one of its
    /// off-thread data refreshes completes (so the UI can re-run the
    /// current query). Call once, after all providers are registered.
    pub fn set_refresh_notifier(&mut self, notifier: crate::provider::RefreshNotifier) {
        for provider in &mut self.providers {
            provider.set_refresh_notifier(notifier.clone());
        }
    }

    /// Read-only access to the frecency store.
    pub fn frecency(&self) -> &FrecencyStore {
        &self.frecency
    }

    /// Bump the usage counter for an item. Call this after the
    /// item is dispatched.
    pub fn record_usage(&mut self, key: &str) {
        self.frecency.bump(key);
    }

    /// Toggle pin state for the item's usage_key. Returns the new
    /// state (`true` = now pinned). UI binds this to Ctrl+P.
    pub fn toggle_pin(&mut self, key: &str) -> bool {
        self.pins.toggle(key)
    }

    /// True if the given key is currently pinned. UI uses this to
    /// render the ★ glyph without round-tripping through the
    /// runtime for every result row.
    pub fn is_pinned(&self, key: &str) -> bool {
        self.pins.is_pinned(key)
    }

    /// Ask the matching provider whether the given item can be
    /// deleted (Delete key in the UI).
    pub fn can_delete(&self, item: &LauncherItem) -> bool {
        self.providers
            .iter()
            .find(|p| p.name() == item.provider_name)
            .map(|p| p.can_delete(item))
            .unwrap_or(false)
    }

    /// Preview for `item`, asked of the provider that produced it.
    /// Drives the launcher's side preview pane.
    pub fn preview_for(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        self.providers
            .iter()
            .find(|p| p.name() == item.provider_name)
            .and_then(|p| p.preview(item))
    }

    /// Run the matching provider's `delete_item` on the given item
    /// *and* drop the corresponding frecency / pin entries so the
    /// next browse-mode pass treats the item as if the user had
    /// never used it. No-op when no provider claims ownership or
    /// `can_delete` returns false.
    pub fn delete_item(&mut self, item: &LauncherItem) {
        let name = item.provider_name.clone();
        let owns = self
            .providers
            .iter()
            .any(|p| p.name() == name && p.can_delete(item));
        if !owns {
            return;
        }
        if let Some(p) = self.providers.iter_mut().find(|p| p.name() == name) {
            p.delete_item(item);
        }
        // Clear the user-learned ranking so the item drops back to
        // its provider's base score. Without this, Delete would
        // remove the provider-owned entry (history line, clipboard
        // row, …) but a stale frecency bump would keep promoting
        // it the next time it was re-synthesised by the provider.
        if let Some(key) = &item.usage_key {
            self.frecency.forget(key);
            self.pins.unpin(key);
            self.hidden.unhide(key);
        }
    }

    /// Return the alternative action closure for the item, if the
    /// owning provider defined one. UI binds this to Ctrl+Enter.
    pub fn alt_action(&self, item: &LauncherItem) -> Option<std::rc::Rc<dyn Fn() + 'static>> {
        self.providers
            .iter()
            .find(|p| p.name() == item.provider_name)
            .and_then(|p| p.alt_action(item))
    }

    /// Distinct provider categories in registration order, with
    /// `"All"` prepended (selecting "All" disables the per-category
    /// filter). UI renders one tab pill per entry.
    pub fn categories(&self) -> Vec<ProviderCategory> {
        let mut seen: Vec<String> = vec!["All".into()];
        for p in &self.providers {
            let cat = display_category_for(p.category()).to_string();
            if !seen.iter().any(|c| c == &cat) {
                seen.push(cat);
            }
        }
        seen.into_iter()
            .map(|label| ProviderCategory { label })
            .collect()
    }

    /// Currently-active category label (`"All"` when unfiltered).
    pub fn active_category_label(&self) -> String {
        self.active_category.clone().unwrap_or_else(|| "All".into())
    }

    /// Jump to a category by exact label match. Pass `"All"` to
    /// clear the filter. Returns the new active label (so the UI
    /// can highlight the right tab).
    pub fn select_category(&mut self, label: &str) -> String {
        self.active_category = if label == "All" {
            None
        } else {
            Some(label.to_string())
        };
        self.active_category_label()
    }

    /// Advance the active category by `direction` (`+1` = next,
    /// `-1` = previous). Wraps. Used by Tab / Shift+Tab.
    pub fn cycle_category(&mut self, direction: i32) -> String {
        let cats = self.categories();
        if cats.is_empty() {
            return self.active_category_label();
        }
        let current_label = self.active_category_label();
        let idx = cats
            .iter()
            .position(|c| c.label == current_label)
            .unwrap_or(0);
        let len = cats.len() as i32;
        let next = ((idx as i32 + direction).rem_euclid(len)) as usize;
        self.select_category(&cats[next].label)
    }

    /// True when the user has toggled Ctrl+E (exact / substring
    /// matching mode). Providers that respect it read this and
    /// short-circuit their fuzzy matcher.
    pub fn is_exact_search(&self) -> bool {
        self.exact_search
    }

    /// Toggle exact-search mode. Returns the new state.
    pub fn toggle_exact_search(&mut self) -> bool {
        self.exact_search = !self.exact_search;
        self.exact_search
    }

    /// Snapshot the current query so Ctrl+R can restore it on the
    /// next launcher open.
    pub fn remember_query(&mut self, query: &str) {
        if !query.is_empty() {
            self.last_query = query.to_string();
        }
    }

    /// Last remembered query — empty string if nothing has been
    /// typed yet this session. UI calls this on Ctrl+R.
    pub fn last_query(&self) -> &str {
        &self.last_query
    }

    /// Persist any pending frecency bumps + pin changes to disk.
    pub fn flush(&mut self) {
        self.frecency.flush();
        self.pins.flush();
    }

    /// Forward the panel-opened lifecycle hook to every provider.
    pub fn on_opened(&mut self) {
        for p in &mut self.providers {
            p.on_opened();
        }
    }

    /// Forward the panel-closed lifecycle hook to every provider.
    pub fn on_closed(&mut self) {
        for p in &mut self.providers {
            p.on_closed();
        }
    }

    /// The hot path: take a query, return decorated + scored +
    /// sorted [`DisplayItem`]s. Called on every keystroke.
    pub fn query(&self, query: &str) -> Vec<DisplayItem> {
        let trimmed = query.trim_start();

        // The substring to highlight in each result name. For a command-mode
        // query (`>run fire`) it's the part after the prefix token (so we
        // highlight `fire`, not `>run`); for a plain search it's the whole
        // query; for the bare command palette there's nothing to highlight.
        let needle: String = if trimmed == ">" {
            String::new()
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            rest.split_once(char::is_whitespace)
                .map(|(_, after)| after.trim().to_string())
                .unwrap_or_default()
        } else {
            trimmed.to_string()
        };

        // Bare ">" → command palette. Every provider's commands()
        // collected so the user can discover all prefixes at
        // once (`;` does the richer cheatsheet version of this).
        if trimmed == ">" {
            let raw: Vec<LauncherItem> = self.providers.iter().flat_map(|p| p.commands()).collect();
            return self.decorate(raw, &needle);
        }

        // Command mode: any provider that claims `handles_command`
        // for this query gets to own the results exclusively
        // (skipping regular-search providers).
        for p in &self.providers {
            if p.handles_command(trimmed) {
                let results = p.search(trimmed);
                // Command mode is a typed, provider-owned query (`>run`,
                // `>start`, `bt`, …); leave its ordering to the provider +
                // frequency, not recency.
                return self.decorate(self.apply_frecency_and_sort(results, false), &needle);
            }
        }

        // Regular search OR empty-query browse. Two paths:
        //
        // 1. **Specific category active** → call each in-category
        //    provider's `browse(filter)`, **bypassing**
        //    `handles_search`. This is what lets the prefix-only
        //    providers that own a tab (Scripts / Bluetooth /
        //    ProviderList / …) fill their category tabs with real
        //    content the moment the user picks that tab AND lets
        //    the user filter inside the tab by typing. Each such
        //    provider's `browse` impl synthesises the prefix
        //    internally. Without bypass, every prefix-only provider
        //    would render an empty tab — making the strip mostly
        //    useless beyond the few `handles_search=true` providers.
        //    Symbols / Emoji / Clipboard sit in the default "All"
        //    bucket (no tab), so they never reach this path.
        //
        // 2. **All-category** → standard search pipeline: only
        //    providers that opted into search via `handles_search`
        //    contribute; prefix-only providers stay silent unless
        //    their `handles_command` already caught the query above.
        let active = self.active_category.as_deref();
        let in_specific_category = matches!(active, Some(c) if c != "All");

        let mut explicit_provider_names = std::collections::HashSet::new();

        let mut results: Vec<LauncherItem> = if in_specific_category {
            let cat = active.unwrap();
            let mut category_results: Vec<LauncherItem> = self
                .providers
                .iter()
                .filter(|p| provider_matches_display_category(p.as_ref(), cat))
                .flat_map(|p| p.browse(trimmed))
                .collect();

            // Category-tab filter fallback. When the user is on a
            // specific category and typing, providers *should* filter
            // their `browse(filter)` output themselves. Some don't —
            // PlayerctlProvider returns its full transport/player list
            // regardless of the rest of the query, for instance.
            // Apply a uniform name-substring post-filter here so the
            // user's typing always narrows results visually, even
            // when the provider's own filter is a no-op.
            if !trimmed.is_empty() {
                let needle = trimmed.to_ascii_lowercase();
                for p in self.providers.iter().filter(|p| {
                    provider_matches_display_category(p.as_ref(), cat)
                        && p.bypasses_category_for_query(trimmed)
                }) {
                    explicit_provider_names.insert(p.name().to_string());
                }
                category_results.retain(|item| {
                    item.name.to_ascii_lowercase().contains(&needle)
                        || item.description.to_ascii_lowercase().contains(&needle)
                        || explicit_provider_names.contains(&item.provider_name)
                });

                // Explicit provider invocations should still work from
                // any tab. Example: the user can be sitting on "Actions"
                // and type `g pardus`; that is unambiguously a
                // Websearch query and must not be swallowed by the Actions
                // category filter.
                for p in self.providers.iter().filter(|p| {
                    !provider_matches_display_category(p.as_ref(), cat)
                        && p.bypasses_category_for_query(trimmed)
                }) {
                    explicit_provider_names.insert(p.name().to_string());
                    category_results.extend(p.search(query));
                }
            }

            category_results
        } else {
            self.providers
                .iter()
                .filter(|p| p.handles_search())
                .flat_map(|p| {
                    if p.bypasses_category_for_query(trimmed) {
                        explicit_provider_names.insert(p.name().to_string());
                    }
                    p.search(query)
                })
                .collect()
        };

        // Exact-search mode (Ctrl+E): post-filter to rows whose
        // *name* contains the trimmed query as a contiguous
        // case-insensitive substring. Stacks with the category
        // filter above — both are name-substring narrowing, so
        // running both is idempotent (the category filter already
        // covers the exact-search semantics within a tab). Explicit
        // provider invocations are exempt: `g pardus` intentionally
        // produces `Google: pardus`, whose display name should not
        // have to contain the provider prefix token.
        if self.exact_search && !trimmed.is_empty() {
            let needle = trimmed.to_ascii_lowercase();
            results.retain(|item| {
                item.name.to_ascii_lowercase().contains(&needle)
                    || explicit_provider_names.contains(&item.provider_name)
            });
        }

        // Browse-mode hidden filter. When the query is empty the
        // user is in "what do I have?" browse mode — items the
        // user has explicitly hidden should be suppressed so the
        // list reads as the curated subset they care about. When
        // they're actively typing they probably want to *find* a
        // hidden item (right-click → Unhide), so we leave hidden
        // rows in for non-empty queries.
        if trimmed.is_empty() {
            results.retain(|item| {
                item.usage_key
                    .as_deref()
                    .map(|k| !self.hidden.is_hidden(k))
                    .unwrap_or(true)
            });
        }

        // Recency leads in browse mode — a category tab (e.g. Actions) or the
        // empty-query list — so the things you just ran sit at the top. Typed
        // all-search keeps fuzzy-match order.
        let browse = in_specific_category || trimmed.is_empty();
        self.decorate(self.apply_frecency_and_sort(results, browse), &needle)
    }

    /// Shared scoring pipeline: add `usage_boost(count)` plus — in `browse`
    /// mode — a decaying `recency_boost`, plus the pin bonus (when applicable)
    /// to every item that carries a `usage_key`, then stable-sort descending by
    /// score. `browse` is the category-tab / empty-query view, where the most
    /// recently run rows should lead; typed all-search passes `false` so the
    /// fuzzy match score keeps deciding the order.
    fn apply_frecency_and_sort(
        &self,
        mut results: Vec<LauncherItem>,
        browse: bool,
    ) -> Vec<LauncherItem> {
        let now = crate::frecency::now_unix();
        for item in &mut results {
            if let Some(key) = &item.usage_key {
                let count = self.frecency.count(key);
                item.score += usage_boost(count);
                if browse && let Some(last) = self.frecency.last_used(key) {
                    item.score += recency_boost(now.saturating_sub(last));
                }
                if self.pins.is_pinned(key) {
                    item.score += PIN_BONUS;
                }
            }
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Wrap each [`LauncherItem`] in a [`DisplayItem`] carrying
    /// the runtime-stamped decorations (pin flag, hidden flag,
    /// quick-key digit). The hidden flag lets the UI flip the
    /// right-click context-menu label between "Hide" and
    /// "Unhide" without re-querying the store per row.
    fn decorate(&self, items: Vec<LauncherItem>, needle: &str) -> Vec<DisplayItem> {
        // One matcher reused across the whole result set (nucleo amortises its
        // buffers). Skipped entirely when there's nothing to highlight.
        let mut matcher = (!needle.is_empty()).then(crate::scoring::make_matcher);
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                let pinned = item
                    .usage_key
                    .as_deref()
                    .map(|k| self.pins.is_pinned(k))
                    .unwrap_or(false);
                let hidden = item
                    .usage_key
                    .as_deref()
                    .map(|k| self.hidden.is_hidden(k))
                    .unwrap_or(false);
                let quick_key = if idx < 9 {
                    (idx + 1).to_string()
                } else {
                    String::new()
                };
                let match_indices = match matcher.as_mut() {
                    Some(m) => crate::scoring::fuzzy_indices(m, needle, &item.name),
                    None => Vec::new(),
                };
                DisplayItem {
                    item,
                    pinned,
                    quick_key,
                    hidden,
                    match_indices,
                    zebra_odd: false,
                }
            })
            .collect()
    }

    /// True when the given usage_key is hidden. UI uses this to
    /// label the right-click context-menu entry.
    pub fn is_hidden(&self, key: &str) -> bool {
        self.hidden.is_hidden(key)
    }

    /// Toggle hide state for a usage_key. Returns the new state
    /// (`true` = now hidden). UI binds this to the "Hide" /
    /// "Unhide" entry in the launcher row's right-click context
    /// menu. Persists immediately.
    pub fn toggle_hidden(&mut self, key: &str) -> bool {
        self.hidden.toggle(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    struct StubProvider {
        name: String,
        items: Vec<(String, f64)>,
    }

    impl Provider for StubProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn search(&self, _query: &str) -> Vec<LauncherItem> {
            self.items
                .iter()
                .map(|(n, s)| LauncherItem {
                    id: format!("{}:{}", self.name, n),
                    name: n.clone(),
                    description: String::new(),
                    icon: String::new(),
                    icon_is_path: false,
                    score: *s,
                    provider_name: self.name.clone(),
                    usage_key: Some(format!("{}:{}", self.name, n)),
                    on_activate: Rc::new(|| {}),
                })
                .collect()
        }
    }

    struct CategorizedProvider {
        name: String,
        cat: String,
        items: Vec<(String, f64)>,
    }

    impl Provider for CategorizedProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn category(&self) -> &str {
            &self.cat
        }

        fn search(&self, _query: &str) -> Vec<LauncherItem> {
            self.items
                .iter()
                .map(|(n, s)| LauncherItem {
                    id: format!("{}:{}", self.name, n),
                    name: n.clone(),
                    description: String::new(),
                    icon: String::new(),
                    icon_is_path: false,
                    score: *s,
                    provider_name: self.name.clone(),
                    usage_key: Some(format!("{}:{}", self.name, n)),
                    on_activate: Rc::new(|| {}),
                })
                .collect()
        }
    }

    fn ephemeral_frecency() -> FrecencyStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_runtime_frec_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        FrecencyStore::load_from(path)
    }

    fn ephemeral_pins() -> PinStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_runtime_pins_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        PinStore::load_from(path)
    }

    #[test]
    fn empty_runtime_returns_nothing() {
        let rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        assert!(rt.query("anything").is_empty());
    }

    #[test]
    fn results_sort_by_score_descending() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![
                ("low".into(), 0.1),
                ("high".into(), 0.9),
                ("mid".into(), 0.5),
            ],
        }));
        let out = rt.query("q");
        assert_eq!(out[0].item.name, "high");
        assert_eq!(out[1].item.name, "mid");
        assert_eq!(out[2].item.name, "low");
    }

    #[test]
    fn usage_boost_can_break_ties() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![("alpha".into(), 0.5), ("beta".into(), 0.5)],
        }));
        for _ in 0..100 {
            rt.record_usage("stub:alpha");
        }
        let out = rt.query("q");
        assert_eq!(out[0].item.name, "alpha");
    }

    #[test]
    fn pin_overrides_frecency() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![("frec".into(), 0.5), ("pinned".into(), 0.0)],
        }));
        // Even with heavy frecency, the pinned item wins.
        for _ in 0..1000 {
            rt.record_usage("stub:frec");
        }
        rt.toggle_pin("stub:pinned");
        let out = rt.query("q");
        assert_eq!(out[0].item.name, "pinned");
        assert!(out[0].pinned);
    }

    /// Browse mode (empty query) keeps every pinned item above every
    /// unpinned one, and preserves the provider's own ordering inside
    /// each tier. Stable sort + uniform PIN_BONUS make this
    /// emergent — pinned items all carry the same score so their
    /// relative order matches what the provider returned (Apps
    /// returns alphabetical, so pinned apps come out alphabetical).
    /// Regression guard for #147: any future change to scoring
    /// must not let an unpinned item slip ahead of a pinned one
    /// in the empty-query browse list.
    #[test]
    fn pinned_apps_top_of_browse_alphabetical() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        // Score 0.0 mirrors AppsProvider's empty-query browse
        // return (apps_provider.rs:149-155).
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![
                ("alpha".into(), 0.0),
                ("bravo".into(), 0.0),
                ("charlie".into(), 0.0),
                ("delta".into(), 0.0),
            ],
        }));
        rt.toggle_pin("stub:charlie");
        rt.toggle_pin("stub:alpha");

        let out = rt.query("");

        assert_eq!(out[0].item.name, "alpha");
        assert!(out[0].pinned);
        assert_eq!(out[1].item.name, "charlie");
        assert!(out[1].pinned);
        assert_eq!(out[2].item.name, "bravo");
        assert!(!out[2].pinned);
        assert_eq!(out[3].item.name, "delta");
        assert!(!out[3].pinned);
    }

    #[test]
    fn quick_keys_assigned_to_first_nine_rows() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        let items: Vec<(String, f64)> = (0..12)
            .map(|i| (format!("i{i}"), 1.0 - i as f64 * 0.01))
            .collect();
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items,
        }));
        let out = rt.query("q");
        assert_eq!(out[0].quick_key, "1");
        assert_eq!(out[8].quick_key, "9");
        assert_eq!(out[9].quick_key, "");
        assert_eq!(out[11].quick_key, "");
    }

    #[test]
    fn category_cycle_wraps_with_all_prepended() {
        struct CatProvider {
            name: String,
            cat: String,
        }
        impl Provider for CatProvider {
            fn name(&self) -> &str {
                &self.name
            }
            fn category(&self) -> &str {
                &self.cat
            }
            fn search(&self, _q: &str) -> Vec<LauncherItem> {
                Vec::new()
            }
        }
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(CatProvider {
            name: "a".into(),
            cat: "Apps".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "b".into(),
            cat: "Compositor".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "c".into(),
            cat: "Run".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "d".into(),
            cat: "System".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "f".into(),
            cat: "Search".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "g".into(),
            cat: "Help".into(),
        }));
        rt.register(Box::new(CatProvider {
            name: "h".into(),
            cat: "Connect".into(),
        }));
        // Categories include implicit "All" prepended and collapse
        // Compositor / Run / System / Connect into one Actions tab.
        let cats = rt.categories();
        assert_eq!(
            cats.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            vec!["All", "Apps", "Actions", "Search", "Help"]
        );
        // Cycle forward: All → Apps → Actions → Search → Help → All
        assert_eq!(rt.cycle_category(1), "Apps");
        assert_eq!(rt.cycle_category(1), "Actions");
        assert_eq!(rt.cycle_category(1), "Search");
        assert_eq!(rt.cycle_category(1), "Help");
        assert_eq!(rt.cycle_category(1), "All");
        // Cycle back: All → Help
        assert_eq!(rt.cycle_category(-1), "Help");
    }

    #[test]
    fn actions_category_merges_action_like_provider_categories() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        for (name, cat) in [
            ("compositor", "Compositor"),
            ("run", "Run"),
            ("system", "System"),
            ("connect", "Connect"),
        ] {
            rt.register(Box::new(CategorizedProvider {
                name: name.into(),
                cat: cat.into(),
                items: vec![(format!("{name} item"), 1.0)],
            }));
        }
        rt.register(Box::new(CategorizedProvider {
            name: "search".into(),
            cat: "Search".into(),
            items: vec![("search item".into(), 1.0)],
        }));

        rt.select_category("Actions");
        let out = rt.query("");
        let names: Vec<_> = out.iter().map(|d| d.item.name.as_str()).collect();
        assert!(names.contains(&"compositor item"));
        assert!(names.contains(&"run item"));
        assert!(names.contains(&"system item"));
        assert!(names.contains(&"connect item"));
        assert!(!names.contains(&"search item"));
    }

    #[test]
    fn explicit_websearch_prefix_bypasses_active_category_filter() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(CategorizedProvider {
            name: "run".into(),
            cat: "Run".into(),
            items: vec![("Run: irrelevant".into(), 1.0)],
        }));
        rt.register(Box::new(crate::providers::WebsearchProvider::new()));

        rt.select_category("Actions");
        let out = rt.query("g pardus");

        assert!(out.iter().any(|d| d.item.name == "Google: pardus"));
        assert!(
            out.iter()
                .all(|d| d.item.provider_name == "Web search" || d.item.name.contains("g pardus"))
        );
    }

    #[test]
    fn explicit_websearch_prefix_survives_search_category_filter() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(crate::providers::WebsearchProvider::new()));

        rt.select_category("Search");
        let out = rt.query("g pardus");

        assert!(out.iter().any(|d| d.item.name == "Google: pardus"));
    }

    #[test]
    fn search_category_contains_search_providers_not_help_cheatsheet() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(crate::providers::WebsearchProvider::new()));
        rt.register(Box::new(crate::providers::ArchLinuxPkgsProvider::new()));
        rt.register(Box::new(crate::providers::ProviderListProvider::new(
            Rc::new(|_| {}),
        )));

        rt.select_category("Search");
        let search = rt.query("");
        assert!(search.iter().any(|d| d.item.name == "Google search"));
        assert!(
            search
                .iter()
                .any(|d| d.item.name == "Arch / AUR package search")
        );
        assert!(search.iter().all(|d| d.item.provider_name != "Providers"));

        rt.select_category("Help");
        let help = rt.query("");
        assert!(help.iter().any(|d| d.item.name.starts_with("g <query>")));
    }

    #[test]
    fn explicit_websearch_prefix_survives_exact_search_filter() {
        let mut rt = LauncherRuntime::with_stores(ephemeral_frecency(), ephemeral_pins());
        rt.register(Box::new(crate::providers::WebsearchProvider::new()));

        rt.toggle_exact_search();
        let out = rt.query("g pardus");

        assert!(out.iter().any(|d| d.item.name == "Google: pardus"));
    }
}
