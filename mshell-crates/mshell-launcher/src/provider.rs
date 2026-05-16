//! The `Provider` trait every result source implements.
//!
//! Providers are stateful (Apps tracks the desktop-entry list,
//! Settings tracks the sidebar index) but the runtime treats them
//! uniformly: ask for results, append, sort, deduplicate.

use crate::item::LauncherItem;

/// A source of launcher results.
///
/// The runtime owns providers behind `Box<dyn Provider>` so they
/// must be object-safe. Lifetimes are kept simple: providers live
/// as long as the runtime does.
pub trait Provider {
    /// Stable, human-readable name shown next to results and used
    /// for the source-badge column. Should match the i18n key in
    /// `mshell-style` if/when we localise.
    fn name(&self) -> &str;

    /// When `false`, the provider is excluded from regular
    /// (non-command) search but may still contribute through
    /// `commands()`. Default: `true`.
    fn handles_search(&self) -> bool {
        true
    }

    /// Called when the query starts with `>`. Returns `true` if
    /// this provider wants to take over result generation — the
    /// runtime will skip every other provider and only call
    /// `search()` on this one.
    fn handles_command(&self, _query: &str) -> bool {
        false
    }

    /// Slash-style commands the provider advertises. The runtime
    /// concatenates these from every provider when the query is
    /// exactly `>`, so the user can see what's available.
    fn commands(&self) -> Vec<LauncherItem> {
        Vec::new()
    }

    /// Run a query and return matching items. The runtime calls
    /// this on every keystroke — implementations should be cheap
    /// (most do a simple string match over a pre-built list).
    ///
    /// Empty `query` means "browse mode" — providers may return a
    /// pinned/popular list (Apps) or nothing (Calculator).
    fn search(&self, query: &str) -> Vec<LauncherItem>;

    /// Notification hook called when the launcher panel opens. Lets
    /// providers refresh stale data (Apps re-scans desktop entries,
    /// Settings rebuilds its index after a theme change). Default:
    /// no-op.
    fn on_opened(&mut self) {}

    /// Notification hook called when the launcher panel closes.
    /// Lets providers drop transient state (open file handles,
    /// in-flight async requests). Default: no-op.
    fn on_closed(&mut self) {}
}
