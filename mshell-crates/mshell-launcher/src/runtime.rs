//! The runtime that glues providers, scoring, and frecency
//! together.
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
//!    returns nothing, etc.).
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

use crate::{frecency::FrecencyStore, item::LauncherItem, provider::Provider, scoring::usage_boost};

/// Owns the provider list + the frecency store + an entry-point
/// API that the UI can call on every keystroke.
pub struct LauncherRuntime {
    providers: Vec<Box<dyn Provider>>,
    frecency: FrecencyStore,
}

impl LauncherRuntime {
    /// Construct an empty runtime. Use [`LauncherRuntime::register`]
    /// to add providers; their declaration order is preserved when
    /// the runtime needs a stable tie-breaker.
    pub fn new(frecency: FrecencyStore) -> Self {
        Self {
            providers: Vec::new(),
            frecency,
        }
    }

    /// Add a provider. Order matters for command-mode dispatch
    /// (first matching provider wins) and for empty-query browse
    /// (results are concatenated in registration order).
    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    /// Read-only access to the frecency store. Tests use this; the
    /// UI calls [`LauncherRuntime::record_usage`] instead.
    pub fn frecency(&self) -> &FrecencyStore {
        &self.frecency
    }

    /// Bump the usage counter for an item. Call this in the UI's
    /// activation handler *after* the item is dispatched. Disk
    /// flush is opportunistic — call [`LauncherRuntime::flush`]
    /// when the launcher closes.
    pub fn record_usage(&mut self, key: &str) {
        self.frecency.bump(key);
    }

    /// Persist any pending frecency bumps to disk.
    pub fn flush(&mut self) {
        self.frecency.flush();
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

    /// The hot path: take a query, return scored + sorted items.
    /// Called on every keystroke from the search entry.
    pub fn query(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();

        // Command palette: bare ">" — list every command across
        // every provider in registration order.
        if trimmed == ">" {
            return self
                .providers
                .iter()
                .flat_map(|p| p.commands())
                .collect();
        }

        // Command mode: ">cmd ..." — find the first provider that
        // claims it and let it own the results. No scoring on top.
        if trimmed.starts_with('>') {
            for p in &self.providers {
                if p.handles_command(trimmed) {
                    return p.search(trimmed);
                }
            }
            // No provider claimed the prefix — fall through to
            // regular search so the user at least sees something.
        }

        // Regular search. Collect from every searching provider,
        // apply usage boost, sort by descending score.
        let mut results: Vec<LauncherItem> = self
            .providers
            .iter()
            .filter(|p| p.handles_search())
            .flat_map(|p| p.search(query))
            .collect();

        if !query.is_empty() {
            for item in &mut results {
                if let Some(key) = &item.usage_key {
                    let count = self.frecency.count(key);
                    item.score += usage_boost(count);
                }
            }
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        results
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

    fn ephemeral_store() -> FrecencyStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_runtime_{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        FrecencyStore::load_from(path)
    }

    #[test]
    fn empty_runtime_returns_nothing() {
        let rt = LauncherRuntime::new(ephemeral_store());
        assert!(rt.query("anything").is_empty());
    }

    #[test]
    fn results_sort_by_score_descending() {
        let mut rt = LauncherRuntime::new(ephemeral_store());
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![("low".into(), 0.1), ("high".into(), 0.9), ("mid".into(), 0.5)],
        }));
        let out = rt.query("q");
        assert_eq!(out[0].name, "high");
        assert_eq!(out[1].name, "mid");
        assert_eq!(out[2].name, "low");
    }

    #[test]
    fn usage_boost_can_break_ties() {
        let mut rt = LauncherRuntime::new(ephemeral_store());
        rt.register(Box::new(StubProvider {
            name: "stub".into(),
            items: vec![("alpha".into(), 0.5), ("beta".into(), 0.5)],
        }));
        // Bump alpha 100x — usage boost ~0.66 > 0 so alpha wins
        // even though raw scores tied.
        for _ in 0..100 {
            rt.record_usage("stub:alpha");
        }
        let out = rt.query("q");
        assert_eq!(out[0].name, "alpha");
    }
}
