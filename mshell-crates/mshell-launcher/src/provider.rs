//! The `Provider` trait every result source implements.
//!
//! Providers are stateful (Apps tracks the desktop-entry list,
//! Settings tracks the sidebar index) but the runtime treats them
//! uniformly: ask for results, append, sort, deduplicate.

use crate::item::LauncherItem;

/// A thread-safe callback a provider invokes when an off-thread data
/// refresh lands, asking the launcher UI to re-run the current query so the
/// freshly-loaded rows appear. Handed to providers by the runtime via
/// [`Provider::set_refresh_notifier`]; safe to call from a worker thread.
pub type RefreshNotifier = std::sync::Arc<dyn Fn() + Send + Sync>;

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

    /// Return true when `query` is an explicit, provider-owned
    /// invocation that should still work while the UI is filtered
    /// to a different category tab. Example: Websearch owns
    /// `g <query>`, so typing `g pardus` from the Run tab should
    /// produce the Google row instead of being swallowed by the
    /// active Run-category filter. Default: false.
    fn bypasses_category_for_query(&self, _query: &str) -> bool {
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

    /// Content of this provider for the category-tab path,
    /// optionally filtered by the user's current query.
    ///
    /// Called by the runtime when the user has Tab'd onto this
    /// provider's category and is browsing inside it. Defaults to
    /// `search(filter)` so providers that already serve raw
    /// queries (Apps, Calculator, Websearch, …) work unchanged.
    /// Prefix-only providers that DO own a category tab (Scripts,
    /// Bluetooth, ProviderList, …) override this to synthesise the
    /// prefix internally, so typing inside the tab returns the same
    /// rows the prefix would. Prefix-only providers with no tab of
    /// their own (Symbols `.`, Emoji `:`, Clipboard `>clip` — all in
    /// the default "All" bucket) never reach this path: they are
    /// served exclusively by the command branch in `query`.
    ///
    /// `filter` is the user's query, already trimmed by the
    /// runtime. Empty string means "show the full browse list".
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        self.search(filter)
    }

    /// Notification hook called when the launcher panel opens. Lets
    /// providers refresh stale data (Apps re-scans desktop entries,
    /// Settings rebuilds its index after a theme change). Default:
    /// no-op.
    fn on_opened(&mut self) {}

    /// Notification hook called when the launcher panel closes.
    /// Lets providers drop transient state (open file handles,
    /// in-flight async requests). Default: no-op.
    fn on_closed(&mut self) {}

    /// Hand the provider a notifier to call when one of its off-thread
    /// data refreshes completes, so the launcher can re-run the current
    /// query and show the new rows. Providers that read live external
    /// state through a blocking subprocess (Bluetooth / audio / media)
    /// refresh in the background to keep `search`/`browse` off the main
    /// thread and use this to repaint. Default: no-op (most providers
    /// serve from in-memory state synchronously).
    fn set_refresh_notifier(&mut self, _notifier: RefreshNotifier) {}

    /// Coarse category bucket the provider falls under. Drives the
    /// Tab/Shift+Tab provider-cycle and the visual category strip
    /// above the result list. Providers that share a category
    /// (e.g. Compositor + Run + System all collapse into `"Actions"`)
    /// flow into the same tab and the user cycles between buckets, not
    /// every individual provider. Default: `"All"` (the catch-all
    /// bucket the runtime uses for "no filter, mix everything") —
    /// which also means **no tab of one's own**, since `categories()`
    /// always prepends "All" and never duplicates it. Prefix-only
    /// providers reachable purely by their trigger (Symbols `.`,
    /// Emoji `:`, Clipboard `>clip`) deliberately stay on that
    /// default so they cost nothing until the prefix is typed.
    fn category(&self) -> &str {
        "All"
    }

    /// True when the runtime should let the user delete this item
    /// (Delete key in the UI). Providers with a frecency or history
    /// backing store return true here for any item they own.
    /// Default: false.
    fn can_delete(&self, _item: &LauncherItem) -> bool {
        false
    }

    /// Side-effect to perform when the user presses Delete on the
    /// item. Typically removes the matching frecency / history entry
    /// from the provider's backing cache. The runtime never calls
    /// this without first checking `can_delete`.
    fn delete_item(&mut self, _item: &LauncherItem) {}

    /// Optional alternative action — bound to Ctrl+Enter in the UI.
    /// Apps return a "launch in terminal" closure, Files return
    /// "open enclosing folder", Websearch returns "copy URL", etc.
    /// `None` (the default) means Ctrl+Enter falls back to the
    /// regular `on_activate` for that item.
    fn alt_action(&self, _item: &LauncherItem) -> Option<std::rc::Rc<dyn Fn() + 'static>> {
        None
    }

    /// Detail shown in the launcher's preview pane for `item` when it
    /// is the selection. Default: a plain text preview from the item's
    /// name + description. Providers override for richer content
    /// (calc result, clipboard payload, colour swatch).
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        Some(crate::LauncherPreview::text(
            item.name.clone(),
            item.description.clone(),
        ))
    }
}
