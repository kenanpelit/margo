//! A single, displayable result row produced by a [`Provider`].
//!
//! Items are intentionally **opaque to the runtime**: they carry an
//! `Rc<dyn Fn()>` that the UI invokes on activation. This lets each
//! provider encode whatever side effect it wants (spawn a process,
//! emit an internal mshell message, navigate to a settings tab,
//! copy text to the clipboard…) without the runtime needing to know
//! about any of those mechanisms.
//!
//! [`Provider`]: crate::provider::Provider

use std::rc::Rc;

/// One row in the launcher's result list.
pub struct LauncherItem {
    /// Stable per-provider id used for keyed reconciliation in the
    /// `DynamicBox` widget. Two items with the same id replace each
    /// other; mixing providers means callers should namespace their
    /// ids (e.g. `apps:firefox.desktop`, `calc:result`).
    pub id: String,

    /// Primary display text. Bold in the row layout.
    pub name: String,

    /// Secondary text shown beneath / next to `name`. Empty string
    /// hides it.
    pub description: String,

    /// Icon name resolved against the configured icon theme — or a
    /// path to an image when `icon_is_path` is true.
    pub icon: String,

    /// When true, `icon` is treated as a filesystem path instead of
    /// a themed icon name. Useful for emoji bitmaps or app icons
    /// shipped outside the icon theme.
    pub icon_is_path: bool,

    /// Search score — higher is better. 0.0 for items that come
    /// from `commands()` / empty-query browse modes (those are
    /// shown in provider order, not score order).
    pub score: f64,

    /// Name of the [`Provider`] that produced this item. Used for
    /// stable per-provider grouping in the result list and for the
    /// source-badge column.
    ///
    /// [`Provider`]: crate::provider::Provider
    pub provider_name: String,

    /// When `Some`, the runtime bumps the corresponding entry in
    /// the frecency store on activation and applies a usage boost
    /// to this item's score. Providers that don't want to track
    /// usage (e.g. calculator results) leave this `None`.
    pub usage_key: Option<String>,

    /// Side effect to run when the user selects this row. Boxed
    /// rather than enum-typed so providers can encode arbitrarily
    /// complex actions (compositor calls, multi-step spawns) without
    /// the runtime needing to grow. Providers that want visible
    /// activation feedback (clipboard copy, twilight toggle…)
    /// invoke [`crate::notify::desktop`] from within this closure
    /// so the runtime stays oblivious to UI side-effects.
    pub on_activate: Rc<dyn Fn() + 'static>,
}

impl std::fmt::Debug for LauncherItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherItem")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("icon", &self.icon)
            .field("icon_is_path", &self.icon_is_path)
            .field("score", &self.score)
            .field("provider_name", &self.provider_name)
            .field("usage_key", &self.usage_key)
            .field("on_activate", &"<fn>")
            .finish()
    }
}
